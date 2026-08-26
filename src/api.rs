use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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
use serde::Serialize;
use tokio::{
    fs::File,
    sync::{Mutex, RwLock},
};
use tokio_util::io::ReaderStream;

use crate::{
    Result,
    analysis::{AnalysisService, validate_citations},
    domain::{
        AgentSession, AnalysisJob, AnalysisJobKind, AnalysisProvider, AnalyzeRequest,
        CitationStatus, Clarification, ClarifyRequest, CreateHighlightRequest, ExtractedPaper,
        FeedbackRecord, FeedbackRequest, FeedbackStatus, Highlight, HighlightOrigin, PaperId,
        PaperMap, PaperOverview, PaperView, ProcessingQueue, ProcessingStage, ProcessingStatus,
    },
    error::Error,
    extract::PdfExtractor,
    jobs::JobTracker,
    library::LibraryCatalog,
    store::ArtifactStore,
};

static USER_HIGHLIGHT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FEEDBACK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct AppState {
    catalog: Arc<RwLock<LibraryCatalog>>,
    library_root: Arc<PathBuf>,
    store: ArtifactStore,
    extractor: PdfExtractor,
    analysis: AnalysisService,
    jobs: JobTracker,
    highlight_write: Arc<Mutex<()>>,
    frontend_root: Option<Arc<PathBuf>>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
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

        if let Err((stage, error)) = self.run_analysis(id, provider).await {
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
            if let Err((stage, error)) = state.run_analysis(&id, provider).await {
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
            .transition(id, ProcessingStage::Analysis, "read")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        let outcome = self
            .analysis
            .analyze(provider, &paper, &self.store.paper_dir(id))
            .await
            .map_err(|error| (ProcessingStage::Analysis, error))?;
        for task in ["read", "structure", "evidence", "explain"] {
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
            let needs_validation = analysis.schema_version < 2
                || analysis.sections.iter().any(|section| {
                    section
                        .key_quotes
                        .iter()
                        .any(|quote| quote.validation == CitationStatus::Unverified)
                });
            if needs_validation {
                let highlight_guard = self.highlight_write.lock().await;
                validate_citations(&mut analysis, &paper.layout);
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
        .route("/api/queue", get(processing_queue))
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

async fn processing_queue(State(state): State<AppState>) -> Result<Json<ProcessingQueue>> {
    state.processing_queue().await.map(Json)
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
}
