use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use lysilogy_ingest::acquire::{
    ContentClass, MAX_HTML_BYTES, MAX_PDF_BYTES, classify_content_type, classify_status,
    has_pdf_magic,
};
use lysilogy_ingest::openalex::contact_email_from_environment;
use reqwest::{
    Client, Url,
    header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, LOCATION},
    redirect::Policy,
};
use sha2::{Digest, Sha256};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, net::lookup_host};

use crate::{Error, Result};

pub const MAX_PUBLIC_REDIRECTS: usize = 5;

/// How long a connection may take to establish.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a whole PDF request may take.
const PDF_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// How long reading a landing page may take once headers arrive.
const HTML_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// How far into a PDF `%PDF-` may appear.
const PDF_PREFIX_BYTES: usize = 1_024;
const PDF_LIMIT_MIB: u64 = MAX_PDF_BYTES / (1024 * 1024);

pub async fn public_http_client(url: &Url, request_timeout: Duration) -> Result<Client> {
    public_client(
        url,
        request_timeout,
        request_timeout,
        concat!("lysilogy/", env!("CARGO_PKG_VERSION")),
    )
    .await
}

async fn public_client(
    url: &Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    user_agent: &str,
) -> Result<Client> {
    let (host, address) = resolve_public_target(url).await.ok_or_else(|| {
        Error::InvalidRequest(
            "remote URLs must use standard HTTP(S) and resolve only to public addresses".to_owned(),
        )
    })?;
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
        .user_agent(user_agent);
    if host.parse::<IpAddr>().is_err() {
        builder = builder.resolve(&host, address);
    }
    builder
        .build()
        .map_err(|error| Error::RemoteImport(format!("could not create HTTP client: {error}")))
}

/// `Lysilogy/<version>`, plus ` (mailto:<email>)` when a contact email is given
/// and safe to put in a header.
#[must_use]
pub fn polite_user_agent(contact_email: Option<&str>) -> String {
    let base = concat!("Lysilogy/", env!("CARGO_PKG_VERSION"));
    match contact_email.map(str::trim) {
        Some(email)
            if !email.is_empty()
                && email.chars().all(|character| {
                    character.is_ascii_graphic() && character != '(' && character != ')'
                }) =>
        {
            format!("{base} (mailto:{email})")
        }
        _ => base.to_owned(),
    }
}

/// What a public URL turned out to be.
#[derive(Debug)]
pub enum PublicResource {
    /// A PDF streamed to `temp_path` inside the caller's temp directory.
    Pdf {
        final_url: String,
        temp_path: PathBuf,
        sha256: String,
        bytes: u64,
    },
    /// A landing page, capped at [`MAX_HTML_BYTES`].
    Html { final_url: String, body: String },
}

/// A failed download: the error to report, a short stable reason, and whether
/// trying again later could succeed.
#[derive(Debug)]
pub struct DownloadError {
    pub error: Error,
    pub reason: String,
    pub transient: bool,
}

impl DownloadError {
    fn permanent(error: Error, reason: impl Into<String>) -> Self {
        Self {
            error,
            reason: reason.into(),
            transient: false,
        }
    }

    fn transient(error: Error, reason: impl Into<String>) -> Self {
        Self {
            error,
            reason: reason.into(),
            transient: true,
        }
    }

    #[must_use]
    pub fn into_error(self) -> Error {
        self.error
    }
}

fn not_a_pdf() -> DownloadError {
    DownloadError::permanent(
        Error::InvalidRequest("the downloaded resource is not a PDF".to_owned()),
        "not_pdf",
    )
}

fn too_large() -> DownloadError {
    DownloadError::permanent(
        Error::InvalidRequest(format!("remote PDFs are limited to {PDF_LIMIT_MIB} MiB")),
        "too_large",
    )
}

fn temp_io(path: &Path, error: std::io::Error) -> DownloadError {
    DownloadError::permanent(Error::io(path, error), "io")
}

/// Download a public HTTP(S) resource: a PDF into `temp_dir`, or (with
/// `accept_html`) a landing page body.
///
/// Every hop, up to [`MAX_PUBLIC_REDIRECTS`] redirects, is re-checked against the
/// public-address guard. The `Content-Type` is classified before the body is
/// read, the PDF stream is capped at 100 MiB and hashed as it arrives, and
/// `%PDF-` must appear in the first kilobyte. The temp file is created with a
/// random name and removed on any error.
///
/// # Errors
/// A [`DownloadError`] with the error to report and whether a retry could help.
pub async fn download_public_resource(
    url: &Url,
    temp_dir: &Path,
    accept_html: bool,
) -> std::result::Result<PublicResource, DownloadError> {
    let user_agent = polite_user_agent(contact_email_from_environment().as_deref());
    let accept = if accept_html {
        "application/pdf,application/octet-stream;q=0.8,text/html;q=0.5"
    } else {
        "application/pdf,application/octet-stream;q=0.8"
    };
    let mut current = url.clone();
    for redirect_count in 0..=MAX_PUBLIC_REDIRECTS {
        let client = public_client(&current, CONNECT_TIMEOUT, PDF_REQUEST_TIMEOUT, &user_agent)
            .await
            .map_err(|error| {
                let reason = match &error {
                    Error::InvalidRequest(_) => "non_public_address",
                    _ => "client",
                };
                DownloadError::permanent(error, reason)
            })?;
        let response = client
            .get(current.clone())
            .header(ACCEPT, accept)
            .send()
            .await
            .map_err(|error| {
                DownloadError::transient(
                    Error::RemoteImport(format!("request failed: {error}")),
                    "request_failed",
                )
            })?;

        if response.status().is_redirection() {
            if redirect_count == MAX_PUBLIC_REDIRECTS {
                return Err(DownloadError::permanent(
                    Error::RemoteImport(format!(
                        "remote server exceeded {MAX_PUBLIC_REDIRECTS} redirects"
                    )),
                    "too_many_redirects",
                ));
            }
            let bad_redirect = |message: String| {
                DownloadError::permanent(Error::RemoteImport(message), "bad_redirect")
            };
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| bad_redirect("redirect omitted its destination".to_owned()))?
                .to_str()
                .map_err(|_| bad_redirect("redirect destination was not text".to_owned()))?;
            current = current
                .join(location)
                .map_err(|error| bad_redirect(format!("invalid redirect: {error}")))?;
            current.set_fragment(None);
            continue;
        }
        let status = response.status();
        if !status.is_success() {
            let error = Error::RemoteImport(format!("remote server returned HTTP {status}"));
            return Err(match classify_status(status.as_u16()) {
                Some(failure) if failure.is_transient() => {
                    DownloadError::transient(error, failure.reason())
                }
                _ => DownloadError::permanent(error, format!("http_{}", status.as_u16())),
            });
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let class = classify_content_type(content_type.as_deref());
        if let Some(rejection) = reject_content_class(&class, accept_html) {
            return Err(rejection);
        }
        return match class {
            ContentClass::Html => read_html(response, current).await,
            _ => stream_pdf(response, current, temp_dir).await,
        };
    }
    Err(DownloadError::permanent(
        Error::RemoteImport("remote redirect handling ended unexpectedly".to_owned()),
        "bad_redirect",
    ))
}

/// The rejection for a content class the caller does not accept, before any
/// body is read.
pub fn reject_content_class(class: &ContentClass, accept_html: bool) -> Option<DownloadError> {
    match class {
        ContentClass::PdfLike => None,
        ContentClass::Html if accept_html => None,
        ContentClass::Html => Some(not_a_pdf()),
        ContentClass::Other(main) => {
            let mut error = not_a_pdf();
            error.reason = format!("content_type:{main}");
            Some(error)
        }
    }
}

async fn read_html(
    mut response: reqwest::Response,
    final_url: Url,
) -> std::result::Result<PublicResource, DownloadError> {
    let read = async {
        let mut body: Vec<u8> = Vec::new();
        while body.len() < MAX_HTML_BYTES {
            let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| download_failed(&error))?
            else {
                break;
            };
            let remaining = MAX_HTML_BYTES - body.len();
            body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        }
        Ok(body)
    };
    let body = tokio::time::timeout(HTML_READ_TIMEOUT, read)
        .await
        .map_err(|_| {
            DownloadError::transient(
                Error::RemoteImport("download timed out".to_owned()),
                "timeout",
            )
        })??;
    Ok(PublicResource::Html {
        final_url: final_url.to_string(),
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn download_failed(error: &reqwest::Error) -> DownloadError {
    DownloadError::transient(
        Error::RemoteImport(format!("download failed: {error}")),
        "body_failed",
    )
}

async fn stream_pdf(
    response: reqwest::Response,
    final_url: Url,
    temp_dir: &Path,
) -> std::result::Result<PublicResource, DownloadError> {
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_PDF_BYTES)
    {
        return Err(too_large());
    }
    let path = temp_dir.join(format!(".lysilogy-download-{}.part", uuid::Uuid::new_v4()));
    let output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .await
        .map_err(|error| temp_io(&path, error))?;
    match write_pdf_stream(response, output, &path).await {
        Ok((sha256, bytes)) => Ok(PublicResource::Pdf {
            final_url: final_url.to_string(),
            temp_path: path,
            sha256,
            bytes,
        }),
        Err(error) => {
            if let Err(remove) = tokio::fs::remove_file(&path).await {
                tracing::warn!(path = %path.display(), %remove, "could not remove partial download");
            }
            Err(error)
        }
    }
}

async fn write_pdf_stream(
    mut response: reqwest::Response,
    mut output: tokio::fs::File,
    path: &Path,
) -> std::result::Result<(String, u64), DownloadError> {
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut prefix = Vec::with_capacity(PDF_PREFIX_BYTES);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| download_failed(&error))?
    {
        bytes = bytes.saturating_add(chunk.len() as u64);
        if bytes > MAX_PDF_BYTES {
            return Err(too_large());
        }
        if prefix.len() < PDF_PREFIX_BYTES {
            let remaining = PDF_PREFIX_BYTES - prefix.len();
            prefix.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        }
        hasher.update(&chunk);
        output
            .write_all(&chunk)
            .await
            .map_err(|error| temp_io(path, error))?;
    }
    output.flush().await.map_err(|error| temp_io(path, error))?;
    output
        .sync_all()
        .await
        .map_err(|error| temp_io(path, error))?;
    if !has_pdf_magic(&prefix) {
        return Err(not_a_pdf());
    }
    Ok((format!("{:x}", hasher.finalize()), bytes))
}

async fn resolve_public_target(url: &Url) -> Option<(String, SocketAddr)> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let expected_port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    if url.port().is_some_and(|port| port != expected_port) {
        return None;
    }
    let raw_host = url.host_str()?;
    if raw_host.ends_with('.') {
        return None;
    }
    let host = raw_host.to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") {
        return None;
    }

    if let Ok(address) = host.parse::<IpAddr>() {
        return is_public_address(address)
            .then_some((host, SocketAddr::new(address, expected_port)));
    }

    let addresses = lookup_host((host.as_str(), expected_port))
        .await
        .ok()?
        .collect::<Vec<_>>();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_public_address(address.ip()))
    {
        return None;
    }
    Some((host, *addresses.first()?))
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    !(first == 0
        || first == 10
        || first == 127
        || first >= 224
        || (first == 100 && (64..=127).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 192 && second == 0 && third == 2)
        || (first == 192 && second == 88 && third == 99)
        || (first == 192 && second == 168)
        || (first == 198 && (second == 18 || second == 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113))
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(embedded) = address.to_ipv4() {
        return is_public_ipv4(embedded);
    }
    let segments = address.segments();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        || (segments[0] == 0x0064 && segments[1] == 0xff9b)
        || (segments[0] == 0x2001 && segments[1] < 0x0200)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || segments[0] == 0x2002)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_reserved_and_metadata_networks() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            let parsed = address
                .parse::<IpAddr>()
                .expect("test address should parse");
            assert!(!is_public_address(parsed), "{address} must not be fetched");
        }
        assert!(is_public_address(IpAddr::V4(Ipv4Addr::new(
            93, 184, 216, 34
        ))));
        assert!(is_public_address(
            "2606:2800:220:1:248:1893:25c8:1946"
                .parse()
                .expect("test address should parse")
        ));
    }

    #[tokio::test]
    async fn rejects_unsafe_url_shapes_before_a_request() {
        for value in [
            "ftp://93.184.216.34/source",
            "https://reader@93.184.216.34/source",
            "https://93.184.216.34:8443/source",
            "https://127.0.0.1/source",
            "https://example.com./source",
        ] {
            let url = Url::parse(value).expect("test URL should parse");
            assert!(
                resolve_public_target(&url).await.is_none(),
                "{value} must be rejected"
            );
        }

        let public_literal =
            Url::parse("https://93.184.216.34/source").expect("public test URL should parse");
        assert!(resolve_public_target(&public_literal).await.is_some());
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;

    #[test]
    fn should_append_contact_email_to_user_agent_when_configured() {
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(polite_user_agent(None), format!("Lysilogy/{version}"));
        assert_eq!(
            polite_user_agent(Some("reader@example.org")),
            format!("Lysilogy/{version} (mailto:reader@example.org)")
        );
    }

    #[test]
    fn should_omit_contact_email_from_user_agent_when_it_is_unsafe_or_blank() {
        let plain = format!("Lysilogy/{}", env!("CARGO_PKG_VERSION"));
        for email in [
            "",
            "   ",
            "a b@example.org",
            "x@example.org)\r\nX: y",
            "é@example.org",
        ] {
            assert_eq!(polite_user_agent(Some(email)), plain, "{email:?}");
        }
    }

    #[tokio::test]
    async fn should_refuse_private_target_without_creating_temp_file_when_downloading() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let url = Url::parse("http://127.0.0.1/paper.pdf").expect("url");
        let error = download_public_resource(&url, directory.path(), true)
            .await
            .expect_err("loopback is refused");
        assert!(!error.transient);
        assert_eq!(error.reason, "non_public_address");
        assert_eq!(
            std::fs::read_dir(directory.path()).expect("read").count(),
            0
        );
    }

    #[test]
    fn should_name_content_type_when_rejecting_other_classes() {
        let class = classify_content_type(Some("image/png"));
        let rejection = reject_content_class(&class, true).expect("rejected");
        assert_eq!(rejection.reason, "content_type:image/png");
        assert!(!rejection.transient);
    }
}
