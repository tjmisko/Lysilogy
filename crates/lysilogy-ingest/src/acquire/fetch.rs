//! Fetching candidate links: the transport-neutral [`PdfFetcher`] trait, content
//! classification, per-host politeness, and a scripted fake for tests.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use reqwest::Url;
use sha2::{Digest, Sha256};

/// Largest PDF accepted, in bytes (header and stream).
pub const MAX_PDF_BYTES: u64 = 100 * 1024 * 1024;

/// Largest landing page read, in bytes.
pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// How far into a file `%PDF-` may appear.
const PDF_MAGIC_WINDOW: usize = 1024;

/// One URL to fetch, with a directory the fetcher may write a temp file into.
#[derive(Clone, Copy, Debug)]
pub struct FetchRequest<'a> {
    pub url: &'a str,
    pub temp_dir: &'a Path,
}

/// What a URL turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchedResource {
    /// A PDF streamed to `temp_path`.
    Pdf {
        final_url: String,
        temp_path: PathBuf,
        sha256: String,
        bytes: i64,
    },
    /// A landing page, capped at [`MAX_HTML_BYTES`].
    Html { final_url: String, body: String },
}

/// Why a fetch failed. Transient failures are worth retrying on a later run.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("transient fetch failure: {0}")]
    Transient(String),
    #[error("permanent fetch failure: {0}")]
    Permanent(String),
}

impl FetchError {
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }

    /// The short reason without the kind prefix.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Transient(reason) | Self::Permanent(reason) => reason,
        }
    }
}

/// The future returned by a fetcher.
pub type FetchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<FetchedResource, FetchError>> + Send + 'a>>;

/// Fetches one URL as a PDF or a landing page. The live implementation lives in
/// the root crate (public-address guard, redirects, caps); tests use
/// [`ScriptedFetcher`].
pub trait PdfFetcher: Send + Sync {
    fn fetch<'a>(&'a self, request: FetchRequest<'a>) -> FetchFuture<'a>;
}

/// What a `Content-Type` header says the body is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentClass {
    /// Stream it and require `%PDF-`.
    PdfLike,
    /// Read it as a landing page.
    Html,
    /// Reject without reading the body.
    Other(String),
}

/// Classify a `Content-Type` header value: parameters stripped, case-insensitive.
/// A missing or empty header is treated as PDF-like (the magic check decides).
#[must_use]
pub fn classify_content_type(content_type: Option<&str>) -> ContentClass {
    let main = content_type
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match main.as_str() {
        ""
        | "application/pdf"
        | "application/x-pdf"
        | "application/octet-stream"
        | "binary/octet-stream"
        | "application/download"
        | "application/force-download" => ContentClass::PdfLike,
        "text/html" | "application/xhtml+xml" => ContentClass::Html,
        _ => ContentClass::Other(main),
    }
}

/// Whether `%PDF-` appears within the first kilobyte.
#[must_use]
pub fn has_pdf_magic(bytes: &[u8]) -> bool {
    let window = &bytes[..bytes.len().min(PDF_MAGIC_WINDOW)];
    window.windows(5).any(|chunk| chunk == b"%PDF-")
}

/// The failure an HTTP status implies, or `None` for success and redirects.
#[must_use]
pub fn classify_status(status: u16) -> Option<FetchError> {
    let reason = format!("http_{status}");
    match status {
        408 | 425 | 429 | 500..=599 => Some(FetchError::Transient(reason)),
        400..=499 => Some(FetchError::Permanent(reason)),
        _ => None,
    }
}

/// How long to wait before the next request to a host.
#[must_use]
pub fn throttle_delay(last: Option<Instant>, now: Instant, interval: Duration) -> Duration {
    let Some(last) = last else {
        return Duration::ZERO;
    };
    (last + interval).saturating_duration_since(now)
}

/// Minimum interval between requests to one host. Shared across runs in a process.
pub struct HostThrottle {
    default_interval: Duration,
    intervals: HashMap<String, Duration>,
    next_slot: Mutex<HashMap<String, Instant>>,
}

impl HostThrottle {
    #[must_use]
    pub fn new(default_interval: Duration, overrides: &[(&str, Duration)]) -> Self {
        Self {
            default_interval,
            intervals: overrides
                .iter()
                .map(|(host, interval)| ((*host).to_ascii_lowercase(), *interval))
                .collect(),
            next_slot: Mutex::new(HashMap::new()),
        }
    }

    /// A throttle that never waits.
    #[must_use]
    pub fn disabled() -> Self {
        Self::new(Duration::ZERO, &[])
    }

    /// The politeness defaults: 3 s for arXiv, 100 ms for API hosts, 1 s otherwise.
    #[must_use]
    pub fn polite() -> Self {
        let arxiv = Duration::from_secs(3);
        let api = Duration::from_millis(100);
        Self::new(
            Duration::from_secs(1),
            &[
                ("arxiv.org", arxiv),
                ("www.arxiv.org", arxiv),
                ("export.arxiv.org", arxiv),
                ("api.openalex.org", api),
                ("api.unpaywall.org", api),
            ],
        )
    }

    fn interval_for(&self, host: &str) -> Duration {
        self.intervals
            .get(host)
            .copied()
            .unwrap_or(self.default_interval)
    }

    /// Reserve the next slot for the URL's host and return how long to wait.
    fn reserve(&self, url: &str, now: Instant) -> Duration {
        let host = Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
            .unwrap_or_default();
        let interval = self.interval_for(&host);
        if interval.is_zero() {
            return Duration::ZERO;
        }
        let mut slots = self
            .next_slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let delay = throttle_delay(slots.get(&host).copied(), now, interval);
        slots.insert(host, now + delay);
        delay
    }

    /// Sleep until the URL's host may be contacted again, and claim that slot.
    pub async fn wait(&self, url: &str) {
        let delay = self.reserve(url, Instant::now());
        if delay.is_zero() {
            return;
        }
        tokio::time::sleep(delay).await;
    }
}

/// One scripted answer for a URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptedResponse {
    /// A PDF body (no magic is added or checked here).
    Pdf(Vec<u8>),
    /// A landing page body.
    Html(String),
    Error(FetchError),
}

/// A deterministic fetcher for tests.
///
/// Answers from a URL-to-response script,
/// writes PDF bytes into the request's temp directory so the blob store path is
/// exercised, and counts calls. It performs no network I/O.
#[derive(Default)]
pub struct ScriptedFetcher {
    script: Mutex<HashMap<String, VecDeque<ScriptedResponse>>>,
    calls: AtomicUsize,
    urls: Mutex<Vec<String>>,
    files: AtomicU64,
}

impl ScriptedFetcher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer every request for `url` with `response`.
    #[must_use]
    pub fn with(self, url: &str, response: ScriptedResponse) -> Self {
        self.with_sequence(url, vec![response])
    }

    /// Answer successive requests for `url` in order; the last answer repeats.
    #[must_use]
    pub fn with_sequence(self, url: &str, responses: Vec<ScriptedResponse>) -> Self {
        self.script
            .lock()
            .expect("script lock")
            .insert(url.to_owned(), responses.into());
        self
    }

    /// Total fetch calls so far.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Requested URLs, in call order.
    #[must_use]
    pub fn urls(&self) -> Vec<String> {
        self.urls.lock().expect("urls lock").clone()
    }

    fn next_response(&self, url: &str) -> ScriptedResponse {
        let not_found = || ScriptedResponse::Error(FetchError::Permanent("http_404".to_owned()));
        self.script
            .lock()
            .expect("script lock")
            .get_mut(url)
            .map_or_else(not_found, |queue| {
                if queue.len() > 1 {
                    return queue.pop_front().unwrap_or_else(not_found);
                }
                queue.front().cloned().unwrap_or_else(not_found)
            })
    }
}

impl PdfFetcher for ScriptedFetcher {
    fn fetch<'a>(&'a self, request: FetchRequest<'a>) -> FetchFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.urls
            .lock()
            .expect("urls lock")
            .push(request.url.to_owned());
        let response = self.next_response(request.url);
        let file_number = self.files.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match response {
                ScriptedResponse::Error(error) => Err(error),
                ScriptedResponse::Html(body) => Ok(FetchedResource::Html {
                    final_url: request.url.to_owned(),
                    body,
                }),
                ScriptedResponse::Pdf(bytes) => {
                    let temp_path = request.temp_dir.join(format!(
                        "scripted-{}-{file_number}.part",
                        std::process::id()
                    ));
                    tokio::fs::write(&temp_path, &bytes)
                        .await
                        .map_err(|error| FetchError::Transient(format!("io: {error}")))?;
                    Ok(FetchedResource::Pdf {
                        final_url: request.url.to_owned(),
                        temp_path,
                        sha256: format!("{:x}", Sha256::digest(&bytes)),
                        // A scripted body is far below i64::MAX.
                        #[allow(clippy::cast_possible_wrap)]
                        bytes: bytes.len() as i64,
                    })
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_classify_content_types_when_parameters_and_case_vary() {
        assert_eq!(
            classify_content_type(Some("Application/PDF; charset=binary")),
            ContentClass::PdfLike
        );
        assert_eq!(
            classify_content_type(Some(" application/octet-stream ")),
            ContentClass::PdfLike
        );
        assert_eq!(classify_content_type(None), ContentClass::PdfLike);
        assert_eq!(classify_content_type(Some("")), ContentClass::PdfLike);
        assert_eq!(
            classify_content_type(Some("text/HTML; charset=utf-8")),
            ContentClass::Html
        );
        assert_eq!(
            classify_content_type(Some("application/xhtml+xml")),
            ContentClass::Html
        );
        assert_eq!(
            classify_content_type(Some("Image/PNG")),
            ContentClass::Other("image/png".to_owned())
        );
    }

    #[test]
    fn should_detect_pdf_magic_within_first_kilobyte_only() {
        assert!(has_pdf_magic(b"%PDF-1.7\n"));
        let mut late = vec![b' '; 1019];
        late.extend_from_slice(b"%PDF-");
        assert!(has_pdf_magic(&late), "magic ending at byte 1024 counts");
        let mut too_late = vec![b' '; 1020];
        too_late.extend_from_slice(b"%PDF-");
        assert!(!has_pdf_magic(&too_late), "magic past 1024 bytes does not");
        assert!(!has_pdf_magic(b"<html>nope</html>"));
        assert!(!has_pdf_magic(b""));
        assert!(!has_pdf_magic(b"%PDF"));
    }

    #[test]
    fn should_compute_zero_delay_when_interval_elapsed() {
        let start = Instant::now();
        let interval = Duration::from_secs(3);
        assert_eq!(throttle_delay(None, start, interval), Duration::ZERO);
        assert_eq!(
            throttle_delay(Some(start), start + Duration::from_secs(5), interval),
            Duration::ZERO
        );
        assert_eq!(
            throttle_delay(Some(start), start + Duration::from_secs(1), interval),
            Duration::from_secs(2)
        );
        assert_eq!(
            throttle_delay(Some(start + Duration::from_secs(1)), start, interval),
            Duration::from_secs(4),
            "a slot reserved in the future pushes the next one past it"
        );
    }

    #[test]
    fn should_classify_429_and_5xx_as_transient_and_404_as_permanent() {
        for status in [408, 425, 429, 500, 503, 599] {
            assert!(
                classify_status(status).is_some_and(|error| error.is_transient()),
                "{status}"
            );
        }
        for status in [400, 401, 403, 404, 410, 451] {
            assert!(
                classify_status(status).is_some_and(|error| !error.is_transient()),
                "{status}"
            );
        }
        for status in [200, 204, 301, 302] {
            assert_eq!(classify_status(status), None, "{status}");
        }
    }

    #[test]
    fn should_space_requests_per_host_when_reserving_slots() {
        let throttle = HostThrottle::new(
            Duration::from_secs(1),
            &[("arxiv.org", Duration::from_secs(3))],
        );
        let now = Instant::now();
        assert_eq!(
            throttle.reserve("https://arxiv.org/pdf/1", now),
            Duration::ZERO
        );
        assert_eq!(
            throttle.reserve("https://arxiv.org/pdf/2", now),
            Duration::from_secs(3)
        );
        assert_eq!(
            throttle.reserve("https://ARXIV.org/pdf/3", now),
            Duration::from_secs(6),
            "host matching is case-insensitive and slots queue"
        );
        assert_eq!(
            throttle.reserve("https://example.org/a.pdf", now),
            Duration::ZERO,
            "other hosts have their own clock"
        );
        assert_eq!(
            throttle.reserve("https://example.org/b.pdf", now),
            Duration::from_secs(1)
        );
        assert_eq!(
            throttle.reserve("not a url", now),
            Duration::ZERO,
            "unparseable URLs share the default clock without panicking"
        );
    }

    #[tokio::test]
    async fn should_not_wait_when_throttle_is_disabled() {
        let throttle = HostThrottle::disabled();
        let started = Instant::now();
        for _ in 0..5 {
            throttle.wait("https://arxiv.org/pdf/1").await;
        }
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn should_use_polite_intervals_when_building_defaults() {
        let throttle = HostThrottle::polite();
        assert_eq!(throttle.interval_for("arxiv.org"), Duration::from_secs(3));
        assert_eq!(
            throttle.interval_for("export.arxiv.org"),
            Duration::from_secs(3)
        );
        assert_eq!(
            throttle.interval_for("api.openalex.org"),
            Duration::from_millis(100)
        );
        assert_eq!(throttle.interval_for("example.org"), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn should_write_pdf_bytes_and_hash_when_scripted_fetcher_serves_pdf() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fetcher = ScriptedFetcher::new()
            .with(
                "https://a.test/x.pdf",
                ScriptedResponse::Pdf(b"%PDF-1.4 hi".to_vec()),
            )
            .with(
                "https://a.test/page",
                ScriptedResponse::Html("<html></html>".to_owned()),
            );

        let pdf = fetcher
            .fetch(FetchRequest {
                url: "https://a.test/x.pdf",
                temp_dir: dir.path(),
            })
            .await
            .expect("pdf");
        let FetchedResource::Pdf {
            temp_path,
            sha256,
            bytes,
            ..
        } = pdf
        else {
            panic!("expected pdf");
        };
        assert_eq!(bytes, 11);
        assert_eq!(sha256.len(), 64);
        assert_eq!(std::fs::read(&temp_path).expect("file"), b"%PDF-1.4 hi");

        let html = fetcher
            .fetch(FetchRequest {
                url: "https://a.test/page",
                temp_dir: dir.path(),
            })
            .await
            .expect("html");
        assert!(matches!(html, FetchedResource::Html { .. }));

        let unknown = fetcher
            .fetch(FetchRequest {
                url: "https://a.test/none",
                temp_dir: dir.path(),
            })
            .await;
        assert_eq!(unknown, Err(FetchError::Permanent("http_404".to_owned())));
        assert_eq!(fetcher.calls(), 3);
    }

    #[tokio::test]
    async fn should_advance_through_sequence_when_responses_are_scripted_in_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fetcher = ScriptedFetcher::new().with_sequence(
            "https://a.test/x.pdf",
            vec![
                ScriptedResponse::Error(FetchError::Transient("http_503".to_owned())),
                ScriptedResponse::Pdf(b"%PDF-".to_vec()),
            ],
        );
        let request = FetchRequest {
            url: "https://a.test/x.pdf",
            temp_dir: dir.path(),
        };
        assert!(fetcher.fetch(request).await.is_err());
        assert!(fetcher.fetch(request).await.is_ok());
        assert!(fetcher.fetch(request).await.is_ok(), "last answer repeats");
    }
}
