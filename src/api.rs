use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use chrono::Utc;
use percent_encoding::percent_decode_str;
use reqwest::{
    Url,
    header::{ACCEPT, CONTENT_LENGTH, LOCATION},
};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::{Mutex, RwLock},
};
use tokio_util::io::ReaderStream;

use crate::{
    Result,
    analysis::{AnalysisService, validate_citations},
    domain::{
        AgentSession, AnalysisJob, AnalysisJobKind, AnalysisProvider, AnalyzeRequest,
        CitationStatus, Clarification, ClarifyRequest, CreateHighlightRequest, ExperimentArm,
        ExperimentCatalog, ExperimentJudgment, ExperimentJudgmentRequest, ExperimentRecord,
        ExperimentRun, ExperimentStatus, ExperimentView, ExtractedPaper, FeedbackRecord,
        FeedbackRequest, FeedbackStatus, Highlight, HighlightOrigin, LearningRamp, PaperId,
        PaperMap, PaperOverview, PaperView, ProcessingQueue, ProcessingStage, ProcessingStatus,
        PromptExperiment, RemotePdfSource, StartExperimentRequest,
    },
    error::Error,
    extract::PdfExtractor,
    jobs::JobTracker,
    library::LibraryCatalog,
    remote::{MAX_PUBLIC_REDIRECTS, public_http_client},
    store::ArtifactStore,
};

static USER_HIGHLIGHT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FEEDBACK_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static IMPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static EXPERIMENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const EXPERIMENT_CATALOG: &str = include_str!("../experiments/catalog.json");
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_REMOTE_PDF_BYTES: u64 = 100 * 1024 * 1024;
const MAX_REMOTE_URL_LENGTH: usize = 4_096;

#[derive(Clone, Debug)]
pub struct AppState {
    catalog: Arc<RwLock<LibraryCatalog>>,
    library_root: Arc<PathBuf>,
    store: ArtifactStore,
    extractor: PdfExtractor,
    analysis: AnalysisService,
    jobs: JobTracker,
    highlight_write: Arc<Mutex<()>>,
    import_write: Arc<Mutex<()>>,
    experiment_write: Arc<Mutex<()>>,
    frontend_root: Option<Arc<PathBuf>>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Debug, Deserialize)]
struct ImportPdfRequest {
    url: String,
}

#[derive(Debug, Serialize)]
struct ImportPdfResponse {
    paper: PaperOverview,
    library: LibraryResponse,
    source: RemotePdfSource,
}

#[derive(Debug, Serialize)]
pub struct LibraryResponse {
    pub name: String,
    pub papers: Vec<PaperOverview>,
}

impl AppState {
    pub async fn new(
        library_root: impl Into<PathBuf>,
        data_root: impl Into<PathBuf>,
    ) -> Result<Self> {
        Self::with_services(
            library_root,
            data_root,
            PdfExtractor::default(),
            AnalysisService::default(),
        )
        .await
    }

    pub async fn with_services(
        library_root: impl Into<PathBuf>,
        data_root: impl Into<PathBuf>,
        extractor: PdfExtractor,
        analysis: AnalysisService,
    ) -> Result<Self> {
        let library_root = library_root.into();
        let store = ArtifactStore::new(data_root);
        store.initialize().await?;
        let catalog = LibraryCatalog::scan(&library_root, &store).await?;
        let jobs = JobTracker::load(store.clone()).await?;
        Ok(Self {
            catalog: Arc::new(RwLock::new(catalog)),
            library_root: Arc::new(library_root),
            store,
            extractor,
            analysis,
            jobs,
            highlight_write: Arc::new(Mutex::new(())),
            import_write: Arc::new(Mutex::new(())),
            experiment_write: Arc::new(Mutex::new(())),
            frontend_root: None,
        })
    }

    pub async fn library(&self) -> LibraryResponse {
        let catalog = self.catalog.read().await;
        LibraryResponse {
            name: catalog
                .root()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Articles")
                .to_owned(),
            papers: catalog.overviews(),
        }
    }

    pub async fn refresh(&self) -> Result<LibraryResponse> {
        let replacement = LibraryCatalog::scan(self.library_root.as_ref(), &self.store).await?;
        self.catalog.write().await.replace_with(replacement);
        Ok(self.library().await)
    }

    async fn import_remote_pdf(&self, value: &str) -> Result<ImportPdfResponse> {
        let original = parse_remote_pdf_url(value)?;
        let import_guard = self.import_write.lock().await;
        let temporary_path = self.library_root.join(format!(
            ".lysilogy-import-{}-{}.tmp",
            std::process::id(),
            IMPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let download = download_remote_pdf(&original, &temporary_path).await;
        let (final_url, byte_length) = match download {
            Ok(download) => download,
            Err(error) => {
                remove_temporary_import(&temporary_path).await;
                return Err(error);
            }
        };
        let imported = self
            .finish_remote_import(&original, &final_url, byte_length, &temporary_path)
            .await;
        drop(import_guard);
        if imported.is_err() {
            remove_temporary_import(&temporary_path).await;
        }
        imported
    }

    async fn finish_remote_import(
        &self,
        original: &Url,
        final_url: &Url,
        byte_length: u64,
        temporary_path: &Path,
    ) -> Result<ImportPdfResponse> {
        let filename = match available_import_filename(
            self.library_root.as_ref(),
            &remote_pdf_filename(final_url),
        )
        .await
        {
            Ok(filename) => filename,
            Err(error) => return Err(error),
        };
        let final_path = self.library_root.join(&filename);
        if let Err(error) = tokio::fs::rename(temporary_path, &final_path).await {
            return Err(Error::io(&final_path, error));
        }

        let id = PaperId::from_relative_path(Path::new(&filename));
        let source = RemotePdfSource {
            original_url: original.to_string(),
            final_url: final_url.to_string(),
            imported_at: Utc::now(),
            byte_length,
        };
        self.store.save_remote_source(&id, &source).await?;
        let library = self.refresh().await?;
        let paper = library
            .papers
            .iter()
            .find(|paper| paper.id == id)
            .cloned()
            .ok_or_else(|| {
                Error::Task("imported PDF was not discovered after rescan".to_owned())
            })?;
        Ok(ImportPdfResponse {
            paper,
            library,
            source,
        })
    }

    pub async fn paper(&self, id: &PaperId) -> Result<PaperView> {
        let overview = self
            .catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.overview.clone())
            .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
        let analysis = self.store.load_analysis(id).await?;
        Ok(PaperView {
            paper: overview,
            analysis,
        })
    }

    pub async fn analyze_now(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
        force: bool,
    ) -> Result<PaperView> {
        let overview = {
            let mut catalog = self.catalog.write().await;
            let entry = catalog
                .get_mut(id)
                .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
            if !force && matches!(entry.overview.status, ProcessingStatus::Ready) {
                drop(catalog);
                return self.paper(id).await;
            }
            if matches!(
                entry.overview.status,
                ProcessingStatus::Queued { .. }
                    | ProcessingStatus::Extracting
                    | ProcessingStatus::Analyzing { .. }
            ) {
                return Err(Error::AlreadyProcessing(id.to_string()));
            }
            entry.overview.status = ProcessingStatus::Queued { provider };
            entry.overview.clone()
        };
        if let Err(error) = self
            .jobs
            .begin(
                id.clone(),
                overview.metadata.title,
                provider,
                AnalysisJobKind::Initial,
                None,
            )
            .await
        {
            self.catalog
                .write()
                .await
                .mark_failure(id, ProcessingStage::Persistence, &error);
            return Err(error);
        }

        if let Err((stage, error)) = self.run_analysis(id, provider, force).await {
            if let Err(tracking_error) = self.jobs.fail(id, stage, &error).await {
                tracing::error!(paper_id = %id, %tracking_error, "could not persist failed job state");
            }
            self.catalog.write().await.mark_failure(id, stage, &error);
            return Err(error);
        }
        self.paper(id).await
    }

    async fn queue_analysis(
        &self,
        id: PaperId,
        provider: AnalysisProvider,
        force: bool,
    ) -> Result<PaperOverview> {
        let overview = {
            let mut catalog = self.catalog.write().await;
            let entry = catalog
                .get_mut(&id)
                .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
            if !force && matches!(entry.overview.status, ProcessingStatus::Ready) {
                return Ok(entry.overview.clone());
            }
            if matches!(
                entry.overview.status,
                ProcessingStatus::Queued { .. }
                    | ProcessingStatus::Extracting
                    | ProcessingStatus::Analyzing { .. }
            ) {
                return Err(Error::AlreadyProcessing(id.to_string()));
            }
            entry.overview.status = ProcessingStatus::Queued { provider };
            let overview = entry.overview.clone();
            drop(catalog);
            overview
        };

        if let Err(error) = self
            .jobs
            .begin(
                id.clone(),
                overview.metadata.title.clone(),
                provider,
                AnalysisJobKind::Initial,
                None,
            )
            .await
        {
            self.catalog
                .write()
                .await
                .mark_failure(&id, ProcessingStage::Persistence, &error);
            return Err(error);
        }

        let state = self.clone();
        tokio::spawn(async move {
            if let Err((stage, error)) = state.run_analysis(&id, provider, force).await {
                tracing::error!(paper_id = %id, %stage, %error, "paper analysis failed");
                if let Err(tracking_error) = state.jobs.fail(&id, stage, &error).await {
                    tracing::error!(paper_id = %id, %tracking_error, "could not persist failed job state");
                }
                state.catalog.write().await.mark_failure(&id, stage, &error);
            }
        });
        Ok(overview)
    }

    async fn run_analysis(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
        reset_stages: bool,
    ) -> std::result::Result<(), (ProcessingStage, Error)> {
        self.jobs
            .transition(id, ProcessingStage::Extraction, "extract")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        let paper = self.load_or_extract(id).await?;
        self.jobs
            .task_completed(id, "extract")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;

        {
            let mut catalog = self.catalog.write().await;
            if let Some(entry) = catalog.get_mut(id) {
                entry.overview.metadata = paper.metadata.clone();
                entry.overview.status = ProcessingStatus::Analyzing { provider };
            }
        }
        self.jobs
            .transition(id, ProcessingStage::Analysis, "prefetch")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        for (task, detail) in [
            ("orientation", "Fast orientation and abstract fallback"),
            ("structure", "Structural map and exact paper evidence"),
            ("context", "Independent live external-source research"),
        ] {
            self.jobs
                .task_active(id, task, Some(detail.to_owned()))
                .await
                .map_err(|error| (ProcessingStage::Persistence, error))?;
        }
        let outcome = self
            .analysis
            .analyze(provider, &paper, &self.store.paper_dir(id), reset_stages)
            .await
            .map_err(|error| (ProcessingStage::Analysis, error))?;
        for task in ["prefetch", "orientation", "structure", "context"] {
            self.jobs
                .task_completed(id, task)
                .await
                .map_err(|error| (ProcessingStage::Persistence, error))?;
        }
        self.jobs
            .task_active(
                id,
                "verify",
                Some("Normalized model output and verified citations".to_owned()),
            )
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.jobs
            .task_completed(id, "verify")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.jobs
            .transition(id, ProcessingStage::Persistence, "persist")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        let analysis = outcome.analysis;
        let highlight_guard = self.highlight_write.lock().await;
        self.store
            .save_analysis(id, &analysis)
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        if let Some(session) = &outcome.session {
            self.store
                .save_agent_session(id, session)
                .await
                .map_err(|error| (ProcessingStage::Persistence, error))?;
        }
        drop(highlight_guard);
        self.jobs
            .task_completed(id, "persist")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.jobs
            .complete(id, outcome.session.is_some())
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        {
            let mut catalog = self.catalog.write().await;
            if let Some(entry) = catalog.get_mut(id) {
                entry.overview.status = ProcessingStatus::Ready;
                entry.overview.metadata = paper.metadata;
                entry.overview.analyzed_at = Some(analysis.generated_at);
                entry.overview.one_line_summary = Some(analysis.thesis);
            }
        }
        Ok(())
    }

    async fn load_or_extract(
        &self,
        id: &PaperId,
    ) -> std::result::Result<crate::domain::ExtractedPaper, (ProcessingStage, Error)> {
        let (source_path, fallback_metadata) = {
            let catalog = self.catalog.read().await;
            let entry = catalog.get(id).ok_or_else(|| {
                (
                    ProcessingStage::Discovery,
                    Error::PaperNotFound(id.to_string()),
                )
            })?;
            let source_path = entry.source_path.clone();
            let metadata = entry.overview.metadata.clone();
            drop(catalog);
            (source_path, metadata)
        };

        match self.store.load_extraction(id).await {
            Ok(Some(paper)) => return Ok(paper),
            Ok(None) => {}
            Err(error) => return Err((ProcessingStage::Persistence, error)),
        }

        self.set_status(id, ProcessingStatus::Extracting).await;
        let paper = self
            .extractor
            .extract(&source_path, &fallback_metadata)
            .await
            .map_err(|error| (ProcessingStage::Extraction, error))?;
        self.store
            .save_extraction(id, &paper)
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        let has_analysis = self
            .store
            .load_analysis(id)
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?
            .is_some();
        {
            let mut catalog = self.catalog.write().await;
            if let Some(entry) = catalog.get_mut(id) {
                entry.overview.metadata = paper.metadata.clone();
                entry.overview.status = if has_analysis {
                    ProcessingStatus::Ready
                } else {
                    ProcessingStatus::Extracted
                };
            }
        }
        Ok(paper)
    }

    async fn set_status(&self, id: &PaperId, status: ProcessingStatus) {
        if let Some(entry) = self.catalog.write().await.get_mut(id) {
            entry.overview.status = status;
        }
    }

    async fn source_path(&self, id: &PaperId) -> Result<PathBuf> {
        self.catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.source_path.clone())
            .ok_or_else(|| Error::PaperNotFound(id.to_string()))
    }

    pub async fn markdown(&self, id: &PaperId) -> Result<String> {
        if self.catalog.read().await.get(id).is_none() {
            return Err(Error::PaperNotFound(id.to_string()));
        }
        if let Some(markdown) = self.store.load_markdown(id).await? {
            return Ok(markdown);
        }
        if let Some(paper) = self.store.load_extraction(id).await? {
            return self.store.ensure_markdown(id, &paper).await;
        }

        {
            let mut catalog = self.catalog.write().await;
            let entry = catalog
                .get_mut(id)
                .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
            if matches!(
                entry.overview.status,
                ProcessingStatus::Queued { .. }
                    | ProcessingStatus::Extracting
                    | ProcessingStatus::Analyzing { .. }
            ) {
                return Err(Error::AlreadyProcessing(id.to_string()));
            }
            entry.overview.status = ProcessingStatus::Extracting;
            drop(catalog);
        }

        let paper = match self.load_or_extract(id).await {
            Ok(paper) => paper,
            Err((stage, error)) => {
                self.catalog.write().await.mark_failure(id, stage, &error);
                return Err(error);
            }
        };
        match self.store.ensure_markdown(id, &paper).await {
            Ok(markdown) => Ok(markdown),
            Err(error) => {
                self.catalog
                    .write()
                    .await
                    .mark_failure(id, ProcessingStage::Persistence, &error);
                Err(error)
            }
        }
    }

    pub async fn paper_map(&self, id: &PaperId) -> Result<PaperMap> {
        if self.catalog.read().await.get(id).is_none() {
            return Err(Error::PaperNotFound(id.to_string()));
        }
        let paper = match self.load_or_extract(id).await {
            Ok(paper) => paper,
            Err((stage, error)) => {
                self.catalog.write().await.mark_failure(id, stage, &error);
                return Err(error);
            }
        };
        let mut mapped_spans = Vec::new();
        if let Some(mut analysis) = self.store.load_analysis(id).await? {
            let needs_validation = analysis.schema_version < 5
                || analysis.sections.iter().any(|section| {
                    section
                        .key_quotes
                        .iter()
                        .any(|quote| quote.validation == CitationStatus::Unverified)
                });
            if needs_validation {
                let highlight_guard = self.highlight_write.lock().await;
                validate_citations(&mut analysis, &paper.layout)?;
                self.store.save_analysis(id, &analysis).await?;
                drop(highlight_guard);
            }
            mapped_spans.extend(analysis.sections.iter().map(|section| section.pages));
        }
        let highlights = self.store.load_highlights(id).await?;
        let mut layout = paper.layout;
        if !mapped_spans.is_empty() {
            layout.pages.retain(|page| {
                mapped_spans
                    .iter()
                    .any(|span| page.number >= span.start && page.number <= span.end)
            });
        }
        Ok(PaperMap { layout, highlights })
    }

    pub async fn create_highlight(
        &self,
        id: &PaperId,
        request: &CreateHighlightRequest,
    ) -> Result<Highlight> {
        if request.note.chars().count() > 4_000 {
            return Err(Error::InvalidRequest(
                "highlight notes are limited to 4,000 characters".to_owned(),
            ));
        }
        let paper_map = self.paper_map(id).await?;
        let anchor = crate::layout::anchor_for_sentence_range(
            &paper_map.layout,
            request.start_sentence_id.trim(),
            request.end_sentence_id.as_deref().map(str::trim),
        )
        .ok_or_else(|| {
            Error::InvalidRequest(
                "highlight sentence range was not found on one PDF page".to_owned(),
            )
        })?;
        let highlight_guard = self.highlight_write.lock().await;
        let mut highlights = self.store.load_highlights(id).await?;
        if let Some(existing) = highlights.iter_mut().find(|highlight| {
            matches!(highlight.origin, HighlightOrigin::User)
                && highlight.anchor.page == anchor.page
                && highlight.anchor.start_token == anchor.start_token
                && highlight.anchor.end_token == anchor.end_token
        }) {
            existing.kind = request.kind;
            existing.note = request.note.trim().to_owned();
            let existing = existing.clone();
            self.store.save_highlights(id, &highlights).await?;
            drop(highlight_guard);
            return Ok(existing);
        }

        let now = Utc::now();
        let sequence = USER_HIGHLIGHT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let highlight = Highlight {
            id: format!("user-{}-{sequence}", now.timestamp_micros()),
            origin: HighlightOrigin::User,
            kind: request.kind,
            text: anchor.exact_text.clone(),
            anchor,
            note: request.note.trim().to_owned(),
            created_at: now,
        };
        highlights.push(highlight.clone());
        self.store.save_highlights(id, &highlights).await?;
        drop(highlight_guard);
        Ok(highlight)
    }

    pub async fn delete_highlight(&self, id: &PaperId, highlight_id: &str) -> Result<()> {
        if self.catalog.read().await.get(id).is_none() {
            return Err(Error::PaperNotFound(id.to_string()));
        }
        let highlight_guard = self.highlight_write.lock().await;
        let mut highlights = self.store.load_highlights(id).await?;
        let position = highlights
            .iter()
            .position(|highlight| highlight.id == highlight_id)
            .ok_or_else(|| Error::InvalidRequest("highlight was not found".to_owned()))?;
        if !matches!(highlights[position].origin, HighlightOrigin::User) {
            return Err(Error::InvalidRequest(
                "AI highlights are regenerated from citations and cannot be deleted".to_owned(),
            ));
        }
        highlights.remove(position);
        self.store.save_highlights(id, &highlights).await?;
        drop(highlight_guard);
        Ok(())
    }

    pub async fn processing_queue(&self) -> Result<ProcessingQueue> {
        self.jobs.queue().await
    }

    pub fn experiment_catalog(&self) -> Result<ExperimentCatalog> {
        let catalog: ExperimentCatalog = serde_json::from_str(EXPERIMENT_CATALOG)?;
        if catalog.experiments.iter().any(|experiment| {
            experiment.id.trim().is_empty()
                || experiment.variants.len() != 2
                || experiment.variants.iter().any(|variant| {
                    variant.id.trim().is_empty() || variant.instruction.trim().is_empty()
                })
        }) {
            return Err(Error::Task(
                "experiment catalog must define exactly two complete variants per experiment"
                    .to_owned(),
            ));
        }
        Ok(catalog)
    }

    pub async fn start_experiment(
        &self,
        id: &PaperId,
        request: StartExperimentRequest,
    ) -> Result<ExperimentView> {
        let run = self.prepare_experiment(id, &request).await?;
        let view = self.experiment_view(run.clone()).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let fallback = run.clone();
            if let Err(error) = state.execute_experiment(run).await {
                tracing::error!(%error, "learning-ramp experiment failed");
                if let Err(save_error) = state.fail_experiment(fallback, &error).await {
                    tracing::error!(%save_error, "could not persist failed experiment");
                }
            }
        });
        Ok(view)
    }

    pub async fn run_experiment_now(
        &self,
        id: &PaperId,
        request: StartExperimentRequest,
    ) -> Result<ExperimentView> {
        let run = self.prepare_experiment(id, &request).await?;
        let fallback = run.clone();
        let run = match self.execute_experiment(run).await {
            Ok(run) => run,
            Err(error) => self.fail_experiment(fallback, &error).await?,
        };
        self.experiment_view(run).await
    }

    async fn prepare_experiment(
        &self,
        id: &PaperId,
        request: &StartExperimentRequest,
    ) -> Result<ExperimentRun> {
        if request.provider == AnalysisProvider::Heuristic {
            return Err(Error::InvalidRequest(
                "prompt experiments require the Codex or Claude reader".to_owned(),
            ));
        }
        let catalog = self.experiment_catalog()?;
        let experiment = find_experiment(&catalog, &request.experiment_id)?;
        let paper = self
            .catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.overview.clone())
            .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
        let now = Utc::now();
        let sequence = EXPERIMENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut variants = experiment.variants.clone();
        if (now.timestamp_micros().cast_unsigned() ^ sequence) & 1 == 1 {
            variants.swap(0, 1);
        }
        let run = ExperimentRun {
            schema_version: 1,
            id: format!("run-{}-{sequence}", now.timestamp_micros()),
            paper_id: id.clone(),
            paper_title: paper.metadata.title,
            experiment_id: experiment.id,
            experiment_name: experiment.name,
            question: experiment.question,
            reader_baseline: catalog.reader_baseline,
            provider: request.provider,
            status: ExperimentStatus::Running,
            model: match request.provider {
                AnalysisProvider::Codex => "gpt-5.6-terra".to_owned(),
                AnalysisProvider::Claude => "configured Claude model".to_owned(),
                AnalysisProvider::Heuristic => unreachable!(),
            },
            reasoning_effort: "medium".to_owned(),
            created_at: now,
            completed_at: None,
            arms: variants
                .into_iter()
                .enumerate()
                .map(|(index, variant)| ExperimentArm {
                    blind_label: if index == 0 { "A" } else { "B" }.to_owned(),
                    variant_id: variant.id,
                    variant_label: variant.label,
                    variant_instruction: variant.instruction,
                    output: None,
                    error: None,
                })
                .collect(),
        };
        self.store.save_experiment_run(&run).await?;
        Ok(run)
    }

    async fn fail_experiment(
        &self,
        mut run: ExperimentRun,
        error: &Error,
    ) -> Result<ExperimentRun> {
        run.status = ExperimentStatus::Failed;
        run.completed_at = Some(Utc::now());
        for arm in &mut run.arms {
            if arm.output.is_none() && arm.error.is_none() {
                arm.error = Some(error.to_string());
            }
        }
        self.store.save_experiment_run(&run).await?;
        Ok(run)
    }

    async fn execute_experiment(&self, mut run: ExperimentRun) -> Result<ExperimentRun> {
        let experiment = find_experiment(&self.experiment_catalog()?, &run.experiment_id)?;
        let paper = self
            .load_or_extract(&run.paper_id)
            .await
            .map_err(|(_, error)| error)?;
        let directory = self.store.paper_dir(&run.paper_id);
        let first_variant = crate::domain::PromptVariant {
            id: run.arms[0].variant_id.clone(),
            label: run.arms[0].variant_label.clone(),
            instruction: run.arms[0].variant_instruction.clone(),
        };
        let second_variant = crate::domain::PromptVariant {
            id: run.arms[1].variant_id.clone(),
            label: run.arms[1].variant_label.clone(),
            instruction: run.arms[1].variant_instruction.clone(),
        };
        let reader_baseline = run.reader_baseline.clone();
        let run_id = run.id.clone();
        let mut first = Box::pin(self.analysis.experiment_variant(
            run.provider,
            &paper,
            &directory,
            crate::analysis::ExperimentVariantRequest {
                experiment: &experiment,
                variant: &first_variant,
                reader_baseline: &reader_baseline,
                run_id: &run_id,
                blind_label: "A",
            },
        ));
        let mut second = Box::pin(self.analysis.experiment_variant(
            run.provider,
            &paper,
            &directory,
            crate::analysis::ExperimentVariantRequest {
                experiment: &experiment,
                variant: &second_variant,
                reader_baseline: &reader_baseline,
                run_id: &run_id,
                blind_label: "B",
            },
        ));
        tokio::select! {
            first_result = &mut first => {
                apply_experiment_arm_result(&mut run.arms[0], first_result);
                self.store.save_experiment_run(&run).await?;
                apply_experiment_arm_result(&mut run.arms[1], second.await);
            }
            second_result = &mut second => {
                apply_experiment_arm_result(&mut run.arms[1], second_result);
                self.store.save_experiment_run(&run).await?;
                apply_experiment_arm_result(&mut run.arms[0], first.await);
            }
        }
        run.status = if run.arms.iter().all(|arm| arm.output.is_some()) {
            ExperimentStatus::Completed
        } else {
            ExperimentStatus::Failed
        };
        run.completed_at = Some(Utc::now());
        self.store.save_experiment_run(&run).await?;
        Ok(run)
    }

    pub async fn experiment_runs(&self, id: &PaperId) -> Result<Vec<ExperimentView>> {
        if self.catalog.read().await.get(id).is_none() {
            return Err(Error::PaperNotFound(id.to_string()));
        }
        let judgments = self.store.load_experiment_judgments(id).await?;
        Ok(self
            .store
            .load_experiment_runs(id)
            .await?
            .into_iter()
            .map(|run| redact_experiment(run, &judgments))
            .collect())
    }

    pub async fn experiment_records(&self) -> Result<Vec<ExperimentRecord>> {
        let paper_ids = self
            .catalog
            .read()
            .await
            .overviews()
            .into_iter()
            .map(|overview| overview.id)
            .collect::<Vec<_>>();
        let mut records = Vec::new();
        for paper_id in paper_ids {
            let judgments = self.store.load_experiment_judgments(&paper_id).await?;
            for run in self.store.load_experiment_runs(&paper_id).await? {
                let judgment = judgments
                    .iter()
                    .find(|judgment| judgment.run_id == run.id)
                    .cloned();
                records.push(ExperimentRecord { run, judgment });
            }
        }
        records.sort_by(|left, right| {
            right
                .run
                .created_at
                .cmp(&left.run.created_at)
                .then_with(|| left.run.id.cmp(&right.run.id))
        });
        Ok(records)
    }

    pub async fn experiment_run(&self, id: &PaperId, run_id: &str) -> Result<ExperimentView> {
        let run = self
            .store
            .load_experiment_run(id, run_id)
            .await?
            .ok_or_else(|| Error::InvalidRequest("experiment run was not found".to_owned()))?;
        self.experiment_view(run).await
    }

    async fn experiment_view(&self, run: ExperimentRun) -> Result<ExperimentView> {
        let judgments = self.store.load_experiment_judgments(&run.paper_id).await?;
        Ok(redact_experiment(run, &judgments))
    }

    pub async fn judge_experiment(
        &self,
        id: &PaperId,
        run_id: &str,
        request: ExperimentJudgmentRequest,
    ) -> Result<ExperimentView> {
        validate_judgment(&request)?;
        let run = self
            .store
            .load_experiment_run(id, run_id)
            .await?
            .ok_or_else(|| Error::InvalidRequest("experiment run was not found".to_owned()))?;
        if run.status != ExperimentStatus::Completed {
            return Err(Error::InvalidRequest(
                "wait for both experiment arms before judging".to_owned(),
            ));
        }
        let _guard = self.experiment_write.lock().await;
        let mut judgments = self.store.load_experiment_judgments(id).await?;
        judgments.retain(|judgment| judgment.run_id != run.id);
        judgments.push(ExperimentJudgment {
            run_id: run.id.clone(),
            experiment_id: run.experiment_id.clone(),
            overall: request.overall,
            early_traction: request.early_traction,
            rigor: request.rigor,
            confidence: request.confidence,
            arm_scores: request.arm_scores,
            note: request.note.trim().to_owned(),
            submitted_at: Utc::now(),
        });
        self.store.save_experiment_judgments(id, &judgments).await?;
        Ok(redact_experiment(run, &judgments))
    }

    async fn queue_feedback(&self, id: PaperId, request: FeedbackRequest) -> Result<AnalysisJob> {
        let feedback = validate_feedback_request(&request)?;
        if self.catalog.read().await.get(&id).is_none() {
            return Err(Error::PaperNotFound(id.to_string()));
        }
        if self.store.load_analysis(&id).await?.is_none() {
            return Err(Error::InvalidRequest(
                "analyze the paper before sending revision feedback".to_owned(),
            ));
        }

        let overview = {
            let mut catalog = self.catalog.write().await;
            let entry = catalog
                .get_mut(&id)
                .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
            if matches!(
                entry.overview.status,
                ProcessingStatus::Queued { .. }
                    | ProcessingStatus::Extracting
                    | ProcessingStatus::Analyzing { .. }
            ) {
                return Err(Error::AlreadyProcessing(id.to_string()));
            }
            entry.overview.status = ProcessingStatus::Queued {
                provider: request.provider,
            };
            let overview = entry.overview.clone();
            drop(catalog);
            overview
        };

        let job = match self
            .jobs
            .begin(
                id.clone(),
                overview.metadata.title,
                request.provider,
                AnalysisJobKind::Revision,
                Some(feedback.clone()),
            )
            .await
        {
            Ok(job) => job,
            Err(error) => {
                self.catalog
                    .write()
                    .await
                    .mark_failure(&id, ProcessingStage::Persistence, &error);
                return Err(error);
            }
        };

        let feedback_id = match self
            .begin_feedback_record(&id, &feedback, request.provider)
            .await
        {
            Ok(feedback_id) => feedback_id,
            Err(error) => {
                if let Err(tracking_error) = self
                    .jobs
                    .fail(&id, ProcessingStage::Persistence, &error)
                    .await
                {
                    tracing::error!(paper_id = %id, %tracking_error, "could not persist failed feedback job");
                }
                self.catalog
                    .write()
                    .await
                    .mark_failure(&id, ProcessingStage::Persistence, &error);
                return Err(error);
            }
        };

        let state = self.clone();
        tokio::spawn(async move {
            if let Err((stage, error)) = state
                .run_feedback(&id, request.provider, &feedback, &feedback_id)
                .await
            {
                tracing::error!(paper_id = %id, %stage, %error, "paper feedback revision failed");
                if let Err(tracking_error) = state.jobs.fail(&id, stage, &error).await {
                    tracing::error!(paper_id = %id, %tracking_error, "could not persist failed feedback job");
                }
                if let Err(feedback_error) = state
                    .finish_feedback(
                        &id,
                        &feedback_id,
                        FeedbackStatus::Failed,
                        None,
                        Some(&error),
                    )
                    .await
                {
                    tracing::error!(paper_id = %id, %feedback_error, "could not persist failed feedback record");
                }
                state.catalog.write().await.mark_failure(&id, stage, &error);
            }
        });
        Ok(job)
    }

    async fn begin_feedback_record(
        &self,
        id: &PaperId,
        feedback: &str,
        provider: AnalysisProvider,
    ) -> Result<String> {
        let now = Utc::now();
        let sequence = FEEDBACK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let feedback_id = format!("feedback-{}-{sequence}", now.timestamp_micros());
        let mut records = self.store.load_feedback(id).await?;
        records.push(FeedbackRecord {
            id: feedback_id.clone(),
            feedback: feedback.to_owned(),
            provider,
            status: FeedbackStatus::Queued,
            submitted_at: now,
            completed_at: None,
            session_id: None,
            error: None,
        });
        self.store.save_feedback(id, &records).await?;
        Ok(feedback_id)
    }

    async fn feedback_context(
        &self,
        id: &PaperId,
    ) -> std::result::Result<(ExtractedPaper, Option<AgentSession>), (ProcessingStage, Error)> {
        let paper = self.load_or_extract(id).await?;
        if self
            .store
            .load_analysis(id)
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?
            .is_none()
        {
            return Err((
                ProcessingStage::Analysis,
                Error::InvalidRequest(
                    "the current atlas disappeared before feedback could be applied".to_owned(),
                ),
            ));
        }
        let session = match self.store.load_agent_session(id).await {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(paper_id = %id, %error, "saved agent session is unusable; retrying from artifact context");
                None
            }
        };
        Ok((paper, session))
    }

    async fn run_feedback(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
        feedback: &str,
        feedback_id: &str,
    ) -> std::result::Result<(), (ProcessingStage, Error)> {
        self.jobs
            .transition(id, ProcessingStage::Analysis, "context")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        let (paper, session) = self.feedback_context(id).await?;
        self.jobs
            .task_completed(id, "context")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.jobs
            .task_active(
                id,
                "feedback",
                Some("Resuming the previous agent when its session is available".to_owned()),
            )
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.set_status(id, ProcessingStatus::Analyzing { provider })
            .await;
        let outcome = self
            .analysis
            .revise(
                provider,
                &paper,
                &self.store.paper_dir(id),
                feedback,
                session.as_ref(),
            )
            .await
            .map_err(|error| (ProcessingStage::Analysis, error))?;
        for task in ["feedback", "revise", "evidence"] {
            self.jobs
                .task_completed(id, task)
                .await
                .map_err(|error| (ProcessingStage::Persistence, error))?;
        }
        self.jobs
            .transition(id, ProcessingStage::Persistence, "persist")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;

        let analysis = outcome.analysis;
        let final_session = outcome.session.as_ref().or(session.as_ref());
        let highlight_guard = self.highlight_write.lock().await;
        self.store
            .save_analysis(id, &analysis)
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        if let Some(agent_session) = final_session {
            self.store
                .save_agent_session(id, agent_session)
                .await
                .map_err(|error| (ProcessingStage::Persistence, error))?;
        }
        drop(highlight_guard);
        self.finish_feedback(
            id,
            feedback_id,
            FeedbackStatus::Applied,
            final_session.map(|agent_session| agent_session.session_id.as_str()),
            None,
        )
        .await
        .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.jobs
            .task_completed(id, "persist")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.jobs
            .complete(id, final_session.is_some())
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        {
            let mut catalog = self.catalog.write().await;
            if let Some(entry) = catalog.get_mut(id) {
                entry.overview.status = ProcessingStatus::Ready;
                entry.overview.metadata = paper.metadata;
                entry.overview.analyzed_at = Some(analysis.generated_at);
                entry.overview.one_line_summary = Some(analysis.thesis);
            }
        }
        Ok(())
    }

    async fn finish_feedback(
        &self,
        id: &PaperId,
        feedback_id: &str,
        status: FeedbackStatus,
        session_id: Option<&str>,
        error: Option<&Error>,
    ) -> Result<()> {
        let mut records = self.store.load_feedback(id).await?;
        let record = records
            .iter_mut()
            .find(|record| record.id == feedback_id)
            .ok_or_else(|| Error::Task(format!("feedback record `{feedback_id}` was not found")))?;
        record.status = status;
        record.completed_at = Some(Utc::now());
        record.session_id = session_id.map(str::to_owned);
        record.error = error.map(ToString::to_string);
        self.store.save_feedback(id, &records).await
    }

    async fn clarify(&self, id: &PaperId, request: &ClarifyRequest) -> Result<Clarification> {
        let paper = self.store.load_extraction(id).await?.ok_or_else(|| {
            Error::InvalidRequest("analyze the paper before clarifying a passage".to_owned())
        })?;
        let analysis = self.store.load_analysis(id).await?.ok_or_else(|| {
            Error::InvalidRequest("analyze the paper before clarifying a passage".to_owned())
        })?;
        if let Some(section_id) = &request.section_id
            && !analysis
                .sections
                .iter()
                .any(|section| &section.id == section_id)
        {
            return Err(Error::InvalidRequest(format!(
                "unknown section: {section_id}"
            )));
        }
        self.analysis
            .clarify(
                request.provider,
                &paper,
                &analysis,
                &self.store.paper_dir(id),
                &request.selection,
                &request.question,
            )
            .await
    }
}

fn apply_experiment_arm_result(arm: &mut ExperimentArm, result: Result<LearningRamp>) {
    match result {
        Ok(output) => arm.output = Some(output),
        Err(error) => arm.error = Some(error.to_string()),
    }
}

fn validate_feedback_request(request: &FeedbackRequest) -> Result<String> {
    let feedback = request.feedback.trim();
    if feedback.is_empty() {
        return Err(Error::InvalidRequest(
            "feedback must say what should change".to_owned(),
        ));
    }
    if feedback.chars().count() > 8_000 {
        return Err(Error::InvalidRequest(
            "feedback is limited to 8,000 characters".to_owned(),
        ));
    }
    if request.provider == AnalysisProvider::Heuristic {
        return Err(Error::InvalidRequest(
            "feedback retries require the Codex or Claude reader".to_owned(),
        ));
    }
    Ok(feedback.to_owned())
}

fn find_experiment(catalog: &ExperimentCatalog, id: &str) -> Result<PromptExperiment> {
    catalog
        .experiments
        .iter()
        .find(|experiment| experiment.id == id.trim())
        .cloned()
        .ok_or_else(|| Error::InvalidRequest(format!("unknown experiment: {}", id.trim())))
}

fn redact_experiment(mut run: ExperimentRun, judgments: &[ExperimentJudgment]) -> ExperimentView {
    let judged = judgments.iter().any(|judgment| judgment.run_id == run.id);
    if !judged {
        for arm in &mut run.arms {
            arm.variant_id.clear();
            arm.variant_label.clear();
            arm.variant_instruction.clear();
        }
    }
    ExperimentView { run, judged }
}

fn validate_judgment(request: &ExperimentJudgmentRequest) -> Result<()> {
    let valid_choice = |value: &str| matches!(value, "A" | "B" | "tie");
    if !valid_choice(&request.overall)
        || !valid_choice(&request.early_traction)
        || !valid_choice(&request.rigor)
    {
        return Err(Error::InvalidRequest(
            "experiment preferences must be A, B, or tie".to_owned(),
        ));
    }
    if !(1..=5).contains(&request.confidence) {
        return Err(Error::InvalidRequest(
            "experiment confidence must be between 1 and 5".to_owned(),
        ));
    }
    let valid_tags = [
        "generic_restatement",
        "coverage_dump",
        "term_dump",
        "contextless_highlight",
        "surface_analogy",
        "missing_breakpoint",
        "generic_caveat",
        "unsupported_field_claim",
        "pseudo_actionability",
        "false_precision",
        "forward_reference",
        "budget_violation",
        "source_monoculture",
    ];
    if request.arm_scores.len() != 2
        || !["A", "B"].iter().all(|label| {
            request
                .arm_scores
                .iter()
                .filter(|score| score.blind_label == *label)
                .count()
                == 1
        })
    {
        return Err(Error::InvalidRequest(
            "experiment judgments require one absolute scorecard for each arm".to_owned(),
        ));
    }
    for score in &request.arm_scores {
        let common_scores = [
            score.paper_specificity_actionability,
            score.early_traction,
            score.fidelity_rigor,
            score.dependency_flow,
            score.economy,
            score.provenance_uncertainty,
        ];
        if common_scores.iter().any(|value| *value > 4)
            || score.target_dimension.is_some_and(|value| value > 4)
        {
            return Err(Error::InvalidRequest(
                "absolute experiment scores must be between 0 and 4".to_owned(),
            ));
        }
        if score.failure_tags.len() > valid_tags.len()
            || score
                .failure_tags
                .iter()
                .any(|tag| !valid_tags.contains(&tag.as_str()))
        {
            return Err(Error::InvalidRequest(
                "experiment scorecard contains an unknown failure tag".to_owned(),
            ));
        }
    }
    if request.note.chars().count() > 4_000 {
        return Err(Error::InvalidRequest(
            "experiment notes are limited to 4,000 characters".to_owned(),
        ));
    }
    Ok(())
}

fn parse_remote_pdf_url(value: &str) -> Result<Url> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidRequest("a PDF URL is required".to_owned()));
    }
    if trimmed.len() > MAX_REMOTE_URL_LENGTH {
        return Err(Error::InvalidRequest(format!(
            "remote PDF URLs are limited to {MAX_REMOTE_URL_LENGTH} bytes"
        )));
    }
    let mut url = Url::parse(trimmed)
        .map_err(|error| Error::InvalidRequest(format!("invalid PDF URL: {error}")))?;
    url.set_fragment(None);
    Ok(url)
}

async fn download_remote_pdf(original: &Url, path: &Path) -> Result<(Url, u64)> {
    let mut current = original.clone();
    for redirect_count in 0..=MAX_PUBLIC_REDIRECTS {
        let client = public_http_client(&current, REMOTE_REQUEST_TIMEOUT).await?;
        let mut response = client
            .get(current.clone())
            .header(ACCEPT, "application/pdf,application/octet-stream;q=0.8")
            .send()
            .await
            .map_err(|error| Error::RemoteImport(format!("request failed: {error}")))?;

        if response.status().is_redirection() {
            if redirect_count == MAX_PUBLIC_REDIRECTS {
                return Err(Error::RemoteImport(format!(
                    "remote server exceeded {MAX_PUBLIC_REDIRECTS} redirects"
                )));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| Error::RemoteImport("redirect omitted its destination".to_owned()))?
                .to_str()
                .map_err(|_| Error::RemoteImport("redirect destination was not text".to_owned()))?;
            current = current
                .join(location)
                .map_err(|error| Error::RemoteImport(format!("invalid redirect: {error}")))?;
            current.set_fragment(None);
            continue;
        }
        if !response.status().is_success() {
            return Err(Error::RemoteImport(format!(
                "remote server returned HTTP {}",
                response.status()
            )));
        }
        if response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > MAX_REMOTE_PDF_BYTES)
        {
            return Err(Error::InvalidRequest(format!(
                "remote PDFs are limited to {} MiB",
                MAX_REMOTE_PDF_BYTES / (1024 * 1024)
            )));
        }

        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .await
            .map_err(|error| Error::io(path, error))?;
        let mut byte_length = 0_u64;
        let mut prefix = Vec::with_capacity(1_024);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| Error::RemoteImport(format!("download failed: {error}")))?
        {
            byte_length = byte_length
                .checked_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| Error::InvalidRequest("remote PDF is too large".to_owned()))?;
            if byte_length > MAX_REMOTE_PDF_BYTES {
                return Err(Error::InvalidRequest(format!(
                    "remote PDFs are limited to {} MiB",
                    MAX_REMOTE_PDF_BYTES / (1024 * 1024)
                )));
            }
            if prefix.len() < 1_024 {
                let remaining = 1_024 - prefix.len();
                prefix.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|error| Error::io(path, error))?;
        }
        output
            .flush()
            .await
            .map_err(|error| Error::io(path, error))?;
        output
            .sync_all()
            .await
            .map_err(|error| Error::io(path, error))?;
        if !contains_pdf_header(&prefix) {
            return Err(Error::InvalidRequest(
                "the downloaded resource is not a PDF".to_owned(),
            ));
        }
        return Ok((current, byte_length));
    }
    Err(Error::RemoteImport(
        "remote redirect handling ended unexpectedly".to_owned(),
    ))
}

fn contains_pdf_header(prefix: &[u8]) -> bool {
    prefix.windows(5).any(|window| window == b"%PDF-")
}

fn remote_pdf_filename(url: &Url) -> String {
    let candidate = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .map_or_else(
            || "download.pdf".to_owned(),
            |segment| percent_decode_str(segment).decode_utf8_lossy().into_owned(),
        );
    sanitize_import_filename(&candidate)
}

fn sanitize_import_filename(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    sanitized = sanitized
        .trim_matches(|character| character == '.' || character == ' ')
        .to_owned();
    if sanitized.is_empty() {
        "download".clone_into(&mut sanitized);
    }
    if sanitized.chars().count() > 180 {
        sanitized = sanitized.chars().take(180).collect();
        sanitized = sanitized.trim_end_matches(['.', ' ']).to_owned();
    }
    if !sanitized.to_ascii_lowercase().ends_with(".pdf") {
        sanitized.push_str(".pdf");
    }
    sanitized
}

async fn available_import_filename(root: &Path, preferred: &str) -> Result<String> {
    if !tokio::fs::try_exists(root.join(preferred))
        .await
        .map_err(|error| Error::io(root, error))?
    {
        return Ok(preferred.to_owned());
    }
    let stem = Path::new(preferred)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("download");
    for suffix in 2..=10_000 {
        let candidate = format!("{stem} ({suffix}).pdf");
        if !tokio::fs::try_exists(root.join(&candidate))
            .await
            .map_err(|error| Error::io(root, error))?
        {
            return Ok(candidate);
        }
    }
    Err(Error::Task(
        "could not allocate a unique filename for the imported PDF".to_owned(),
    ))
}

async fn remove_temporary_import(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "could not clean temporary import");
        }
    }
}

impl std::fmt::Display for ProcessingStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Discovery => "discovery",
            Self::Extraction => "extraction",
            Self::Analysis => "analysis",
            Self::Persistence => "persistence",
        })
    }
}

pub fn build_router(mut state: AppState, frontend_directory: Option<&Path>) -> Router {
    state.frontend_root = frontend_directory.map(|path| Arc::new(path.to_owned()));
    Router::new()
        .route("/api/health", get(health))
        .route("/api/library", get(library))
        .route("/api/library/scan", post(scan_library))
        .route("/api/library/import", post(import_pdf))
        .route("/api/queue", get(processing_queue))
        .route("/api/experiments", get(experiment_catalog))
        .route("/api/papers/{id}", get(paper))
        .route("/api/papers/{id}/source", get(paper_source))
        .route("/api/papers/{id}/markdown", get(paper_markdown))
        .route("/api/papers/{id}/map", get(paper_map))
        .route("/api/papers/{id}/highlights", post(create_highlight))
        .route(
            "/api/papers/{id}/highlights/{highlight_id}",
            delete(delete_highlight),
        )
        .route("/api/papers/{id}/analyze", post(analyze_paper))
        .route("/api/papers/{id}/feedback", post(feedback_paper))
        .route("/api/papers/{id}/clarify", post(clarify_selection))
        .route(
            "/api/papers/{id}/experiments",
            get(paper_experiments).post(start_paper_experiment),
        )
        .route(
            "/api/papers/{id}/experiments/{run_id}",
            get(paper_experiment),
        )
        .route(
            "/api/papers/{id}/experiments/{run_id}/judgment",
            post(judge_paper_experiment),
        )
        .route("/", get(frontend_index))
        .route("/{*asset}", get(frontend_asset))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn library(State(state): State<AppState>) -> Json<LibraryResponse> {
    Json(state.library().await)
}

async fn scan_library(State(state): State<AppState>) -> Result<Json<LibraryResponse>> {
    state.refresh().await.map(Json)
}

async fn import_pdf(
    State(state): State<AppState>,
    Json(request): Json<ImportPdfRequest>,
) -> Result<(StatusCode, Json<ImportPdfResponse>)> {
    let imported = state.import_remote_pdf(&request.url).await?;
    Ok((StatusCode::CREATED, Json(imported)))
}

async fn processing_queue(State(state): State<AppState>) -> Result<Json<ProcessingQueue>> {
    state.processing_queue().await.map(Json)
}

async fn experiment_catalog(State(state): State<AppState>) -> Result<Json<ExperimentCatalog>> {
    state.experiment_catalog().map(Json)
}

async fn paper_experiments(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<ExperimentView>>> {
    let id = parse_id(&id)?;
    state.experiment_runs(&id).await.map(Json)
}

async fn start_paper_experiment(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<StartExperimentRequest>,
) -> Result<impl IntoResponse> {
    let id = parse_id(&id)?;
    let run = state.start_experiment(&id, request).await?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

async fn paper_experiment(
    State(state): State<AppState>,
    AxumPath((id, run_id)): AxumPath<(String, String)>,
) -> Result<Json<ExperimentView>> {
    let id = parse_id(&id)?;
    state.experiment_run(&id, &run_id).await.map(Json)
}

async fn judge_paper_experiment(
    State(state): State<AppState>,
    AxumPath((id, run_id)): AxumPath<(String, String)>,
    Json(request): Json<ExperimentJudgmentRequest>,
) -> Result<Json<ExperimentView>> {
    let id = parse_id(&id)?;
    state
        .judge_experiment(&id, &run_id, request)
        .await
        .map(Json)
}

async fn paper(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PaperView>> {
    let id = parse_id(&id)?;
    state.paper(&id).await.map(Json)
}

async fn analyze_paper(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AnalyzeRequest>,
) -> Result<impl IntoResponse> {
    let id = parse_id(&id)?;
    let overview = state
        .queue_analysis(id, request.provider, request.force)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(overview)))
}

async fn feedback_paper(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<FeedbackRequest>,
) -> Result<impl IntoResponse> {
    let id = parse_id(&id)?;
    let job = state.queue_feedback(id, request).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn clarify_selection(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<ClarifyRequest>,
) -> Result<Json<Clarification>> {
    let id = parse_id(&id)?;
    state.clarify(&id, &request).await.map(Json)
}

async fn paper_source(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response> {
    let id = parse_id(&id)?;
    let source_path = state.source_path(&id).await?;
    let file = File::open(&source_path)
        .await
        .map_err(|error| Error::io(&source_path, error))?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    Ok((headers, Body::from_stream(ReaderStream::new(file))).into_response())
}

async fn paper_markdown(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response> {
    let id = parse_id(&id)?;
    let markdown = state.markdown(&id).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    Ok((headers, markdown).into_response())
}

async fn paper_map(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PaperMap>> {
    let id = parse_id(&id)?;
    state.paper_map(&id).await.map(Json)
}

async fn create_highlight(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<CreateHighlightRequest>,
) -> Result<(StatusCode, Json<Highlight>)> {
    let id = parse_id(&id)?;
    let highlight = state.create_highlight(&id, &request).await?;
    Ok((StatusCode::CREATED, Json(highlight)))
}

async fn delete_highlight(
    State(state): State<AppState>,
    AxumPath((id, highlight_id)): AxumPath<(String, String)>,
) -> Result<StatusCode> {
    let id = parse_id(&id)?;
    state.delete_highlight(&id, &highlight_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_id(value: &str) -> Result<PaperId> {
    PaperId::from_str(value).map_err(Error::InvalidRequest)
}

async fn frontend_index(State(state): State<AppState>) -> Result<Response> {
    serve_frontend_asset(&state, "index.html").await
}

async fn frontend_asset(
    State(state): State<AppState>,
    AxumPath(asset): AxumPath<String>,
) -> Result<Response> {
    serve_frontend_asset(&state, &asset).await
}

async fn serve_frontend_asset(state: &AppState, asset: &str) -> Result<Response> {
    let Some(root) = &state.frontend_root else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let requested = Path::new(asset);
    if requested.components().any(|component| {
        !matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return Err(Error::InvalidRequest("invalid asset path".to_owned()));
    }
    let candidate = root.join(requested);
    let path = if candidate.is_file() {
        candidate
    } else {
        root.join("index.html")
    };
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| Error::io(&path, error))?;
    let content_type = content_type_for(&path);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=300")
        .body(Body::from(bytes))
        .map_err(|error| Error::Task(format!("could not build asset response: {error}")))
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        _ => "text/html; charset=utf-8",
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use super::*;
    use crate::domain::{
        CreateHighlightRequest, DocumentLayout, ExtractedPage, ExtractedPaper, HighlightKind,
        HighlightOrigin, LayoutPage, LayoutSentence, LayoutToken, PaperMetadata, TextRect,
    };

    #[tokio::test]
    async fn lists_discovered_papers_without_extraction() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let fixture = library.path().join("Ada - 1843 - Notes.pdf");
        tokio::fs::write(&fixture, b"not parsed during discovery")
            .await
            .map_err(|error| Error::io(&fixture, error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let app = build_router(state, None);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/library")
                    .body(Body::empty())
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|error| Error::Task(error.to_string()))?
            .to_bytes();
        let library: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(library["papers"].as_array().map(Vec::len), Some(1));
        Ok(())
    }

    #[tokio::test]
    async fn rejects_private_remote_imports_before_connecting() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let response = build_router(state, None)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/library/import")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"url":"http://127.0.0.1/paper.pdf"}"#))
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            tokio::fs::read_dir(library.path())
                .await
                .map_err(|error| Error::io(library.path(), error))?
                .next_entry()
                .await
                .map_err(|error| Error::io(library.path(), error))?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn commits_a_complete_remote_pdf_and_origin_record() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let temporary_path = library.path().join(".import.tmp");
        let bytes = b"%PDF-1.7\nfixture\n%%EOF\n";
        tokio::fs::write(&temporary_path, bytes)
            .await
            .map_err(|error| Error::io(&temporary_path, error))?;
        let original = Url::parse("https://example.com/redirect")
            .map_err(|error| Error::Task(error.to_string()))?;
        let final_url = Url::parse("https://cdn.example.com/A%20Paper.pdf")
            .map_err(|error| Error::Task(error.to_string()))?;

        let imported = state
            .finish_remote_import(
                &original,
                &final_url,
                u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                &temporary_path,
            )
            .await?;

        assert_eq!(imported.paper.relative_path, "A Paper.pdf");
        assert_eq!(imported.source.original_url, original.to_string());
        assert_eq!(imported.source.final_url, final_url.to_string());
        assert!(library.path().join("A Paper.pdf").is_file());
        assert!(!temporary_path.exists());
        let origin = data
            .path()
            .join("papers")
            .join(imported.paper.id.as_str())
            .join("origin.json");
        let persisted = tokio::fs::read_to_string(&origin)
            .await
            .map_err(|error| Error::io(&origin, error))?;
        assert!(persisted.contains(original.as_str()));
        assert!(persisted.contains(final_url.as_str()));
        Ok(())
    }

    #[test]
    fn validates_pdf_headers_and_portable_import_names() {
        assert!(contains_pdf_header(b"preamble\n%PDF-1.7\n"));
        assert!(!contains_pdf_header(b"<!doctype html>"));
        assert_eq!(sanitize_import_filename("../paper:name"), "_paper_name.pdf");
        assert_eq!(sanitize_import_filename("article.PDF"), "article.PDF");

        let encoded = Url::parse("https://example.com/A%20Readable%20Paper.pdf?download=1")
            .expect("test URL should parse");
        assert_eq!(remote_pdf_filename(&encoded), "A Readable Paper.pdf");
    }

    #[tokio::test]
    async fn serves_cached_extraction_as_markdown() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let filename = "Ada - 1843 - Notes.pdf";
        let fixture = library.path().join(filename);
        tokio::fs::write(&fixture, b"discovered without parsing")
            .await
            .map_err(|error| Error::io(&fixture, error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let id = PaperId::from_relative_path(Path::new(filename));
        state
            .store
            .save_extraction(
                &id,
                &ExtractedPaper {
                    metadata: PaperMetadata {
                        title: "Notes".to_owned(),
                        authors: vec!["Ada".to_owned()],
                        year: Some(1843),
                        page_count: Some(1),
                        subject: None,
                    },
                    pages: vec![ExtractedPage {
                        number: 1,
                        text: "ABSTRACT\nAn analytical engine follows notation.".to_owned(),
                    }],
                    layout: DocumentLayout::default(),
                },
            )
            .await?;
        let response = build_router(state, None)
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/papers/{id}/markdown"))
                    .body(Body::empty())
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/markdown; charset=utf-8"))
        );
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|error| Error::Task(error.to_string()))?
            .to_bytes();
        let markdown = String::from_utf8_lossy(&body);
        assert!(markdown.contains("# Notes"));
        assert!(markdown.contains("### Abstract"));
        Ok(())
    }

    #[tokio::test]
    async fn completed_analysis_exposes_a_finished_plaintext_tasklist() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let filename = "Ada - 1843 - Notes.pdf";
        let fixture = library.path().join(filename);
        tokio::fs::write(&fixture, b"discovered without parsing")
            .await
            .map_err(|error| Error::io(&fixture, error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let id = PaperId::from_relative_path(Path::new(filename));
        state
            .store
            .save_extraction(
                &id,
                &ExtractedPaper {
                    metadata: PaperMetadata {
                        title: "Notes".to_owned(),
                        authors: vec!["Ada".to_owned()],
                        year: Some(1843),
                        page_count: Some(1),
                        subject: None,
                    },
                    pages: vec![ExtractedPage {
                        number: 1,
                        text: "ABSTRACT\nAn analytical engine follows notation. The notation makes the operation legible."
                            .to_owned(),
                    }],
                    layout: DocumentLayout::default(),
                },
            )
            .await?;
        let view = state
            .analyze_now(&id, AnalysisProvider::Heuristic, true)
            .await?;
        assert!(view.analysis.is_some());
        let queue = state.processing_queue().await?;
        assert_eq!(queue.jobs.len(), 1);
        assert_eq!(queue.jobs[0].progress, 100);
        assert!(matches!(
            queue.jobs[0].status,
            crate::domain::AnalysisJobStatus::Completed
        ));
        let tasklist = state
            .store
            .load_tasklist(&id)
            .await?
            .ok_or_else(|| Error::Task("tasklist was not written".to_owned()))?;
        assert!(
            tasklist
                .lines()
                .filter(|line| line.starts_with("- [x]"))
                .count()
                >= 5
        );
        Ok(())
    }

    #[tokio::test]
    async fn creates_and_deletes_a_sentence_anchored_reader_highlight() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let filename = "Ada - 1843 - Notes.pdf";
        let fixture = library.path().join(filename);
        tokio::fs::write(&fixture, b"discovered without parsing")
            .await
            .map_err(|error| Error::io(&fixture, error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let id = PaperId::from_relative_path(Path::new(filename));
        let rect = TextRect {
            x_min: 10.0,
            y_min: 20.0,
            x_max: 90.0,
            y_max: 30.0,
        };
        let sentence_id = "p0001-s00001".to_owned();
        state
            .store
            .save_extraction(
                &id,
                &ExtractedPaper {
                    metadata: PaperMetadata {
                        title: "Notes".to_owned(),
                        ..PaperMetadata::default()
                    },
                    pages: vec![ExtractedPage {
                        number: 1,
                        text: "A grounded sentence.".to_owned(),
                    }],
                    layout: DocumentLayout {
                        schema_version: 1,
                        pages: vec![LayoutPage {
                            number: 1,
                            width: 200.0,
                            height: 300.0,
                            tokens: vec![LayoutToken {
                                index: 0,
                                text: "A grounded sentence.".to_owned(),
                                line: 0,
                                rects: vec![rect],
                            }],
                            sentences: vec![LayoutSentence {
                                id: sentence_id.clone(),
                                page: 1,
                                start_token: 0,
                                end_token: 0,
                                text: "A grounded sentence.".to_owned(),
                                rects: vec![rect],
                            }],
                        }],
                    },
                },
            )
            .await?;

        let highlight = state
            .create_highlight(
                &id,
                &CreateHighlightRequest {
                    start_sentence_id: sentence_id,
                    end_sentence_id: None,
                    kind: HighlightKind::Note,
                    note: "Reader note".to_owned(),
                },
            )
            .await?;
        assert!(matches!(highlight.origin, HighlightOrigin::User));
        assert_eq!(state.store.load_highlights(&id).await?.len(), 1);
        let jsonl = tokio::fs::read_to_string(state.store.paper_dir(&id).join("highlights.jsonl"))
            .await
            .map_err(|error| Error::io("highlights.jsonl", error))?;
        assert_eq!(jsonl.lines().count(), 1);
        assert!(jsonl.contains(r#""type":"user""#));
        let markdown = tokio::fs::read_to_string(state.store.paper_dir(&id).join("highlights.md"))
            .await
            .map_err(|error| Error::io("highlights.md", error))?;
        assert!(markdown.contains("Owner: Reader"));
        state.delete_highlight(&id, &highlight.id).await?;
        assert!(state.store.load_highlights(&id).await?.is_empty());
        Ok(())
    }

    #[test]
    fn feedback_requires_a_model_reader_and_bounded_text() {
        let empty = FeedbackRequest {
            feedback: "   ".to_owned(),
            provider: AnalysisProvider::Codex,
        };
        assert!(validate_feedback_request(&empty).is_err());
        let offline = FeedbackRequest {
            feedback: "Explain the result more plainly".to_owned(),
            provider: AnalysisProvider::Heuristic,
        };
        assert!(validate_feedback_request(&offline).is_err());
        let valid = FeedbackRequest {
            feedback: "  Explain the result more plainly.  ".to_owned(),
            provider: AnalysisProvider::Claude,
        };
        assert!(matches!(
            validate_feedback_request(&valid).as_deref(),
            Ok("Explain the result more plainly.")
        ));
    }

    #[test]
    fn experiment_catalog_is_single_factor_and_two_armed() -> Result<()> {
        let catalog: ExperimentCatalog = serde_json::from_str(EXPERIMENT_CATALOG)?;
        assert_eq!(catalog.experiments.len(), 5);
        assert!(catalog.experiments.iter().all(|experiment| {
            !experiment.question.trim().is_empty() && experiment.variants.len() == 2
        }));
        let ids = catalog
            .experiments
            .iter()
            .map(|experiment| experiment.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), catalog.experiments.len());
        Ok(())
    }

    #[test]
    fn prompt_identities_stay_blind_until_judgment() {
        let run = ExperimentRun {
            schema_version: 1,
            id: "run-1".to_owned(),
            paper_id: PaperId::from_relative_path(Path::new("paper.pdf")),
            paper_title: "Paper".to_owned(),
            experiment_id: "conceptual-bridge".to_owned(),
            experiment_name: "Conceptual bridge".to_owned(),
            question: "Which is smoother?".to_owned(),
            reader_baseline: vec!["mathematics".to_owned()],
            provider: AnalysisProvider::Codex,
            status: ExperimentStatus::Completed,
            model: "fixed-model".to_owned(),
            reasoning_effort: "medium".to_owned(),
            created_at: Utc::now(),
            completed_at: Some(Utc::now()),
            arms: vec![ExperimentArm {
                blind_label: "A".to_owned(),
                variant_id: "bounded_bridge".to_owned(),
                variant_label: "Bounded bridge".to_owned(),
                variant_instruction: "Use one bounded bridge.".to_owned(),
                output: None,
                error: None,
            }],
        };
        let blind = redact_experiment(run.clone(), &[]);
        assert!(!blind.judged);
        assert!(blind.run.arms[0].variant_id.is_empty());
        assert!(blind.run.arms[0].variant_instruction.is_empty());

        let revealed = redact_experiment(
            run,
            &[ExperimentJudgment {
                run_id: "run-1".to_owned(),
                experiment_id: "conceptual-bridge".to_owned(),
                overall: "A".to_owned(),
                early_traction: "A".to_owned(),
                rigor: "tie".to_owned(),
                confidence: 4,
                arm_scores: Vec::new(),
                note: String::new(),
                submitted_at: Utc::now(),
            }],
        );
        assert!(revealed.judged);
        assert_eq!(revealed.run.arms[0].variant_id, "bounded_bridge");
        assert!(!revealed.run.arms[0].variant_instruction.is_empty());
    }

    #[test]
    fn experiment_judgments_require_complete_bounded_choices() {
        let valid = ExperimentJudgmentRequest {
            overall: "A".to_owned(),
            early_traction: "tie".to_owned(),
            rigor: "B".to_owned(),
            confidence: 4,
            arm_scores: vec![
                crate::domain::ExperimentArmScore {
                    blind_label: "A".to_owned(),
                    paper_specificity_actionability: 3,
                    early_traction: 3,
                    fidelity_rigor: 4,
                    dependency_flow: 2,
                    economy: 3,
                    provenance_uncertainty: 4,
                    target_dimension: Some(3),
                    hard_reject: false,
                    failure_tags: Vec::new(),
                },
                crate::domain::ExperimentArmScore {
                    blind_label: "B".to_owned(),
                    paper_specificity_actionability: 2,
                    early_traction: 2,
                    fidelity_rigor: 3,
                    dependency_flow: 2,
                    economy: 2,
                    provenance_uncertainty: 3,
                    target_dimension: Some(2),
                    hard_reject: false,
                    failure_tags: vec!["generic_restatement".to_owned()],
                },
            ],
            note: "A gets to the mechanism sooner.".to_owned(),
        };
        assert!(validate_judgment(&valid).is_ok());
        let invalid = ExperimentJudgmentRequest {
            confidence: 9,
            overall: "maybe".to_owned(),
            ..valid
        };
        assert!(validate_judgment(&invalid).is_err());
    }
}
