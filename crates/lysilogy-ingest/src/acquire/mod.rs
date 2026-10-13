//! Open-access acquisition: find where a Work's full text is legally available,
//! fetch it politely, store it content-addressed, and map it into the library.
//!
//! [`run_acquisition`] is a resumable state machine over `ops.acquisition`: each
//! stage writes its output to the row before the next starts, so a rerun skips
//! finished stages and a transient failure resumes where it stopped.

pub mod fetch;
pub mod landing;
pub mod locate;
pub mod picker;
pub mod store;
pub mod verify;

pub use fetch::{
    ContentClass, FetchError, FetchFuture, FetchRequest, FetchedResource, HostThrottle,
    MAX_HTML_BYTES, MAX_PDF_BYTES, PdfFetcher, ScriptedFetcher, ScriptedResponse,
    classify_content_type, classify_status, has_pdf_magic, throttle_delay,
};
pub use landing::{PageLink, page_links, pdf_link_from_meta};
pub use locate::{
    AcquisitionIdentifiers, LinkCandidate, OPENALEX_OA_KIND, UNPAYWALL_KIND, arxiv_pdf_url,
    fetch_openalex_oa_cached, fetch_unpaywall_cached, identifiers_from, normalize_candidate_url,
    openalex_oa_url, parse_openalex_oa, parse_unpaywall, rank_candidates, unpaywall_url,
};
pub use picker::{
    LINK_PICK_PROMPT_VERSION, LINK_PICK_RECORD_KIND, LINK_PICK_SCHEMA, LandingLinkPicker, LinkPick,
    LinkPickRequest, MAX_PICK_LINKS, PickFuture, PickerError, ScriptedLinkPicker, ground_pick,
    link_pick_cache_key, link_pick_prompt, pick_cached,
};
pub use store::{
    FETCHED_DIR, blob_path, blob_storage_key, fetched_filename, place_library_copy, store_blob,
};
pub use verify::title_appears_in;

use std::io;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use lysilogy_core::{
    AcquisitionState, CandidateOrigin, DocumentFormat, DocumentId, DocumentVersionId, ScannedFile,
    SourceRecordId, UnavailableReason, WorkId, WorkLinkMethod, WorkspaceId,
};
use lysilogy_db::acquisition::{
    AcquisitionRepository, AcquisitionRow, CandidateRecord, DownloadRecord, TrialRecord,
};
use lysilogy_db::document::DocumentRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use reqwest::Url;
use serde::Serialize;
use tokio::io::AsyncReadExt as _;

use crate::openalex::{JsonTransport, OpenAlexClient, ProviderError};
use crate::{IngestError, Result};

/// Most candidates tried in one run.
pub const MAX_CANDIDATES_PER_RUN: usize = 6;

/// How long an `unavailable` verdict is trusted before a rerun looks again.
pub const UNAVAILABLE_RECHECK_DAYS: i64 = 30;

/// Everything a run needs. The fetcher, throttle and picker are injected so
/// tests and the live app share one code path.
pub struct AcquisitionContext<'a, T: JsonTransport> {
    pub works: &'a WorkRepository,
    pub documents: &'a DocumentRepository,
    pub records: &'a SourceRecordRepository,
    pub acquisitions: &'a AcquisitionRepository,
    pub openalex: &'a OpenAlexClient<T>,
    pub fetcher: &'a dyn PdfFetcher,
    pub throttle: &'a HostThrottle,
    pub picker: Option<&'a dyn LandingLinkPicker>,
    pub contact_email: Option<&'a str>,
    pub blob_root: &'a Path,
    pub library_root: &'a Path,
}

/// Per-run switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AcquisitionOptions {
    /// Look again at a Work marked unavailable, bypassing cached provider answers.
    pub retry_unavailable: bool,
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcquisitionOutcome {
    /// A copy was downloaded and mapped to a library document.
    Mapped,
    /// An earlier run already mapped it and the document is still present.
    AlreadyMapped,
    /// The Work already had a local copy; nothing was fetched.
    AlreadyLocal,
    /// No copy can be obtained.
    Unavailable { reason: UnavailableReason },
    /// Marked unavailable recently; not looked at again.
    SkippedUnavailable {
        reason: UnavailableReason,
        checked_at: DateTime<Utc>,
    },
    /// A transient failure; a rerun resumes.
    Pending { error: String },
    /// Another runner holds this Work's lease.
    Busy,
}

/// The result of one run.
#[derive(Clone, Debug, Serialize)]
pub struct AcquisitionSummary {
    pub work_id: WorkId,
    pub outcome: AcquisitionOutcome,
    pub state: AcquisitionState,
    /// Stages reached by this run, in order.
    pub stages: Vec<AcquisitionState>,
    pub candidates: usize,
    pub tried: usize,
    pub source: Option<CandidateOrigin>,
    pub pdf_url: Option<String>,
    pub sha256: Option<String>,
    pub bytes: Option<i64>,
    pub document_id: Option<DocumentId>,
    pub relative_path: Option<String>,
    /// Stable `snake_case` codes with short context.
    pub warnings: Vec<String>,
}

/// Whether a run reached the network, and whether that is already counted as
/// an attempt (a recorded failure counts itself).
#[derive(Clone, Copy, PartialEq, Eq)]
enum NetworkUse {
    None,
    Used,
    Counted,
}

/// Mutable bookkeeping for one run.
struct Run {
    work: WorkId,
    retrying: bool,
    refresh: bool,
    stages: Vec<AcquisitionState>,
    warnings: Vec<String>,
    tried: usize,
    network: NetworkUse,
    relative_path: Option<String>,
    winning_record: Option<SourceRecordId>,
}

impl Run {
    const fn new(work: WorkId, retrying: bool) -> Self {
        Self {
            work,
            retrying,
            refresh: false,
            stages: Vec::new(),
            warnings: Vec::new(),
            tried: 0,
            network: NetworkUse::None,
            relative_path: None,
            winning_record: None,
        }
    }

    fn used_network(&mut self) {
        if self.network == NetworkUse::None {
            self.network = NetworkUse::Used;
        }
    }

    fn warn(&mut self, warning: String) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }
}

/// A downloaded PDF waiting in the temp directory.
struct Download {
    origin: CandidateOrigin,
    pdf_url: String,
    final_url: String,
    temp_path: std::path::PathBuf,
    sha256: String,
    bytes: i64,
    record: Option<SourceRecordId>,
}

enum Attempt {
    Downloaded(Download),
    /// The candidate failed; carries the message when the failure was transient.
    Failed(Option<String>),
}

enum LandingChoice {
    Link(String, CandidateOrigin),
    /// No usable link; carries the trial reason.
    NoLink(&'static str),
    /// The picker itself failed; retry later.
    PickerFailed(String),
}

/// Take a Work from "no copy" to a mapped library document, as far as it can go.
///
/// Follows `work_redirect` first. A second concurrent run for the same Work
/// returns [`AcquisitionOutcome::Busy`] and touches nothing. The lease is
/// always released.
pub async fn run_acquisition<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    work: WorkId,
    options: AcquisitionOptions,
) -> Result<AcquisitionSummary> {
    let work = ctx.works.resolve_redirect(work).await?;
    if ctx.works.get(work).await?.is_none() {
        return Err(IngestError::Db(lysilogy_db::DbError::not_found()));
    }
    if !ctx.acquisitions.claim(work).await? {
        return summarize(ctx, Run::new(work, false), AcquisitionOutcome::Busy).await;
    }
    let result = run_claimed(ctx, work, options).await;
    if let Err(error) = ctx.acquisitions.release(work).await {
        tracing::warn!(%work, %error, "failed to release acquisition lease");
    }
    result
}

async fn run_claimed<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    work: WorkId,
    options: AcquisitionOptions,
) -> Result<AcquisitionSummary> {
    let mut run = Run::new(work, options.retry_unavailable);
    let mut row = load_row(ctx, work).await?;
    let workspace = ctx.documents.ensure_default_workspace().await?;

    if row.state == AcquisitionState::Mapped {
        let path = match row.document_id {
            Some(document) => ctx.documents.document_presence(document).await?.flatten(),
            None => None,
        };
        if let Some(path) = path {
            run.relative_path = Some(path);
            return summarize(ctx, run, AcquisitionOutcome::AlreadyMapped).await;
        }
        // The document was tombstoned or deleted: map the stored blob again.
        ctx.acquisitions
            .set_state(work, AcquisitionState::Downloaded)
            .await?;
        row = load_row(ctx, work).await?;
    }

    if row.state == AcquisitionState::Unavailable {
        if let (false, Some(reason), Some(checked_at)) = (
            options.retry_unavailable,
            row.unavailable_reason,
            row.checked_at,
        ) && checked_at + Duration::days(UNAVAILABLE_RECHECK_DAYS) > Utc::now()
        {
            let outcome = AcquisitionOutcome::SkippedUnavailable { reason, checked_at };
            return summarize(ctx, run, outcome).await;
        }
        ctx.acquisitions.reset_to_unresolved(work).await?;
        run.refresh = true;
        row = load_row(ctx, work).await?;
    }

    let local = ctx
        .documents
        .local_documents_for_works(workspace, &[work])
        .await?;
    if let Some(local) = local.into_iter().next() {
        ctx.acquisitions
            .mark_mapped(work, local.document_id, None)
            .await?;
        run.stages.push(AcquisitionState::Mapped);
        run.relative_path = Some(local.relative_path);
        return summarize(ctx, run, AcquisitionOutcome::AlreadyLocal).await;
    }

    if row.state == AcquisitionState::Unresolved {
        if let Some(outcome) = stage_identifiers(ctx, &mut run).await? {
            return summarize(ctx, run, outcome).await;
        }
        row = load_row(ctx, work).await?;
    }
    if row.state == AcquisitionState::IdentifierFound {
        if let Some(outcome) = stage_links(ctx, &mut run, &row).await? {
            return summarize(ctx, run, outcome).await;
        }
        row = load_row(ctx, work).await?;
    }
    if row.state == AcquisitionState::LinkFound {
        if let Some(outcome) = stage_download(ctx, &mut run, &row).await? {
            return summarize(ctx, run, outcome).await;
        }
        row = load_row(ctx, work).await?;
    }
    if row.state == AcquisitionState::Downloaded {
        let outcome = stage_map(ctx, &mut run, &row, workspace).await?;
        return summarize(ctx, run, outcome).await;
    }
    Err(IngestError::Io(io::Error::other(format!(
        "acquisition stopped in unexpected state {}",
        row.state.as_str()
    ))))
}

async fn load_row<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    work: WorkId,
) -> Result<AcquisitionRow> {
    ctx.acquisitions
        .get(work)
        .await?
        .ok_or_else(|| IngestError::Db(lysilogy_db::DbError::not_found()))
}

async fn summarize<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: Run,
    outcome: AcquisitionOutcome,
) -> Result<AcquisitionSummary> {
    if run.network == NetworkUse::Used {
        ctx.acquisitions.bump_attempts(run.work).await?;
    }
    let row = ctx.acquisitions.get(run.work).await?;
    let Some(row) = row else {
        return Ok(AcquisitionSummary {
            work_id: run.work,
            outcome,
            state: AcquisitionState::Unresolved,
            stages: run.stages,
            candidates: 0,
            tried: run.tried,
            source: None,
            pdf_url: None,
            sha256: None,
            bytes: None,
            document_id: None,
            relative_path: run.relative_path,
            warnings: run.warnings,
        });
    };
    Ok(AcquisitionSummary {
        work_id: run.work,
        outcome,
        state: row.state,
        stages: run.stages,
        candidates: row.candidates.len(),
        tried: run.tried,
        source: row.source,
        pdf_url: row.pdf_url,
        sha256: row.sha256,
        bytes: row.bytes,
        document_id: row.document_id,
        relative_path: run.relative_path,
        warnings: run.warnings,
    })
}

async fn stage_identifiers<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
) -> Result<Option<AcquisitionOutcome>> {
    let stored = ctx.works.active_identifiers(run.work).await?;
    let identifiers = identifiers_from(&stored);
    if identifiers.is_empty() {
        ctx.acquisitions
            .mark_unavailable(run.work, UnavailableReason::NoIdentifier)
            .await?;
        return Ok(Some(AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::NoIdentifier,
        }));
    }
    let value = serde_json::to_value(&identifiers).map_err(io::Error::other)?;
    ctx.acquisitions
        .record_identifiers(run.work, &value)
        .await?;
    run.stages.push(AcquisitionState::IdentifierFound);
    Ok(None)
}

/// How a provider failure affects the run.
fn classify_provider_error(error: ProviderError) -> Result<Option<String>> {
    match error {
        ProviderError::Db(error) => Err(IngestError::Db(error)),
        ProviderError::Request(message) => Ok(Some(message)),
        ProviderError::Status(status @ (408 | 425 | 429 | 500..=599)) => {
            Ok(Some(format!("http_{status}")))
        }
        ProviderError::Status(_) | ProviderError::Malformed => Ok(None),
    }
}

async fn stage_links<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    row: &AcquisitionRow,
) -> Result<Option<AcquisitionOutcome>> {
    let identifiers: AcquisitionIdentifiers = row
        .identifiers
        .clone()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let mut found: Vec<LinkCandidate> = Vec::new();
    let mut transient: Option<String> = None;

    if let Some(url) = identifiers.arxiv.as_deref().and_then(arxiv_pdf_url) {
        found.push(LinkCandidate {
            url,
            origin: CandidateOrigin::Arxiv,
            source_record_id: None,
        });
    }

    if let Some(url) = openalex_oa_url(&identifiers) {
        ctx.throttle.wait(&url).await;
        run.used_network();
        match fetch_openalex_oa_cached(ctx.records, ctx.openalex, &identifiers, run.refresh).await {
            Ok(Some((record, value))) => found.extend(parse_openalex_oa(&value, Some(record))),
            Ok(None) => {}
            Err(error) => {
                run.warn("openalex_lookup_failed".to_owned());
                transient = classify_provider_error(error)?.or(transient);
            }
        }
    }

    let email = ctx.contact_email.filter(|email| !email.trim().is_empty());
    match (identifiers.doi.as_deref(), email) {
        (Some(doi), Some(email)) => {
            ctx.throttle.wait(&unpaywall_url(doi, email)).await;
            run.used_network();
            match fetch_unpaywall_cached(ctx.records, ctx.openalex, doi, email, run.refresh).await {
                Ok((record, value)) => found.extend(parse_unpaywall(&value, Some(record))),
                Err(error) => {
                    run.warn("unpaywall_lookup_failed".to_owned());
                    transient = classify_provider_error(error)?.or(transient);
                }
            }
        }
        (Some(_), None) => run.warn("unpaywall_skipped_no_email".to_owned()),
        _ => {}
    }

    let ranked = rank_candidates(found);
    if ranked.is_empty() {
        if let Some(error) = transient {
            return Ok(Some(pending(ctx, run, error).await?));
        }
        ctx.acquisitions
            .mark_unavailable(run.work, UnavailableReason::NoOaLocation)
            .await?;
        return Ok(Some(AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::NoOaLocation,
        }));
    }
    if let Some(error) = transient {
        run.warn(format!("provider_transient:{error}"));
    }
    let records: Vec<CandidateRecord> = ranked.into_iter().map(CandidateRecord::from).collect();
    ctx.acquisitions
        .record_candidates(run.work, &records)
        .await?;
    run.stages.push(AcquisitionState::LinkFound);
    Ok(None)
}

async fn pending<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    error: String,
) -> Result<AcquisitionOutcome> {
    ctx.acquisitions.record_failure(run.work, &error).await?;
    run.network = NetworkUse::Counted;
    Ok(AcquisitionOutcome::Pending { error })
}

/// Whether the candidate's most recent trial ended in a permanent failure.
fn failed_permanently(trials: &[TrialRecord], url: &str) -> bool {
    trials
        .iter()
        .rev()
        .find(|trial| trial.url == url)
        .is_some_and(|trial| trial.outcome.starts_with("permanent:"))
}

async fn stage_download<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    row: &AcquisitionRow,
) -> Result<Option<AcquisitionOutcome>> {
    let temp_dir = ctx.blob_root.join("tmp");
    tokio::fs::create_dir_all(&temp_dir).await?;
    let work = ctx.works.get(run.work).await?;
    let title = work.as_ref().and_then(|work| work.display_title.clone());
    let year = work.as_ref().and_then(|work| work.display_year);

    let mut last_transient: Option<String> = None;
    let mut untried = 0usize;
    for candidate in &row.candidates {
        if !run.retrying && failed_permanently(&row.trials, &candidate.url) {
            continue;
        }
        if run.tried >= MAX_CANDIDATES_PER_RUN {
            untried += 1;
            continue;
        }
        run.tried += 1;
        let attempt = try_candidate(ctx, run, candidate, &temp_dir, (&title, year)).await?;
        match attempt {
            Attempt::Downloaded(download) => {
                save_download(ctx, run, download).await?;
                return Ok(None);
            }
            Attempt::Failed(Some(error)) => last_transient = Some(error),
            Attempt::Failed(None) => {}
        }
    }

    if let Some(error) = last_transient {
        return Ok(Some(pending(ctx, run, error).await?));
    }
    if untried > 0 {
        let message = format!("candidate_limit_reached:{untried}_untried");
        return Ok(Some(pending(ctx, run, message).await?));
    }
    ctx.acquisitions
        .mark_unavailable(run.work, UnavailableReason::AllLinksFailed)
        .await?;
    Ok(Some(AcquisitionOutcome::Unavailable {
        reason: UnavailableReason::AllLinksFailed,
    }))
}

async fn save_download<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    download: Download,
) -> Result<()> {
    store_blob(ctx.blob_root, &download.temp_path, &download.sha256).await?;
    let blob = ctx
        .documents
        .upsert_blob(
            &blob_storage_key(&download.sha256),
            &download.sha256,
            download.bytes,
            "application/pdf",
        )
        .await?;
    ctx.acquisitions
        .record_download(
            run.work,
            DownloadRecord {
                source: download.origin,
                pdf_url: download.pdf_url,
                final_url: download.final_url,
                sha256: download.sha256,
                bytes: download.bytes,
                blob,
            },
        )
        .await?;
    run.winning_record = download.record;
    run.stages.push(AcquisitionState::Downloaded);
    Ok(())
}

async fn log_trial<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &Run,
    url: &str,
    origin: CandidateOrigin,
    outcome: String,
) -> Result<()> {
    ctx.acquisitions
        .push_trial(
            run.work,
            TrialRecord {
                url: url.to_owned(),
                origin,
                outcome,
                at: Utc::now(),
            },
        )
        .await?;
    Ok(())
}

fn error_outcome(error: &FetchError) -> String {
    let kind = if error.is_transient() {
        "transient"
    } else {
        "permanent"
    };
    format!("{kind}:{}", error.reason())
}

/// The reason a downloaded file is not an acceptable PDF, if any. The runner
/// checks this itself so no fetcher implementation can skip it.
async fn reject_pdf(temp_path: &Path, bytes: i64) -> Option<&'static str> {
    // MAX_PDF_BYTES is 100 MiB, far below i64::MAX.
    #[allow(clippy::cast_possible_wrap)]
    if bytes > MAX_PDF_BYTES as i64 {
        return Some("too_large");
    }
    let Ok(file) = tokio::fs::File::open(temp_path).await else {
        return Some("unreadable");
    };
    let mut head = Vec::with_capacity(1024);
    let read = file.take(1024).read_to_end(&mut head).await;
    if read.is_err() {
        return Some("unreadable");
    }
    (!has_pdf_magic(&head)).then_some("not_a_pdf")
}

async fn fetch_checked<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    url: &str,
    temp_dir: &Path,
) -> std::result::Result<FetchedResource, FetchError> {
    run.used_network();
    ctx.throttle.wait(url).await;
    let resource = ctx.fetcher.fetch(FetchRequest { url, temp_dir }).await?;
    if let FetchedResource::Pdf {
        temp_path, bytes, ..
    } = &resource
        && let Some(reason) = reject_pdf(temp_path, *bytes).await
    {
        let _ = tokio::fs::remove_file(temp_path).await;
        return Err(FetchError::Permanent(reason.to_owned()));
    }
    Ok(resource)
}

async fn try_candidate<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    candidate: &CandidateRecord,
    temp_dir: &Path,
    work_brief: (&Option<String>, Option<i32>),
) -> Result<Attempt> {
    let fetched = fetch_checked(ctx, run, &candidate.url, temp_dir).await;
    match fetched {
        Err(error) => {
            log_trial(
                ctx,
                run,
                &candidate.url,
                candidate.origin,
                error_outcome(&error),
            )
            .await?;
            Ok(Attempt::Failed(
                error.is_transient().then(|| error.reason().to_owned()),
            ))
        }
        Ok(FetchedResource::Pdf {
            final_url,
            temp_path,
            sha256,
            bytes,
        }) => {
            log_trial(ctx, run, &candidate.url, candidate.origin, "pdf".to_owned()).await?;
            Ok(Attempt::Downloaded(Download {
                origin: candidate.origin,
                pdf_url: candidate.url.clone(),
                final_url,
                temp_path,
                sha256,
                bytes,
                record: candidate.source_record_id,
            }))
        }
        Ok(FetchedResource::Html { final_url, body }) => {
            follow_landing(ctx, run, candidate, &final_url, &body, temp_dir, work_brief).await
        }
    }
}

async fn choose_landing_link<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    page_url: &str,
    body: &str,
    work_brief: (&Option<String>, Option<i32>),
) -> Result<LandingChoice> {
    if let Some(href) = pdf_link_from_meta(body, page_url) {
        return Ok(LandingChoice::Link(href, CandidateOrigin::LandingMeta));
    }
    let Some(picker) = ctx.picker else {
        return Ok(LandingChoice::NoLink("landing_no_pdf_link"));
    };
    let links = page_links(body, page_url, MAX_PICK_LINKS);
    if links.is_empty() {
        return Ok(LandingChoice::NoLink("landing_no_pdf_link"));
    }
    let request = LinkPickRequest {
        work_title: work_brief.0.clone(),
        work_year: work_brief.1,
        page_url: page_url.to_owned(),
        links,
    };
    let pick = match pick_cached(ctx.records, picker, &request, run.refresh).await? {
        Ok(pick) => pick,
        Err(error) => {
            run.warn("link_picker_failed".to_owned());
            return Ok(LandingChoice::PickerFailed(error.to_string()));
        }
    };
    match ground_pick(&request, &pick) {
        Ok(Some(link)) => Ok(LandingChoice::Link(link.href, CandidateOrigin::Model)),
        Ok(None) => Ok(LandingChoice::NoLink("pick_none")),
        Err(code) => {
            run.warn(format!("pick_rejected:{code}"));
            Ok(LandingChoice::NoLink("pick_rejected"))
        }
    }
}

async fn follow_landing<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    candidate: &CandidateRecord,
    page_url: &str,
    body: &str,
    temp_dir: &Path,
    work_brief: (&Option<String>, Option<i32>),
) -> Result<Attempt> {
    let choice = choose_landing_link(ctx, run, page_url, body, work_brief).await?;
    let (link, origin) = match choice {
        LandingChoice::Link(link, origin) if link != page_url && link != candidate.url => {
            (link, origin)
        }
        LandingChoice::Link(..) => {
            let outcome = "permanent:landing_links_to_itself".to_owned();
            log_trial(ctx, run, &candidate.url, candidate.origin, outcome).await?;
            return Ok(Attempt::Failed(None));
        }
        LandingChoice::NoLink(reason) => {
            let outcome = format!("permanent:{reason}");
            log_trial(ctx, run, &candidate.url, candidate.origin, outcome).await?;
            return Ok(Attempt::Failed(None));
        }
        LandingChoice::PickerFailed(message) => {
            let outcome = format!("transient:{message}");
            log_trial(ctx, run, &candidate.url, candidate.origin, outcome).await?;
            return Ok(Attempt::Failed(Some(message)));
        }
    };

    let hop = fetch_checked(ctx, run, &link, temp_dir).await;
    let hop = match hop {
        Ok(FetchedResource::Html { .. }) => Err(FetchError::Permanent("landing_chain".to_owned())),
        other => other,
    };
    match hop {
        Ok(FetchedResource::Pdf {
            final_url,
            temp_path,
            sha256,
            bytes,
        }) => {
            log_trial(ctx, run, &link, origin, "pdf".to_owned()).await?;
            log_trial(
                ctx,
                run,
                &candidate.url,
                candidate.origin,
                "pdf_via_landing".to_owned(),
            )
            .await?;
            Ok(Attempt::Downloaded(Download {
                origin,
                pdf_url: link,
                final_url,
                temp_path,
                sha256,
                bytes,
                record: candidate.source_record_id,
            }))
        }
        Ok(FetchedResource::Html { .. }) => unreachable!("html hops are rejected above"),
        Err(error) => {
            log_trial(ctx, run, &link, origin, error_outcome(&error)).await?;
            let outcome = if error.is_transient() {
                error_outcome(&error)
            } else {
                "permanent:landing_hop_failed".to_owned()
            };
            log_trial(ctx, run, &candidate.url, candidate.origin, outcome).await?;
            Ok(Attempt::Failed(
                error.is_transient().then(|| error.reason().to_owned()),
            ))
        }
    }
}

fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

async fn stage_map<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    row: &AcquisitionRow,
    workspace: WorkspaceId,
) -> Result<AcquisitionOutcome> {
    let stored = row
        .sha256
        .clone()
        .zip(row.bytes)
        .zip(row.blob_id)
        .map(|((sha256, bytes), blob)| (sha256, bytes, blob));
    let blob_ok = match &stored {
        Some((sha256, bytes, _)) => tokio::fs::metadata(blob_path(ctx.blob_root, sha256))
            .await
            .is_ok_and(|metadata| {
                // A stored size is never negative; compare as unsigned.
                #[allow(clippy::cast_sign_loss)]
                let expected = *bytes as u64;
                metadata.len() == expected
            }),
        None => false,
    };
    let Some((sha256, bytes, blob)) = stored.filter(|_| blob_ok) else {
        run.warn("blob_missing".to_owned());
        ctx.acquisitions
            .set_state(run.work, AcquisitionState::LinkFound)
            .await?;
        return Ok(AcquisitionOutcome::Pending {
            error: "blob_missing".to_owned(),
        });
    };

    let existing = ctx
        .documents
        .present_document_for_sha(workspace, &sha256)
        .await?;
    let (document, version, relative_path, linked_work) = match existing {
        Some(found) => found,
        None => register_library_copy(ctx, run, workspace, &sha256, bytes).await?,
    };

    ctx.documents.set_version_blob(version, blob).await?;
    link_document(ctx, run, row, document, linked_work).await?;
    ctx.acquisitions
        .mark_mapped(run.work, document, Some(version))
        .await?;
    run.relative_path = Some(relative_path);
    run.stages.push(AcquisitionState::Mapped);
    Ok(AcquisitionOutcome::Mapped)
}

async fn register_library_copy<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &Run,
    workspace: WorkspaceId,
    sha256: &str,
    bytes: i64,
) -> Result<(DocumentId, DocumentVersionId, String, Option<WorkId>)> {
    let work = ctx.works.get(run.work).await?;
    let filename = fetched_filename(
        work.as_ref().and_then(|work| work.display_title.as_deref()),
        work.as_ref().and_then(|work| work.display_year),
        run.work,
    );
    let blob = blob_path(ctx.blob_root, sha256);
    let relative_path = place_library_copy(ctx.library_root, &blob, &filename, sha256).await?;
    let file = ScannedFile {
        relative_path: relative_path.clone(),
        sha256: sha256.to_owned(),
        bytes,
        format: DocumentFormat::Pdf,
    };
    ctx.documents
        .apply_partial_scan(workspace, std::slice::from_ref(&file))
        .await?;
    let not_registered = || {
        IngestError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("document was not registered: {relative_path}"),
        ))
    };
    let document = ctx
        .documents
        .find_by_path(workspace, &relative_path)
        .await?
        .ok_or_else(not_registered)?;
    let version = ctx
        .documents
        .version_for_sha(document.id, sha256)
        .await?
        .ok_or_else(not_registered)?;
    Ok((document.id, version, relative_path, document.work_id))
}

async fn link_document<T: JsonTransport>(
    ctx: &AcquisitionContext<'_, T>,
    run: &mut Run,
    row: &AcquisitionRow,
    document: DocumentId,
    linked_work: Option<WorkId>,
) -> Result<()> {
    if let Some(other) = linked_work {
        let other = ctx.works.resolve_redirect(other).await?;
        if other != run.work {
            run.warn(format!("document_linked_elsewhere:{other}"));
        }
        return Ok(());
    }
    let origin = row.source.map_or("unknown", CandidateOrigin::as_str);
    let host = host_of(row.pdf_url.as_deref().unwrap_or_default());
    let record = run.winning_record.or_else(|| {
        row.candidates
            .iter()
            .find(|candidate| Some(&candidate.url) == row.pdf_url.as_ref())
            .and_then(|candidate| candidate.source_record_id)
    });
    ctx.documents
        .set_work_link(
            document,
            run.work,
            WorkLinkMethod::Acquisition,
            record,
            &format!("acquired:{origin}:{host}"),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
