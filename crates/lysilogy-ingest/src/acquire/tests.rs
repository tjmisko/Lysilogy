//! Runner tests. All use fakes: a scripted fetcher, a recording transport, and
//! temporary blob and library roots. Nothing touches the network.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lysilogy_core::{IdentifierScheme, IdentifierStatus, Visibility, WorkKind};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::*;
use crate::openalex::fake::RecordingTransport;

const EMAIL: &str = "tester@example.test";

fn sha_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// An `OpenAlex` open-access response.
fn oa_response(
    best: Option<&str>,
    oa_url: Option<&str>,
    locations: &[String],
    landing: Option<&str>,
) -> Value {
    let locations: Vec<Value> = locations
        .iter()
        .map(|url| json!({"is_oa": true, "pdf_url": url, "landing_page_url": null}))
        .collect();
    json!({
        "best_oa_location": {"is_oa": true, "pdf_url": best, "landing_page_url": landing},
        "open_access": {"oa_url": oa_url},
        "locations": locations,
    })
}

fn nothing_offered() -> Value {
    oa_response(None, None, &[], None)
}

struct Harness {
    pool: PgPool,
    works: WorkRepository,
    documents: DocumentRepository,
    records: SourceRecordRepository,
    acquisitions: AcquisitionRepository,
    transport: RecordingTransport,
    client: OpenAlexClient<RecordingTransport>,
    blob_dir: tempfile::TempDir,
    library_dir: tempfile::TempDir,
    marker: String,
    work: WorkId,
}

impl Harness {
    async fn new(responder: impl Fn(&str) -> Value + Send + Sync + 'static) -> Option<Self> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let pool = lysilogy_db::connect(&url).await.expect("connect");
        lysilogy_db::migrate(&pool).await.expect("migrate");
        let works = WorkRepository::new(pool.clone());
        let work = works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;
        let transport = RecordingTransport::new(responder);
        Some(Self {
            documents: DocumentRepository::new(pool.clone()),
            records: SourceRecordRepository::new(pool.clone()),
            acquisitions: AcquisitionRepository::new(pool.clone()),
            client: OpenAlexClient::new(transport.clone()),
            transport,
            blob_dir: tempfile::tempdir().expect("blob dir"),
            library_dir: tempfile::tempdir().expect("library dir"),
            marker: uuid::Uuid::new_v4().simple().to_string(),
            pool,
            works,
            work,
        })
    }

    fn pdf_bytes(&self) -> Vec<u8> {
        format!("%PDF-1.4 {}", self.marker).into_bytes()
    }

    fn pdf_url(&self, name: &str) -> String {
        format!("https://pdfs.example.org/{}/{name}.pdf", self.marker)
    }

    fn page_url(&self, name: &str) -> String {
        format!("https://journal.example.org/{}/{name}", self.marker)
    }

    fn title(&self) -> String {
        format!("Acquisition Test {}", self.marker)
    }

    async fn add_identifier(&self, work: WorkId, scheme: IdentifierScheme, value: &str) {
        self.works
            .add_identifier(work, scheme, value, None, IdentifierStatus::Active)
            .await
            .expect("identifier")
            .expect("identifier is new");
    }

    async fn with_doi(&self) {
        let doi = format!("10.9999/acq-{}", self.marker);
        self.add_identifier(self.work, IdentifierScheme::Doi, &doi)
            .await;
    }

    /// Give a Work a unique, well-formed arXiv ID and return its PDF URL.
    async fn arxiv_url_for(&self, work: WorkId) -> String {
        let digits = uuid::Uuid::new_v4().as_u128();
        let id = format!(
            "{:04}.{:05}",
            1000 + digits % 9000,
            (digits >> 20) % 100_000
        );
        self.add_identifier(work, IdentifierScheme::Arxiv, &id)
            .await;
        arxiv_pdf_url(&id).expect("arxiv url")
    }

    async fn with_arxiv(&self) -> String {
        self.arxiv_url_for(self.work).await
    }

    async fn with_title(&self) {
        self.works
            .set_display_title(self.work, &self.title())
            .await
            .expect("title");
        self.works
            .set_display_year(self.work, 2016)
            .await
            .expect("year");
    }

    async fn run_for(
        &self,
        work: WorkId,
        fetcher: &dyn PdfFetcher,
        picker: Option<&dyn LandingLinkPicker>,
        email: Option<&str>,
        options: AcquisitionOptions,
    ) -> Result<AcquisitionSummary> {
        let throttle = HostThrottle::disabled();
        let context = AcquisitionContext {
            works: &self.works,
            documents: &self.documents,
            records: &self.records,
            acquisitions: &self.acquisitions,
            openalex: &self.client,
            fetcher,
            throttle: &throttle,
            picker,
            contact_email: email,
            blob_root: &self.blob_dir.path().join("blobs"),
            library_root: self.library_dir.path(),
        };
        run_acquisition(&context, work, options).await
    }

    async fn run(&self, fetcher: &dyn PdfFetcher) -> AcquisitionSummary {
        self.run_for(
            self.work,
            fetcher,
            None,
            None,
            AcquisitionOptions::default(),
        )
        .await
        .expect("run")
    }

    async fn run_with_picker(
        &self,
        fetcher: &dyn PdfFetcher,
        picker: &dyn LandingLinkPicker,
    ) -> AcquisitionSummary {
        self.run_for(
            self.work,
            fetcher,
            Some(picker),
            None,
            AcquisitionOptions::default(),
        )
        .await
        .expect("run")
    }

    async fn row(&self) -> AcquisitionRow {
        self.acquisitions
            .get(self.work)
            .await
            .expect("get")
            .expect("row")
    }

    fn blob_file(&self, sha256: &str) -> std::path::PathBuf {
        blob_path(&self.blob_dir.path().join("blobs"), sha256)
    }

    async fn default_workspace(&self) -> WorkspaceId {
        self.documents
            .ensure_default_workspace()
            .await
            .expect("workspace")
    }

    /// Register a library document with the given bytes without writing a file.
    async fn register_document(&self, relative_path: &str, bytes: &[u8]) -> DocumentId {
        let workspace = self.default_workspace().await;
        let file = ScannedFile {
            relative_path: relative_path.to_owned(),
            sha256: sha_hex(bytes),
            bytes: i64::try_from(bytes.len()).expect("small"),
            format: DocumentFormat::Pdf,
        };
        self.documents
            .apply_partial_scan(workspace, &[file])
            .await
            .expect("scan");
        self.documents
            .find_by_path(workspace, relative_path)
            .await
            .expect("find")
            .expect("document")
            .id
    }

    async fn tombstone(&self, document: DocumentId) {
        sqlx::query("update reader.document set relative_path = null where id = $1")
            .bind(document.as_uuid())
            .execute(&self.pool)
            .await
            .expect("tombstone");
    }

    async fn count(&self, sql: &str, bind: &str) -> i64 {
        sqlx::query_scalar(sql)
            .bind(bind)
            .fetch_one(&self.pool)
            .await
            .expect("count")
    }
}

macro_rules! harness {
    ($responder:expr) => {
        match Harness::new($responder).await {
            Some(harness) => harness,
            None => {
                eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
                return;
            }
        }
    };
}

#[tokio::test]
async fn should_map_work_when_arxiv_pdf_downloads() {
    let h = harness!(|_| nothing_offered());
    h.with_title().await;
    let url = h.with_arxiv().await;
    let bytes = h.pdf_bytes();
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(bytes.clone()));

    let summary = h.run(&fetcher).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(summary.state, AcquisitionState::Mapped);
    assert_eq!(
        summary.stages,
        vec![
            AcquisitionState::IdentifierFound,
            AcquisitionState::LinkFound,
            AcquisitionState::Downloaded,
            AcquisitionState::Mapped
        ]
    );
    assert_eq!(summary.source, Some(CandidateOrigin::Arxiv));
    assert_eq!(summary.pdf_url.as_deref(), Some(url.as_str()));
    assert_eq!(summary.bytes, Some(i64::try_from(bytes.len()).unwrap()));
    let sha = sha_hex(&bytes);
    assert_eq!(summary.sha256.as_deref(), Some(sha.as_str()));
    let relative = format!("Fetched/2016 - {}.pdf", h.title());
    assert_eq!(summary.relative_path.as_deref(), Some(relative.as_str()));

    // Blob row and file.
    assert_eq!(std::fs::read(h.blob_file(&sha)).expect("blob file"), bytes);
    assert_eq!(
        h.count(
            "select count(*) from reader.blob where storage_key = $1",
            &blob_storage_key(&sha)
        )
        .await,
        1
    );
    // Library copy is a real file with the same bytes.
    let library_file = h.library_dir.path().join(&relative);
    assert_eq!(std::fs::read(&library_file).expect("library copy"), bytes);
    // Document linked by acquisition, version points at the blob.
    let row = h.row().await;
    assert_eq!(row.state, AcquisitionState::Mapped);
    let document = row.document_id.expect("document");
    let link = h
        .documents
        .work_link(document)
        .await
        .expect("link")
        .expect("linked");
    assert_eq!(link.work_id, h.work);
    assert_eq!(link.method, WorkLinkMethod::Acquisition);
    assert_eq!(link.detail.as_deref(), Some("acquired:arxiv:arxiv.org"));
    let version = row.document_version_id.expect("version");
    let blob_id: Option<uuid::Uuid> =
        sqlx::query_scalar("select blob_id from reader.document_version where id = $1")
            .bind(version.as_uuid())
            .fetch_one(&h.pool)
            .await
            .expect("version blob");
    assert_eq!(blob_id, row.blob_id.map(lysilogy_core::BlobId::as_uuid));
    assert!(blob_id.is_some());
    assert_eq!(row.attempts, 1);
    assert!(
        row.lease_until.is_none(),
        "the lease is released after the run"
    );
}

#[tokio::test]
async fn should_not_contact_network_when_rerun_after_mapping() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    let url = h.with_arxiv().await;
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(h.pdf_bytes()));
    assert_eq!(h.run(&fetcher).await.outcome, AcquisitionOutcome::Mapped);
    let fetch_count = fetcher.calls();
    let lookups = h.transport.urls().len();
    assert_eq!(fetch_count, 1);
    assert_eq!(lookups, 1, "the first run asked OpenAlex once");

    let summary = h.run(&fetcher).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::AlreadyMapped);
    assert_eq!(summary.state, AcquisitionState::Mapped);
    assert!(summary.stages.is_empty());
    assert!(summary.relative_path.is_some());
    assert_eq!(fetcher.calls(), fetch_count);
    assert_eq!(h.transport.urls().len(), lookups);
    assert_eq!(h.row().await.attempts, 1, "a no-op rerun is not an attempt");
}

#[tokio::test]
async fn should_resume_from_link_found_when_previous_run_failed_transiently() {
    let pdf = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&pdf);
    let h = harness!(move |_| {
        let url = seen.lock().expect("lock").clone();
        oa_response(Some(&url), None, &[], None)
    });
    *pdf.lock().unwrap() = h.pdf_url("paper");
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new().with_sequence(
        &h.pdf_url("paper"),
        vec![
            ScriptedResponse::Error(FetchError::Transient("http_503".to_owned())),
            ScriptedResponse::Pdf(h.pdf_bytes()),
        ],
    );

    let first = h.run(&fetcher).await;

    assert_eq!(
        first.outcome,
        AcquisitionOutcome::Pending {
            error: "http_503".to_owned()
        }
    );
    assert_eq!(first.state, AcquisitionState::LinkFound);
    let row = h.row().await;
    assert_eq!(row.attempts, 1);
    assert_eq!(row.last_error.as_deref(), Some("http_503"));
    assert_eq!(row.candidates.len(), 1);
    assert_eq!(h.transport.urls().len(), 1);

    let second = h.run(&fetcher).await;

    assert_eq!(second.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(
        second.stages,
        vec![AcquisitionState::Downloaded, AcquisitionState::Mapped],
        "identifier and link stages are skipped"
    );
    assert_eq!(h.transport.urls().len(), 1, "OpenAlex is not asked again");
    assert_eq!(fetcher.calls(), 2);
    assert_eq!(h.row().await.last_error, None);
}

#[tokio::test]
async fn should_mark_unavailable_no_identifier_when_work_has_no_identifiers() {
    let h = harness!(|_| nothing_offered());
    let fetcher = ScriptedFetcher::new();

    let summary = h.run(&fetcher).await;

    assert_eq!(
        summary.outcome,
        AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::NoIdentifier
        }
    );
    assert_eq!(summary.state, AcquisitionState::Unavailable);
    assert_eq!(fetcher.calls(), 0);
    assert!(h.transport.urls().is_empty());
    let row = h.row().await;
    assert_eq!(
        row.unavailable_reason,
        Some(UnavailableReason::NoIdentifier)
    );
    assert!(row.checked_at.is_some());
    assert_eq!(row.attempts, 0, "no network work, no attempt");
}

#[tokio::test]
async fn should_mark_unavailable_no_oa_location_when_providers_offer_nothing() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new();

    let summary = h.run(&fetcher).await;

    assert_eq!(
        summary.outcome,
        AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::NoOaLocation
        }
    );
    assert_eq!(
        summary.stages,
        vec![AcquisitionState::IdentifierFound],
        "link_found is never reached"
    );
    assert_eq!(fetcher.calls(), 0);
    assert_eq!(h.row().await.attempts, 1, "the provider lookup counts");
}

#[tokio::test]
async fn should_mark_unavailable_all_links_failed_when_every_candidate_is_permanent() {
    let url_slot = Arc::new(std::sync::Mutex::new((String::new(), String::new())));
    let slot = Arc::clone(&url_slot);
    let h = harness!(move |_| {
        let (best, oa_url) = slot.lock().expect("lock").clone();
        oa_response(Some(&best), Some(&oa_url), &[], None)
    });
    *url_slot.lock().unwrap() = (h.pdf_url("a"), h.pdf_url("b"));
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new()
        .with(
            &h.pdf_url("a"),
            ScriptedResponse::Error(FetchError::Permanent("http_403".to_owned())),
        )
        .with(
            &h.pdf_url("b"),
            ScriptedResponse::Error(FetchError::Permanent("http_404".to_owned())),
        );

    let summary = h.run(&fetcher).await;

    assert_eq!(
        summary.outcome,
        AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::AllLinksFailed
        }
    );
    assert_eq!(summary.tried, 2);
    assert_eq!(summary.candidates, 2);
    let row = h.row().await;
    assert_eq!(row.state, AcquisitionState::Unavailable);
    let outcomes: Vec<&str> = row
        .trials
        .iter()
        .map(|trial| trial.outcome.as_str())
        .collect();
    assert_eq!(outcomes, ["permanent:http_403", "permanent:http_404"]);
}

#[tokio::test]
async fn should_skip_unavailable_within_recheck_window_when_not_retrying() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new();
    h.run(&fetcher).await;
    let lookups = h.transport.urls().len();

    let summary = h.run(&fetcher).await;

    let checked_at = h.row().await.checked_at.expect("checked_at");
    assert_eq!(
        summary.outcome,
        AcquisitionOutcome::SkippedUnavailable {
            reason: UnavailableReason::NoOaLocation,
            checked_at
        }
    );
    assert_eq!(summary.state, AcquisitionState::Unavailable);
    assert_eq!(h.transport.urls().len(), lookups);
    assert_eq!(fetcher.calls(), 0);
}

#[tokio::test]
async fn should_look_again_when_recheck_window_has_passed() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new();
    h.run(&fetcher).await;
    sqlx::query(
        "update ops.acquisition set checked_at = now() - interval '31 days' where work_id = $1",
    )
    .bind(h.work.as_uuid())
    .execute(&h.pool)
    .await
    .expect("age");

    let summary = h.run(&fetcher).await;

    assert!(matches!(
        summary.outcome,
        AcquisitionOutcome::Unavailable { .. }
    ));
    assert_eq!(
        h.transport.urls().len(),
        2,
        "an expired verdict is rechecked"
    );
}

#[tokio::test]
async fn should_retry_and_bypass_cache_when_retry_unavailable_set() {
    let offer = Arc::new(AtomicBool::new(false));
    let pdf = Arc::new(std::sync::Mutex::new(String::new()));
    let (offer_seen, pdf_seen) = (Arc::clone(&offer), Arc::clone(&pdf));
    let h = harness!(move |_| {
        if offer_seen.load(Ordering::SeqCst) {
            oa_response(Some(&pdf_seen.lock().expect("lock")), None, &[], None)
        } else {
            nothing_offered()
        }
    });
    *pdf.lock().unwrap() = h.pdf_url("late");
    h.with_doi().await;
    let fetcher =
        ScriptedFetcher::new().with(&h.pdf_url("late"), ScriptedResponse::Pdf(h.pdf_bytes()));
    assert!(matches!(
        h.run(&fetcher).await.outcome,
        AcquisitionOutcome::Unavailable { .. }
    ));
    offer.store(true, Ordering::SeqCst);
    assert!(matches!(
        h.run(&fetcher).await.outcome,
        AcquisitionOutcome::SkippedUnavailable { .. }
    ));
    assert_eq!(h.transport.urls().len(), 1);

    let summary = h
        .run_for(
            h.work,
            &fetcher,
            None,
            None,
            AcquisitionOptions {
                retry_unavailable: true,
            },
        )
        .await
        .expect("retry");

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(
        h.transport.urls().len(),
        2,
        "the cached empty answer was bypassed"
    );
    assert_eq!(h.row().await.unavailable_reason, None);
}

#[tokio::test]
async fn should_reject_html_disguised_as_pdf_when_magic_missing() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| oa_response(Some(&seen.lock().expect("lock")), None, &[], None));
    *slot.lock().unwrap() = h.pdf_url("fake");
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new().with(
        &h.pdf_url("fake"),
        ScriptedResponse::Pdf(b"<html><body>Please log in</body></html>".to_vec()),
    );

    let summary = h.run(&fetcher).await;

    assert_eq!(
        summary.outcome,
        AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::AllLinksFailed
        }
    );
    let row = h.row().await;
    assert_eq!(row.trials[0].outcome, "permanent:not_a_pdf");
    assert_eq!(row.blob_id, None);
    let leftovers = std::fs::read_dir(h.blob_dir.path().join("blobs/tmp"))
        .expect("tmp dir")
        .count();
    assert_eq!(leftovers, 0, "the rejected download is deleted");
}

fn landing_with_meta(pdf: &str) -> String {
    format!(
        "<html><head><meta name=\"citation_pdf_url\" content=\"{pdf}\"></head>\
         <body>Abstract</body></html>"
    )
}

fn landing_oa(landing: &str) -> Value {
    oa_response(None, None, &[], Some(landing))
}

#[tokio::test]
async fn should_follow_citation_pdf_url_when_candidate_is_landing_page() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| landing_oa(&seen.lock().expect("lock")));
    *slot.lock().unwrap() = h.page_url("article");
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new()
        .with(
            &h.page_url("article"),
            ScriptedResponse::Html(landing_with_meta(&h.pdf_url("full"))),
        )
        .with(&h.pdf_url("full"), ScriptedResponse::Pdf(h.pdf_bytes()));

    let summary = h.run(&fetcher).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(summary.source, Some(CandidateOrigin::LandingMeta));
    assert_eq!(summary.pdf_url, Some(h.pdf_url("full")));
    assert_eq!(fetcher.urls(), [h.page_url("article"), h.pdf_url("full")]);
    let row = h.row().await;
    let link = h
        .documents
        .work_link(row.document_id.expect("document"))
        .await
        .expect("link")
        .expect("linked");
    assert_eq!(
        link.detail.as_deref(),
        Some("acquired:landing_meta:pdfs.example.org")
    );
}

fn landing_without_pdf_signal(download: &str) -> String {
    format!(
        "<html><body><a href=\"{}/issue\">Current issue</a> \
         <a href=\"{download}\">Download full text</a></body></html>",
        "https://journal.example.org/other"
    )
}

#[tokio::test]
async fn should_not_call_picker_when_model_fallback_disabled() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| landing_oa(&seen.lock().expect("lock")));
    *slot.lock().unwrap() = h.page_url("article");
    h.with_doi().await;
    let hop = format!("https://files.example.org/{}/get?id=1", h.marker);
    let fetcher = ScriptedFetcher::new()
        .with(
            &h.page_url("article"),
            ScriptedResponse::Html(landing_without_pdf_signal(&hop)),
        )
        .with(&hop, ScriptedResponse::Pdf(h.pdf_bytes()));

    let summary = h.run(&fetcher).await;

    assert_eq!(
        summary.outcome,
        AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::AllLinksFailed
        }
    );
    assert_eq!(fetcher.calls(), 1, "only the landing page was fetched");
    assert_eq!(
        h.row().await.trials[0].outcome,
        "permanent:landing_no_pdf_link"
    );
}

#[tokio::test]
async fn should_not_call_picker_when_meta_link_found() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| landing_oa(&seen.lock().expect("lock")));
    *slot.lock().unwrap() = h.page_url("article");
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new()
        .with(
            &h.page_url("article"),
            ScriptedResponse::Html(landing_with_meta(&h.pdf_url("full"))),
        )
        .with(&h.pdf_url("full"), ScriptedResponse::Pdf(h.pdf_bytes()));
    let picker = ScriptedLinkPicker::new("fake:model", |_| {
        Ok(LinkPick {
            pdf_link_index: None,
            reason: "unused".to_owned(),
        })
    });

    let summary = h.run_with_picker(&fetcher, &picker).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(picker.calls(), 0, "the deterministic path wins");
}

fn download_picker(model: &str) -> ScriptedLinkPicker {
    ScriptedLinkPicker::new(model, |request| {
        Ok(LinkPick {
            pdf_link_index: request
                .links
                .iter()
                .position(|link| link.text == "Download full text"),
            reason: "the download link".to_owned(),
        })
    })
}

#[tokio::test]
async fn should_call_picker_and_fetch_grounded_link_when_meta_missing() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| landing_oa(&seen.lock().expect("lock")));
    *slot.lock().unwrap() = h.page_url("article");
    h.with_doi().await;
    h.with_title().await;
    let hop = format!("https://files.example.org/{}/get?id=1", h.marker);
    let fetcher = ScriptedFetcher::new()
        .with(
            &h.page_url("article"),
            ScriptedResponse::Html(landing_without_pdf_signal(&hop)),
        )
        .with(&hop, ScriptedResponse::Pdf(h.pdf_bytes()));
    let picker = download_picker("fake:model");

    let summary = h.run_with_picker(&fetcher, &picker).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(summary.source, Some(CandidateOrigin::Model));
    assert_eq!(summary.pdf_url.as_deref(), Some(hop.as_str()));
    assert_eq!(picker.calls(), 1);
    assert_eq!(fetcher.urls(), [h.page_url("article"), hop]);
}

#[tokio::test]
async fn should_reject_pick_when_index_out_of_range() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| landing_oa(&seen.lock().expect("lock")));
    *slot.lock().unwrap() = h.page_url("article");
    h.with_doi().await;
    let hop = format!("https://files.example.org/{}/get?id=1", h.marker);
    let fetcher = ScriptedFetcher::new().with(
        &h.page_url("article"),
        ScriptedResponse::Html(landing_without_pdf_signal(&hop)),
    );
    let picker = ScriptedLinkPicker::new("fake:model", |_| {
        Ok(LinkPick {
            pdf_link_index: Some(99),
            reason: "hallucinated".to_owned(),
        })
    });

    let summary = h.run_with_picker(&fetcher, &picker).await;

    assert_eq!(
        summary.outcome,
        AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::AllLinksFailed
        }
    );
    assert!(
        summary
            .warnings
            .contains(&"pick_rejected:index_out_of_range".to_owned()),
        "{:?}",
        summary.warnings
    );
    assert_eq!(fetcher.calls(), 1, "an ungrounded pick is never fetched");
    assert_eq!(h.row().await.trials[0].outcome, "permanent:pick_rejected");
}

#[tokio::test]
async fn should_reuse_cached_pick_when_rerun() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| landing_oa(&seen.lock().expect("lock")));
    *slot.lock().unwrap() = h.page_url("article");
    h.with_doi().await;
    h.with_title().await;
    let hop = format!("https://files.example.org/{}/get?id=1", h.marker);
    let fetcher = ScriptedFetcher::new()
        .with(
            &h.page_url("article"),
            ScriptedResponse::Html(landing_without_pdf_signal(&hop)),
        )
        .with_sequence(
            &hop,
            vec![
                ScriptedResponse::Error(FetchError::Transient("http_503".to_owned())),
                ScriptedResponse::Pdf(h.pdf_bytes()),
            ],
        );
    let picker = download_picker("fake:model");

    let first = h.run_with_picker(&fetcher, &picker).await;
    assert!(matches!(first.outcome, AcquisitionOutcome::Pending { .. }));
    assert_eq!(picker.calls(), 1);

    let second = h.run_with_picker(&fetcher, &picker).await;

    assert_eq!(second.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(picker.calls(), 1, "the pick came from source_record");
    assert!(
        h.count(
            "select count(*) from source.source_record \
             where kind = 'acquisition_link_pick' and source_key like $1",
            "fake:model|%"
        )
        .await
            > 0
    );
}

#[tokio::test]
async fn should_not_cache_pick_and_retry_later_when_picker_fails() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| landing_oa(&seen.lock().expect("lock")));
    *slot.lock().unwrap() = h.page_url("article");
    h.with_doi().await;
    let hop = format!("https://files.example.org/{}/get?id=1", h.marker);
    let fetcher = ScriptedFetcher::new().with(
        &h.page_url("article"),
        ScriptedResponse::Html(landing_without_pdf_signal(&hop)),
    );
    let picker = ScriptedLinkPicker::new("fake:down", |_| {
        Err(PickerError::Unavailable("claude not installed".to_owned()))
    });

    let first = h.run_with_picker(&fetcher, &picker).await;
    let second = h.run_with_picker(&fetcher, &picker).await;

    assert!(matches!(first.outcome, AcquisitionOutcome::Pending { .. }));
    assert!(matches!(second.outcome, AcquisitionOutcome::Pending { .. }));
    assert_eq!(picker.calls(), 2, "errors are not cached");
    assert!(first.warnings.contains(&"link_picker_failed".to_owned()));
}

#[tokio::test]
async fn should_skip_unpaywall_and_warn_when_no_contact_email() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new();

    let summary = h.run(&fetcher).await;

    assert!(
        summary
            .warnings
            .contains(&"unpaywall_skipped_no_email".to_owned()),
        "{:?}",
        summary.warnings
    );
    assert!(
        h.transport
            .urls()
            .iter()
            .all(|url| !url.contains("unpaywall")),
        "{:?}",
        h.transport.urls()
    );
}

#[tokio::test]
async fn should_query_unpaywall_when_contact_email_configured() {
    let slot = Arc::new(std::sync::Mutex::new(String::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |url| {
        if url.contains("api.unpaywall.org") {
            json!({"best_oa_location": {"url_for_pdf": seen.lock().expect("lock").clone()}})
        } else {
            nothing_offered()
        }
    });
    *slot.lock().unwrap() = h.pdf_url("unpaywall");
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new().with(
        &h.pdf_url("unpaywall"),
        ScriptedResponse::Pdf(h.pdf_bytes()),
    );

    let summary = h
        .run_for(
            h.work,
            &fetcher,
            None,
            Some(EMAIL),
            AcquisitionOptions::default(),
        )
        .await
        .expect("run");

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(summary.source, Some(CandidateOrigin::UnpaywallBest));
    assert!(!summary.warnings.iter().any(|w| w.contains("unpaywall")));
    let urls = h.transport.urls();
    assert!(
        urls.iter()
            .any(|url| url.contains("api.unpaywall.org") && url.contains("email=tester")),
        "{urls:?}"
    );
}

#[tokio::test]
async fn should_link_existing_library_document_when_sha_already_present() {
    let h = harness!(|_| nothing_offered());
    h.with_title().await;
    let url = h.with_arxiv().await;
    let bytes = h.pdf_bytes();
    let existing = format!("Existing/{}.pdf", h.marker);
    let document = h.register_document(&existing, &bytes).await;
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(bytes));

    let summary = h.run(&fetcher).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(summary.relative_path.as_deref(), Some(existing.as_str()));
    assert!(
        !h.library_dir.path().join("Fetched").exists(),
        "no copy is written for a document the library already has"
    );
    let row = h.row().await;
    assert_eq!(row.document_id, Some(document));
    let link = h
        .documents
        .work_link(document)
        .await
        .expect("link")
        .expect("linked");
    assert_eq!(
        (link.work_id, link.method),
        (h.work, WorkLinkMethod::Acquisition)
    );
    assert!(row.blob_id.is_some());
}

#[tokio::test]
async fn should_not_relink_document_when_linked_to_another_work() {
    let h = harness!(|_| nothing_offered());
    let url = h.with_arxiv().await;
    let bytes = h.pdf_bytes();
    let document = h
        .register_document(&format!("Existing/{}.pdf", h.marker), &bytes)
        .await;
    let other = h
        .works
        .create(WorkKind::Article, Visibility::Private)
        .await
        .expect("other")
        .id;
    h.documents
        .set_work_link(document, other, WorkLinkMethod::Manual, None, "test")
        .await
        .expect("link other");
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(bytes));

    let summary = h.run(&fetcher).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert!(
        summary
            .warnings
            .contains(&format!("document_linked_elsewhere:{other}")),
        "{:?}",
        summary.warnings
    );
    let link = h
        .documents
        .work_link(document)
        .await
        .expect("link")
        .expect("linked");
    assert_eq!((link.work_id, link.method), (other, WorkLinkMethod::Manual));
    assert_eq!(h.row().await.document_id, Some(document));
}

#[tokio::test]
async fn should_remap_when_mapped_document_was_tombstoned() {
    let h = harness!(|_| nothing_offered());
    h.with_title().await;
    let url = h.with_arxiv().await;
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(h.pdf_bytes()));
    let first = h.run(&fetcher).await;
    assert_eq!(first.outcome, AcquisitionOutcome::Mapped);
    let document = h.row().await.document_id.expect("document");
    h.tombstone(document).await;

    let second = h.run(&fetcher).await;

    assert_eq!(second.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(second.state, AcquisitionState::Mapped);
    assert_eq!(second.stages, vec![AcquisitionState::Mapped]);
    assert_eq!(
        fetcher.calls(),
        1,
        "the stored blob is reused, not refetched"
    );
    let row = h.row().await;
    let path = h
        .documents
        .document_presence(row.document_id.expect("document"))
        .await
        .expect("presence")
        .expect("document exists");
    assert_eq!(path, second.relative_path, "the document is present again");
    assert_eq!(
        h.run(&fetcher).await.outcome,
        AcquisitionOutcome::AlreadyMapped
    );
}

#[tokio::test]
async fn should_fall_back_to_link_found_when_blob_file_missing() {
    let h = harness!(|_| nothing_offered());
    h.with_title().await;
    let url = h.with_arxiv().await;
    let bytes = h.pdf_bytes();
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(bytes.clone()));
    h.run(&fetcher).await;
    let row = h.row().await;
    h.tombstone(row.document_id.expect("document")).await;
    std::fs::remove_file(h.blob_file(&sha_hex(&bytes))).expect("remove blob");

    let second = h.run(&fetcher).await;

    assert_eq!(
        second.outcome,
        AcquisitionOutcome::Pending {
            error: "blob_missing".to_owned()
        }
    );
    assert!(second.warnings.contains(&"blob_missing".to_owned()));
    assert_eq!(second.state, AcquisitionState::LinkFound);
    assert_eq!(fetcher.calls(), 1, "falling back does not fetch by itself");

    let third = h.run(&fetcher).await;

    assert_eq!(third.outcome, AcquisitionOutcome::Mapped);
    assert_eq!(fetcher.calls(), 2, "the next run downloads again");
    assert!(h.blob_file(&sha_hex(&bytes)).exists());
}

#[tokio::test]
async fn should_fall_back_to_link_found_when_blob_size_differs() {
    let h = harness!(|_| nothing_offered());
    let url = h.with_arxiv().await;
    let bytes = h.pdf_bytes();
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(bytes.clone()));
    h.run(&fetcher).await;
    let row = h.row().await;
    h.tombstone(row.document_id.expect("document")).await;
    std::fs::write(h.blob_file(&sha_hex(&bytes)), b"%PDF-truncated").expect("truncate");

    let second = h.run(&fetcher).await;

    assert!(matches!(second.outcome, AcquisitionOutcome::Pending { .. }));
    assert_eq!(second.state, AcquisitionState::LinkFound);
}

#[tokio::test]
async fn should_return_busy_when_lease_held() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    assert!(h.acquisitions.claim(h.work).await.expect("claim"));
    let fetcher = ScriptedFetcher::new();

    let summary = h.run(&fetcher).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::Busy);
    assert_eq!(fetcher.calls(), 0);
    assert!(h.transport.urls().is_empty());
    let row = h.row().await;
    assert!(
        row.lease_until.is_some(),
        "a busy run must not release someone else's lease"
    );
    assert_eq!(row.state, AcquisitionState::Unresolved);
}

#[tokio::test]
async fn should_short_circuit_already_local_when_work_has_present_document() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    let relative = format!("Local/{}.pdf", h.marker);
    let document = h
        .register_document(&relative, format!("%PDF-local {}", h.marker).as_bytes())
        .await;
    h.documents
        .set_work_link(document, h.work, WorkLinkMethod::Identifier, None, "test")
        .await
        .expect("link");
    let fetcher = ScriptedFetcher::new();

    let summary = h.run(&fetcher).await;

    assert_eq!(summary.outcome, AcquisitionOutcome::AlreadyLocal);
    assert_eq!(summary.state, AcquisitionState::Mapped);
    assert_eq!(summary.relative_path.as_deref(), Some(relative.as_str()));
    assert_eq!(fetcher.calls(), 0);
    assert!(h.transport.urls().is_empty());
    assert_eq!(h.row().await.document_id, Some(document));
}

#[tokio::test]
async fn should_follow_redirect_when_work_was_merged() {
    let h = harness!(|_| nothing_offered());
    let survivor = h
        .works
        .create(WorkKind::Article, Visibility::Private)
        .await
        .expect("survivor")
        .id;
    let url = h.arxiv_url_for(survivor).await;
    sqlx::query("insert into graph.work_redirect (old_work_id, new_work_id) values ($1, $2)")
        .bind(h.work.as_uuid())
        .bind(survivor.as_uuid())
        .execute(&h.pool)
        .await
        .expect("redirect");
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(h.pdf_bytes()));

    let summary = h
        .run_for(h.work, &fetcher, None, None, AcquisitionOptions::default())
        .await
        .expect("run");

    assert_eq!(summary.work_id, survivor);
    assert_eq!(summary.outcome, AcquisitionOutcome::Mapped);
    assert!(
        h.acquisitions.get(h.work).await.expect("get").is_none(),
        "no row is created for the merged-away Work"
    );
}

#[tokio::test]
async fn should_report_not_found_when_work_does_not_exist() {
    let h = harness!(|_| nothing_offered());
    let fetcher = ScriptedFetcher::new();

    let result = h
        .run_for(
            WorkId::new(),
            &fetcher,
            None,
            None,
            AcquisitionOptions::default(),
        )
        .await;

    assert!(
        matches!(result, Err(IngestError::Db(ref error)) if error.is_not_found()),
        "{result:?}"
    );
}

#[tokio::test]
async fn should_skip_candidates_whose_last_trial_failed_permanently_when_not_retrying() {
    let slot = Arc::new(std::sync::Mutex::new((String::new(), String::new())));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| {
        let (best, oa_url) = seen.lock().expect("lock").clone();
        oa_response(Some(&best), Some(&oa_url), &[], None)
    });
    *slot.lock().unwrap() = (h.pdf_url("dead"), h.pdf_url("flaky"));
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new()
        .with(
            &h.pdf_url("dead"),
            ScriptedResponse::Error(FetchError::Permanent("http_404".to_owned())),
        )
        .with_sequence(
            &h.pdf_url("flaky"),
            vec![
                ScriptedResponse::Error(FetchError::Transient("http_429".to_owned())),
                ScriptedResponse::Pdf(h.pdf_bytes()),
            ],
        );

    let first = h.run(&fetcher).await;
    assert!(matches!(first.outcome, AcquisitionOutcome::Pending { .. }));
    let second = h.run(&fetcher).await;

    assert_eq!(second.outcome, AcquisitionOutcome::Mapped);
    let urls = fetcher.urls();
    let dead_requests = urls.iter().filter(|url| **url == h.pdf_url("dead")).count();
    assert_eq!(dead_requests, 1, "{urls:?}");
    assert_eq!(second.tried, 1);
}

#[tokio::test]
async fn should_continue_with_untried_candidates_when_run_limit_reached() {
    let slot = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&slot);
    let h = harness!(move |_| oa_response(None, None, &seen.lock().expect("lock"), None));
    *slot.lock().unwrap() = (0..7)
        .map(|index| h.pdf_url(&format!("c{index}")))
        .collect();
    h.with_doi().await;
    let fetcher = ScriptedFetcher::new();

    let first = h.run(&fetcher).await;

    assert_eq!(first.tried, MAX_CANDIDATES_PER_RUN);
    assert_eq!(first.candidates, 7);
    let AcquisitionOutcome::Pending { error } = first.outcome else {
        panic!("expected pending, got {:?}", first.outcome);
    };
    assert!(error.starts_with("candidate_limit_reached"), "{error}");

    let second = h.run(&fetcher).await;

    assert_eq!(second.tried, 1, "the six dead links are not retried");
    assert_eq!(
        second.outcome,
        AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::AllLinksFailed
        }
    );
    assert_eq!(fetcher.calls(), 7);
}

#[test]
fn should_serialize_outcome_with_snake_case_kind_when_encoding() {
    assert_eq!(
        serde_json::to_value(AcquisitionOutcome::AlreadyMapped).expect("json"),
        json!({"kind": "already_mapped"})
    );
    assert_eq!(
        serde_json::to_value(AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::NoOaLocation
        })
        .expect("json"),
        json!({"kind": "unavailable", "reason": "no_oa_location"})
    );
    assert_eq!(
        serde_json::to_value(AcquisitionOutcome::Pending {
            error: "http_503".to_owned()
        })
        .expect("json"),
        json!({"kind": "pending", "error": "http_503"})
    );
}

#[test]
fn should_classify_provider_errors_when_deciding_pending_or_unavailable() {
    assert_eq!(
        classify_provider_error(ProviderError::Status(503)).expect("ok"),
        Some("http_503".to_owned())
    );
    assert_eq!(
        classify_provider_error(ProviderError::Status(429)).expect("ok"),
        Some("http_429".to_owned())
    );
    assert!(
        classify_provider_error(ProviderError::Request("timeout".to_owned()))
            .expect("ok")
            .is_some()
    );
    assert_eq!(
        classify_provider_error(ProviderError::Status(403)).expect("ok"),
        None
    );
    assert_eq!(
        classify_provider_error(ProviderError::Malformed).expect("ok"),
        None
    );
}

#[tokio::test]
async fn should_restart_resolution_when_already_local_document_is_tombstoned() {
    let h = harness!(|_| nothing_offered());
    h.with_title().await;
    let url = h.with_arxiv().await;
    let relative = format!("Local/{}.pdf", h.marker);
    let document = h
        .register_document(&relative, format!("%PDF-local {}", h.marker).as_bytes())
        .await;
    h.documents
        .set_work_link(document, h.work, WorkLinkMethod::Identifier, None, "test")
        .await
        .expect("link");
    let fetcher = ScriptedFetcher::new().with(&url, ScriptedResponse::Pdf(h.pdf_bytes()));
    let first = h.run(&fetcher).await;
    assert_eq!(first.outcome, AcquisitionOutcome::AlreadyLocal);
    h.tombstone(document).await;

    let second = h.run(&fetcher).await;

    assert!(matches!(second.outcome, AcquisitionOutcome::Pending { .. }));
    assert_eq!(second.state, AcquisitionState::Unresolved);
    assert_eq!(fetcher.calls(), 0);

    let third = h.run(&fetcher).await;

    assert_eq!(third.outcome, AcquisitionOutcome::Mapped, "{third:?}");
    assert_eq!(fetcher.calls(), 1, "the real candidates were tried");
}

#[tokio::test]
async fn should_persist_warnings_when_recording_them_on_the_row() {
    let h = harness!(|_| nothing_offered());
    h.with_doi().await;
    h.acquisitions.claim(h.work).await.expect("claim");

    h.acquisitions
        .record_warnings(h.work, &[])
        .await
        .expect("empty warnings");
    assert_eq!(h.row().await.last_error, None);

    h.acquisitions
        .record_warnings(h.work, &["title_not_in_front_matter".to_owned()])
        .await
        .expect("warnings");
    assert_eq!(
        h.row().await.last_error.as_deref(),
        Some("warnings: title_not_in_front_matter")
    );
}
