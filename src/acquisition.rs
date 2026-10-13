//! Open-access acquisition for the root crate: the live fetcher, the
//! `AppState` entry points, and work-argument parsing.
//!
//! The resumable state machine lives in [`lysilogy_ingest::acquire`]; this
//! module supplies the public-address downloader, the catalog refresh after a
//! mapping, and the optional model sidecar.

pub mod picker;

use std::{fmt, path::Path, sync::Arc};

use lysilogy_core::{IdentifierScheme, WorkId, identifier};
use lysilogy_db::acquisition::CitedTarget;
use lysilogy_ingest::acquire::{
    AcquisitionContext, AcquisitionOptions, AcquisitionOutcome, AcquisitionSummary, FetchError,
    FetchFuture, FetchRequest, FetchedResource, LandingLinkPicker, PdfFetcher, run_acquisition,
    title_appears_in,
};
use lysilogy_ingest::openalex::contact_email_from_environment;
use reqwest::Url;

use crate::{
    AppState, Error, Result,
    analysis::LocalCliAnalyzer,
    domain::{AnalysisProvider, PaperId},
    remote::{DownloadError, PublicResource, download_public_resource},
};
use picker::SidecarLinkPicker;

/// The live [`PdfFetcher`]: public addresses only, bounded redirects and sizes.
#[derive(Clone, Copy, Debug, Default)]
pub struct PublicPdfFetcher;

impl PdfFetcher for PublicPdfFetcher {
    fn fetch<'a>(&'a self, request: FetchRequest<'a>) -> FetchFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(request.url)
                .map_err(|_| FetchError::Permanent("invalid_url".to_owned()))?;
            match download_public_resource(&url, request.temp_dir, true).await {
                Ok(PublicResource::Pdf {
                    final_url,
                    temp_path,
                    sha256,
                    bytes,
                }) => Ok(FetchedResource::Pdf {
                    final_url,
                    temp_path,
                    sha256,
                    // Downloads are capped at 100 MiB, far below i64::MAX.
                    #[allow(clippy::cast_possible_wrap)]
                    bytes: bytes as i64,
                }),
                Ok(PublicResource::Html { final_url, body }) => {
                    Ok(FetchedResource::Html { final_url, body })
                }
                Err(error) => Err(fetch_error(error)),
            }
        })
    }
}

/// Map a download failure to the runner's transient/permanent split.
pub(crate) fn fetch_error(error: DownloadError) -> FetchError {
    if error.transient {
        FetchError::Transient(error.reason)
    } else {
        FetchError::Permanent(error.reason)
    }
}

/// A shareable fetcher so `AppState` stays `Clone`.
#[derive(Clone)]
pub struct SharedFetcher(pub(crate) Arc<dyn PdfFetcher>);

impl fmt::Debug for SharedFetcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SharedFetcher")
    }
}

impl Default for SharedFetcher {
    fn default() -> Self {
        Self(Arc::new(PublicPdfFetcher))
    }
}

/// Per-call switches for [`AppState::acquire_work`].
#[derive(Clone, Copy, Debug, Default)]
pub struct AcquireOptions {
    pub retry_unavailable: bool,
    /// Let a local model pick the PDF link on landing pages.
    pub model_fallback: Option<AnalysisProvider>,
}

/// A run's summary plus what the catalog now knows.
#[derive(Debug)]
pub struct AcquireReport {
    pub summary: AcquisitionSummary,
    /// The library paper holding the copy, once the catalog contains it.
    pub paper_id: Option<PaperId>,
    pub title: Option<String>,
    pub year: Option<i32>,
}

impl AppState {
    /// Fetch an open-access copy of a Work and map it into the library.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Acquisition` for database or I/O
    /// failures. Unavailable and pending results are not errors.
    pub async fn acquire_work(
        &self,
        work: WorkId,
        options: AcquireOptions,
    ) -> Result<AcquireReport> {
        let graph = self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "acquisition needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })?;
        let picker = options
            .model_fallback
            .map(|provider| {
                SidecarLinkPicker::new(
                    provider,
                    LocalCliAnalyzer::default(),
                    self.store.root().join("acquisition").join("picks"),
                )
            })
            .transpose()?;
        let contact_email = contact_email_from_environment();
        let context = AcquisitionContext {
            works: &graph.works,
            documents: &graph.documents,
            records: &graph.source_records,
            acquisitions: &graph.acquisitions,
            openalex: self.literature.client(),
            fetcher: self.acquisition_fetcher.0.as_ref(),
            throttle: &self.acquisition_throttle,
            picker: picker
                .as_ref()
                .map(|picker| picker as &dyn LandingLinkPicker),
            contact_email: contact_email.as_deref(),
            blob_root: &self.store.root().join("blobs"),
            library_root: self.library_root.as_ref(),
        };
        let mut summary = run_acquisition(
            &context,
            work,
            AcquisitionOptions {
                retry_unavailable: options.retry_unavailable,
            },
        )
        .await
        .map_err(|error| Error::Acquisition(error.to_string()))?;

        let found = graph
            .works
            .get(summary.work_id)
            .await
            .map_err(|error| Error::Acquisition(error.to_string()))?;
        let title = found.as_ref().and_then(|work| work.display_title.clone());
        let year = found.as_ref().and_then(|work| work.display_year);

        let freshly_mapped = matches!(
            summary.outcome,
            AcquisitionOutcome::Mapped | AcquisitionOutcome::AlreadyLocal
        );
        let has_copy = freshly_mapped || summary.outcome == AcquisitionOutcome::AlreadyMapped;
        if freshly_mapped {
            self.refresh().await?;
        }
        let paper_id = if has_copy {
            self.catalog_paper_id(summary.relative_path.as_deref())
                .await
        } else {
            None
        };
        if freshly_mapped
            && let (Some(id), Some(title)) = (&paper_id, &title)
            && self.front_matter_misses_title(id, title).await
        {
            summary
                .warnings
                .push("title_not_in_front_matter".to_owned());
        }
        Ok(AcquireReport {
            summary,
            paper_id,
            title,
            year,
        })
    }

    async fn catalog_paper_id(&self, relative_path: Option<&str>) -> Option<PaperId> {
        let id = PaperId::from_relative_path(Path::new(relative_path?));
        self.catalog.read().await.get(&id).map(|_| id)
    }

    /// Whether the paper's first two pages lack the Work's title. A failed
    /// extraction is not a miss.
    async fn front_matter_misses_title(&self, id: &PaperId, title: &str) -> bool {
        match self.front_matter_text(id).await {
            Ok(text) => title_appears_in(title, &text) == Some(false),
            Err(error) => {
                tracing::warn!(%id, %error, "could not read front matter for the title check");
                false
            }
        }
    }

    /// The Works a paper cites that have no local copy, best candidates first.
    /// Entries with `skipped` are already mapped or recently unavailable and do
    /// not count against a limit.
    ///
    /// # Errors
    /// `PaperNotFound` for an unknown paper; `InvalidRequest` when the paper has
    /// no Work yet (run `lysilogy lit` first).
    pub async fn acquisition_targets_cited_by(
        &self,
        paper: &PaperId,
        retry_unavailable: bool,
    ) -> Result<Vec<CitedTarget>> {
        let graph = self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "acquisition needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })?;
        let relative_path = self
            .catalog
            .read()
            .await
            .get(paper)
            .map(|entry| entry.overview.relative_path.clone())
            .ok_or_else(|| Error::PaperNotFound(paper.to_string()))?;
        let acquisition_error = |error: lysilogy_db::DbError| Error::Acquisition(error.to_string());
        let workspace = graph
            .documents
            .ensure_default_workspace()
            .await
            .map_err(acquisition_error)?;
        let document = graph
            .documents
            .find_by_path(workspace, &relative_path)
            .await
            .map_err(acquisition_error)?;
        let Some(work) = document.and_then(|document| document.work_id) else {
            return Err(Error::InvalidRequest(format!(
                "{relative_path} has no Work yet; run lysilogy lit first"
            )));
        };
        let work = graph
            .works
            .resolve_redirect(work)
            .await
            .map_err(acquisition_error)?;
        graph
            .acquisitions
            .cited_targets(work, workspace, retry_unavailable)
            .await
            .map_err(acquisition_error)
    }

    /// Resolve a command-line Work reference: a `w…` ID, an `OpenAlex` `W…` ID,
    /// `doi:<doi>`, or `arxiv:<id>`.
    ///
    /// # Errors
    /// `InvalidRequest` for an unrecognized or unknown reference.
    pub async fn resolve_work_argument(&self, text: &str) -> Result<WorkId> {
        let graph = self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "acquisition needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })?;
        let text = text.trim();
        let unknown = || Error::InvalidRequest(format!("no Work matches {text}"));
        let database = |error: lysilogy_db::DbError| Error::Acquisition(error.to_string());
        let found = match parse_work_reference(text)? {
            WorkReference::Id(id) => graph.works.get(id).await.map_err(database)?,
            WorkReference::Identifier(scheme, value) => {
                let direct = graph
                    .works
                    .find_by_identifier(scheme, &value)
                    .await
                    .map_err(database)?;
                // Many arXiv preprints are known only by their DataCite DOI.
                match (direct, scheme) {
                    (None, IdentifierScheme::Arxiv) => graph
                        .works
                        .find_by_identifier(
                            IdentifierScheme::Doi,
                            &format!("10.48550/arxiv.{value}"),
                        )
                        .await
                        .map_err(database)?,
                    (direct, _) => direct,
                }
            }
        };
        let work = found.ok_or_else(unknown)?;
        graph
            .works
            .resolve_redirect(work.id)
            .await
            .map_err(database)
    }
}

/// A parsed command-line Work reference, before the database lookup.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WorkReference {
    Id(WorkId),
    Identifier(IdentifierScheme, String),
}

/// Parse a Work reference without touching the database.
pub(crate) fn parse_work_reference(text: &str) -> Result<WorkReference> {
    let text = text.trim();
    if let Ok(id) = text.parse::<WorkId>() {
        return Ok(WorkReference::Id(id));
    }
    if let Some(digits) = text.strip_prefix(['W', 'w'])
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Ok(WorkReference::Identifier(
            IdentifierScheme::Openalex,
            format!("W{digits}"),
        ));
    }
    let lowered = text.to_ascii_lowercase();
    for (prefix, scheme) in [
        ("doi:", IdentifierScheme::Doi),
        ("arxiv:", IdentifierScheme::Arxiv),
    ] {
        if lowered.starts_with(prefix) {
            let raw = &text[prefix.len()..];
            let value = identifier::normalize(scheme, raw).ok_or_else(|| {
                Error::InvalidRequest(format!(
                    "{raw:?} is not a valid {} identifier",
                    prefix.trim_end_matches(':')
                ))
            })?;
            return Ok(WorkReference::Identifier(scheme, value));
        }
    }
    Err(Error::InvalidRequest(format!(
        "{text:?} is not a Work reference; use a w\u{2026} ID, an OpenAlex W\u{2026} ID, doi:<doi>, or arxiv:<id>"
    )))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lysilogy_core::{IdentifierStatus, Visibility, WorkKind};
    use lysilogy_ingest::acquire::{
        HostThrottle, ScriptedFetcher, ScriptedResponse, classify_status,
    };
    use lysilogy_ingest::openalex::{JsonTransport, ProviderError, ProviderFuture};
    use serde_json::Value;

    use super::*;
    use crate::graph::GraphState;

    fn download_error(transient: bool, reason: &str) -> DownloadError {
        DownloadError {
            error: Error::RemoteImport("test".to_owned()),
            reason: reason.to_owned(),
            transient,
        }
    }

    #[test]
    fn should_classify_timeout_as_transient_when_mapping_fetch_errors() {
        let timeout = fetch_error(download_error(true, "request_failed"));
        assert_eq!(timeout, FetchError::Transient("request_failed".to_owned()));
        let body = fetch_error(download_error(true, "body_failed"));
        assert!(body.is_transient());
        let refused = fetch_error(download_error(false, "non_public_address"));
        assert_eq!(
            refused,
            FetchError::Permanent("non_public_address".to_owned())
        );
        for status in [408_u16, 425, 429, 500, 503] {
            assert!(classify_status(status).is_some_and(|error| error.is_transient()));
        }
        for status in [401_u16, 403, 404, 410] {
            assert!(classify_status(status).is_some_and(|error| !error.is_transient()));
        }
    }

    #[tokio::test]
    async fn should_reject_private_address_when_candidate_points_to_loopback() {
        let directory = tempfile::tempdir().expect("temporary directory");
        for url in [
            "http://127.0.0.1/paper.pdf",
            "https://localhost/paper.pdf",
            "http://10.0.0.5/paper.pdf",
            "http://169.254.169.254/latest/meta-data",
            "ftp://93.184.216.34/paper.pdf",
        ] {
            let error = PublicPdfFetcher
                .fetch(FetchRequest {
                    url,
                    temp_dir: directory.path(),
                })
                .await
                .expect_err("guard must refuse before connecting");
            assert_eq!(
                error,
                FetchError::Permanent("non_public_address".to_owned()),
                "{url}"
            );
        }
        let malformed = PublicPdfFetcher
            .fetch(FetchRequest {
                url: "not a url",
                temp_dir: directory.path(),
            })
            .await
            .expect_err("malformed URL");
        assert_eq!(malformed, FetchError::Permanent("invalid_url".to_owned()));
        let leftovers = std::fs::read_dir(directory.path()).expect("read").count();
        assert_eq!(leftovers, 0, "a refused fetch must not leave temp files");
    }

    #[test]
    fn should_parse_work_reference_when_given_w_id_openalex_doi_or_arxiv() {
        let id = WorkId::from_uuid(uuid::Uuid::new_v4());
        assert_eq!(
            parse_work_reference(&id.to_string()).expect("w id"),
            WorkReference::Id(id)
        );
        assert_eq!(
            parse_work_reference(" W2410842990 ").expect("openalex"),
            WorkReference::Identifier(IdentifierScheme::Openalex, "W2410842990".to_owned())
        );
        assert_eq!(
            parse_work_reference("w2410842990").expect("lowercase openalex"),
            WorkReference::Identifier(IdentifierScheme::Openalex, "W2410842990".to_owned())
        );
        assert_eq!(
            parse_work_reference("doi:10.48550/ARXIV.1606.03137").expect("doi"),
            WorkReference::Identifier(
                IdentifierScheme::Doi,
                "10.48550/arxiv.1606.03137".to_owned()
            )
        );
        assert_eq!(
            parse_work_reference("arxiv:1606.03137").expect("arxiv"),
            WorkReference::Identifier(IdentifierScheme::Arxiv, "1606.03137".to_owned())
        );
        for malformed in [
            "",
            "W",
            "Wabc",
            "doi:",
            "doi:not-a-doi",
            "arxiv:../etc",
            "hello",
            "x1",
        ] {
            let error = parse_work_reference(malformed).expect_err(malformed);
            assert!(
                matches!(error, Error::InvalidRequest(_)),
                "{malformed}: {error}"
            );
        }
    }

    fn marker() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    async fn test_graph() -> Option<GraphState> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        Some(GraphState::connect(&url).await.expect("connect"))
    }

    async fn state_with(graph: GraphState) -> (AppState, tempfile::TempDir, tempfile::TempDir) {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state")
            .with_graph(graph)
            .with_acquisition_throttle(HostThrottle::disabled());
        (state, library, data)
    }

    #[tokio::test]
    async fn should_resolve_work_argument_when_given_w_id_openalex_doi_or_arxiv() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let (state, _library, _data) = state_with(graph.clone()).await;
        let work = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;
        let token = marker();
        let openalex = format!(
            "W9{}",
            token
                .bytes()
                .filter(u8::is_ascii_digit)
                .take(9)
                .map(char::from)
                .collect::<String>()
        );
        let doi = format!("10.1000/arg-{token}");
        let arxiv = format!(
            "2{:03}.{:05}",
            token.len() % 1000,
            uuid::Uuid::new_v4().as_u128() % 100_000
        );
        for (scheme, value) in [
            (IdentifierScheme::Openalex, openalex.clone()),
            (IdentifierScheme::Doi, doi.clone()),
            (IdentifierScheme::Arxiv, arxiv.clone()),
        ] {
            graph
                .works
                .add_identifier(work, scheme, &value, None, IdentifierStatus::Active)
                .await
                .expect("identifier");
        }
        for text in [
            work.to_string(),
            openalex.clone(),
            format!("doi:{doi}"),
            format!("arxiv:{arxiv}"),
        ] {
            assert_eq!(
                state.resolve_work_argument(&text).await.expect(&text),
                work,
                "{text}"
            );
        }
        let datacite = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;
        let datacite_arxiv = format!(
            "2{:03}.{:05}",
            token.len() % 7,
            uuid::Uuid::new_v4().as_u128() % 100_000
        );
        graph
            .works
            .add_identifier(
                datacite,
                IdentifierScheme::Doi,
                &format!("10.48550/arxiv.{datacite_arxiv}"),
                None,
                IdentifierStatus::Active,
            )
            .await
            .expect("datacite doi");
        assert_eq!(
            state
                .resolve_work_argument(&format!("arxiv:{datacite_arxiv}"))
                .await
                .expect("arxiv via datacite doi"),
            datacite
        );
        let unknown = state
            .resolve_work_argument("doi:10.1000/never-seen-this")
            .await
            .expect_err("unknown doi");
        assert!(matches!(unknown, Error::InvalidRequest(_)), "{unknown}");
        let unknown_id = state
            .resolve_work_argument(&WorkId::from_uuid(uuid::Uuid::new_v4()).to_string())
            .await
            .expect_err("unknown id");
        assert!(
            matches!(unknown_id, Error::InvalidRequest(_)),
            "{unknown_id}"
        );
    }

    /// Answers every provider request with 404, so only identifier-derived
    /// candidates remain.
    struct NotFoundTransport;

    impl JsonTransport for NotFoundTransport {
        fn get_json<'a>(
            &'a self,
            _url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async { Err(ProviderError::Status(404)) })
        }
    }

    #[tokio::test]
    async fn should_refresh_catalog_and_return_paper_id_when_work_is_mapped() {
        let Some(graph) = test_graph().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let token = uuid::Uuid::new_v4().as_u128();
        let arxiv = format!("2{:03}.{:05}", token % 1000, (token / 1000) % 100_000);
        let work = graph
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;
        graph
            .works
            .add_identifier(
                work,
                IdentifierScheme::Arxiv,
                &arxiv,
                None,
                IdentifierStatus::Active,
            )
            .await
            .expect("identifier");
        graph
            .works
            .set_display_title(work, "A Scripted Study of Fetching")
            .await
            .expect("title");
        graph
            .works
            .set_display_year(work, 2021)
            .await
            .expect("year");

        let pdf_url = format!("https://arxiv.org/pdf/{arxiv}");
        let mut body = b"%PDF-1.4\n".to_vec();
        body.extend_from_slice(marker().as_bytes());
        let fetcher = Arc::new(ScriptedFetcher::new().with(&pdf_url, ScriptedResponse::Pdf(body)));
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state")
            .with_graph(graph.clone())
            .with_literature_client(crate::literature::LiteratureClient::new(NotFoundTransport))
            .with_acquisition_throttle(HostThrottle::disabled())
            .with_acquisition_fetcher(fetcher.clone());
        assert!(state.library().await.papers.is_empty());

        let report = state
            .acquire_work(work, AcquireOptions::default())
            .await
            .expect("acquire");

        assert_eq!(report.summary.outcome, AcquisitionOutcome::Mapped);
        assert_eq!(
            report.title.as_deref(),
            Some("A Scripted Study of Fetching")
        );
        assert_eq!(report.year, Some(2021));
        let relative_path = report.summary.relative_path.clone().expect("relative path");
        assert!(relative_path.starts_with("Fetched/"), "{relative_path}");
        assert!(library.path().join(&relative_path).is_file());
        let paper_id = report.paper_id.expect("paper id after refresh");
        assert_eq!(
            paper_id,
            PaperId::from_relative_path(std::path::Path::new(&relative_path))
        );
        assert!(
            state.paper(&paper_id).await.is_ok(),
            "paper is in the catalog"
        );
        let blob_files = walk_files(&data.path().join("blobs").join("sha256"));
        assert_eq!(blob_files.len(), 1, "{blob_files:?}");

        let calls = fetcher.calls();
        let rerun = state
            .acquire_work(work, AcquireOptions::default())
            .await
            .expect("rerun");
        assert_eq!(rerun.summary.outcome, AcquisitionOutcome::AlreadyMapped);
        assert_eq!(rerun.paper_id, Some(paper_id));
        assert_eq!(fetcher.calls(), calls, "a rerun must not fetch again");
    }

    fn walk_files(root: &std::path::Path) -> Vec<PathBuf> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(walkdir::DirEntry::into_path)
            .collect()
    }
}
