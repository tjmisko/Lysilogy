use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

mod acquisition;
mod citation_graph;
mod curation;
mod expansion;
mod history;
mod labels;
mod literature;
mod neighbors;
mod notes;
mod provenance;
mod ramps;
mod reader_tools;
mod source_index;
mod vimrc;

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path as AxumPath, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post},
};
use chrono::Utc;
use lysilogy_core::{Work, WorkId};
use percent_encoding::percent_decode_str;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    sync::{Mutex, RwLock},
};
use tokio_util::io::ReaderStream;

pub(crate) use self::acquisition::states_for;
pub use self::acquisition::{AcquisitionBrief, AcquisitionStates};
pub(crate) use self::literature::local_papers_for;
pub use self::literature::{LocalPaper, LocalPapers};
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
    graph::GraphState,
    jobs::JobTracker,
    library::{ExtraLibrary, LibraryCatalog},
    remote::{PublicResource, download_public_resource},
    store::ArtifactStore,
};

static USER_HIGHLIGHT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FEEDBACK_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static EXPERIMENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const EXPERIMENT_CATALOG: &str = include_str!("../experiments/catalog.json");
const MAX_REMOTE_URL_LENGTH: usize = 4_096;

#[derive(Clone, Debug)]
pub struct AppState {
    pub(crate) catalog: Arc<RwLock<LibraryCatalog>>,
    pub(crate) library_root: Arc<PathBuf>,
    extra_libraries: Arc<RwLock<Vec<ExtraLibrary>>>,
    pub(crate) store: ArtifactStore,
    pub(crate) extractor: PdfExtractor,
    analysis: AnalysisService,
    citation_http: crate::citation_graph::GraphHttp,
    pub(crate) graph: Option<GraphState>,
    pub(crate) literature: crate::literature::LiteratureClient,
    literature_jobs: literature::LiteratureJobs,
    acquisition_jobs: acquisition::AcquisitionJobs,
    expansion_jobs: expansion::ExpansionJobs,
    pub(crate) acquisition_throttle: Arc<lysilogy_ingest::acquire::HostThrottle>,
    pub(crate) acquisition_fetcher: crate::acquisition::SharedFetcher,
    jobs: JobTracker,
    notes: crate::notes::NotesStore,
    vimrc: Arc<PathBuf>,
    highlight_write: Arc<Mutex<()>>,
    import_write: Arc<Mutex<()>>,
    experiment_write: Arc<Mutex<()>>,
    tools_write: Arc<Mutex<()>>,
    tools_extract: Arc<Mutex<()>>,
    reading_indexes: source_index::IndexJobs,
    frontend_root: Option<Arc<PathBuf>>,
    /// The model pass that fills missing authors and years whenever local
    /// evidence runs. `None` skips it.
    pub(crate) field_extractor: Option<crate::field_extraction::SidecarFieldExtractor>,
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

#[derive(Debug, Deserialize)]
struct UploadPdfQuery {
    filename: String,
}

#[derive(Debug, Serialize)]
struct UploadPdfResponse {
    paper: PaperOverview,
    library: LibraryResponse,
}

const MAX_UPLOAD_BYTES: usize = 200 * 1024 * 1024;

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
    pub directories: Vec<LibraryDirectory>,
}

#[derive(Debug, Serialize)]
pub struct LibraryDirectory {
    /// `primary` for the root the server was started with, else `lib-<hex>`.
    pub id: String,
    pub name: String,
    pub path: String,
    pub primary: bool,
    /// False while the folder is missing or unreadable; its papers are hidden.
    pub available: bool,
    pub papers: usize,
}

#[derive(Debug, Deserialize)]
struct AddDirectoryRequest {
    path: String,
}

#[derive(Debug, Serialize)]
struct CitationsResponse {
    work: Work,
    outgoing: Vec<Work>,
    incoming: Vec<Work>,
    edges: Vec<labels::GraphEdge>,
    local_papers: literature::LocalPapers,
    acquisitions: AcquisitionStates,
    marks: curation::Marks,
    work_tags: curation::WorkTagIds,
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
        let extra_libraries = store.load_extra_libraries().await?;
        let catalog = LibraryCatalog::scan_all(&library_root, &extra_libraries, &store).await?;
        let jobs = JobTracker::load(store.clone()).await?;
        Ok(Self {
            catalog: Arc::new(RwLock::new(catalog)),
            library_root: Arc::new(library_root),
            extra_libraries: Arc::new(RwLock::new(extra_libraries)),
            store,
            extractor,
            analysis,
            citation_http: crate::citation_graph::GraphHttp::from_environment()?,
            jobs,
            notes: crate::notes::NotesStore::new(PathBuf::from("Notes")),
            vimrc: Arc::new(PathBuf::from(".vimrc")),
            highlight_write: Arc::new(Mutex::new(())),
            import_write: Arc::new(Mutex::new(())),
            experiment_write: Arc::new(Mutex::new(())),
            tools_write: Arc::new(Mutex::new(())),
            tools_extract: Arc::new(Mutex::new(())),
            reading_indexes: source_index::IndexJobs::default(),
            graph: None,
            literature: crate::literature::LiteratureClient::from_environment()?,
            literature_jobs: literature::LiteratureJobs::default(),
            acquisition_jobs: acquisition::AcquisitionJobs::default(),
            expansion_jobs: expansion::ExpansionJobs::default(),
            acquisition_throttle: Arc::new(lysilogy_ingest::acquire::HostThrottle::polite()),
            acquisition_fetcher: crate::acquisition::SharedFetcher::default(),
            frontend_root: None,
            field_extractor: None,
        })
    }

    /// Replace the `OpenAlex` client used by the literature flow (for tests).
    #[must_use]
    pub fn with_literature_client(mut self, client: crate::literature::LiteratureClient) -> Self {
        self.literature = client;
        self
    }

    /// Replace the acquisition fetcher (for tests).
    #[must_use]
    pub fn with_acquisition_fetcher(
        mut self,
        fetcher: Arc<dyn lysilogy_ingest::acquire::PdfFetcher>,
    ) -> Self {
        self.acquisition_fetcher = crate::acquisition::SharedFetcher(fetcher);
        self
    }

    /// Replace the acquisition host throttle (for tests).
    #[must_use]
    pub fn with_acquisition_throttle(
        mut self,
        throttle: lysilogy_ingest::acquire::HostThrottle,
    ) -> Self {
        self.acquisition_throttle = Arc::new(throttle);
        self
    }

    /// Run the field pass with `extractor` whenever local evidence runs.
    #[must_use]
    pub fn with_field_extractor(
        mut self,
        extractor: crate::field_extraction::SidecarFieldExtractor,
    ) -> Self {
        self.field_extractor = Some(extractor);
        self
    }

    #[must_use]
    pub fn with_graph(mut self, graph: GraphState) -> Self {
        self.graph = Some(graph);
        self
    }

    /// Connect the graph from `LYSILOGY_DATABASE_URL`, if set. A failure leaves
    /// the reader running without a graph.
    pub async fn with_graph_from_environment(mut self) -> Self {
        if let Ok(url) = std::env::var("LYSILOGY_DATABASE_URL")
            && !url.trim().is_empty()
        {
            match GraphState::connect(&url).await {
                Ok(graph) => self.graph = Some(graph),
                Err(error) => {
                    tracing::warn!(%error, "graph database unavailable; reader continues without it");
                }
            }
        }
        self
    }

    #[must_use]
    pub fn with_notes_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.notes = crate::notes::NotesStore::new(root.into());
        self
    }

    /// The configured Vimrc is read lazily, when a notes editor requests it.
    #[must_use]
    pub fn with_vimrc(mut self, path: impl Into<PathBuf>) -> Self {
        self.vimrc = Arc::new(path.into());
        self
    }

    pub fn with_notes_template(
        mut self,
        template: crate::notes::NoteTemplateConfig,
    ) -> Result<Self> {
        self.notes = self
            .notes
            .with_template(template)
            .map_err(|error| Error::InvalidRequest(error.to_string()))?;
        Ok(self)
    }

    pub async fn library(&self) -> LibraryResponse {
        let catalog = self.catalog.read().await;
        let extras = self.extra_libraries.read().await;
        let papers = catalog.overviews();
        let count_in = |prefix: &str| {
            papers
                .iter()
                .filter(|paper| {
                    paper
                        .relative_path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .count()
        };
        let extra_papers = extras
            .iter()
            .map(|extra| count_in(&extra.id))
            .collect::<Vec<_>>();
        let mut directories = vec![LibraryDirectory {
            id: "primary".to_owned(),
            name: directory_name(catalog.root()),
            path: catalog.root().display().to_string(),
            primary: true,
            available: true,
            papers: papers.len() - extra_papers.iter().sum::<usize>(),
        }];
        directories.extend(extras.iter().zip(extra_papers).map(|(extra, count)| {
            LibraryDirectory {
                id: extra.id.clone(),
                name: directory_name(&extra.path),
                path: extra.path.display().to_string(),
                primary: false,
                available: extra.path.is_dir(),
                papers: count,
            }
        }));
        LibraryResponse {
            name: directory_name(catalog.root()),
            papers,
            directories,
        }
    }

    pub async fn refresh(&self) -> Result<LibraryResponse> {
        let extras = self.extra_libraries.read().await.clone();
        let replacement =
            LibraryCatalog::scan_all(self.library_root.as_ref(), &extras, &self.store).await?;
        self.catalog.write().await.replace_with(replacement);
        Ok(self.library().await)
    }

    /// Adds `path` as another library directory. Overlapping directories are
    /// refused because they would list the same PDF twice under different IDs.
    pub async fn add_library_directory(&self, path: &str) -> Result<LibraryResponse> {
        let trimmed = path.trim();
        if trimmed.is_empty() || !Path::new(trimmed).is_absolute() {
            return Err(Error::InvalidRequest(
                "library folder must be an absolute path".to_owned(),
            ));
        }
        let canonical = tokio::fs::canonicalize(trimmed)
            .await
            .map_err(|_| Error::InvalidRequest(format!("not a readable directory: {trimmed}")))?;
        if !canonical.is_dir() {
            return Err(Error::InvalidRequest(format!(
                "not a directory: {}",
                canonical.display()
            )));
        }
        let _guard = self.import_write.lock().await;
        let primary = tokio::fs::canonicalize(self.library_root.as_ref())
            .await
            .map_err(|error| Error::io(self.library_root.as_ref(), error))?;
        let mut libraries = self.extra_libraries.read().await.clone();
        let overlaps = |other: &Path| canonical.starts_with(other) || other.starts_with(&canonical);
        if overlaps(&primary) || libraries.iter().any(|extra| overlaps(&extra.path)) {
            return Err(Error::InvalidRequest(format!(
                "{} overlaps a folder already in the library",
                canonical.display()
            )));
        }
        libraries.push(ExtraLibrary::for_path(canonical));
        self.store.save_extra_libraries(&libraries).await?;
        *self.extra_libraries.write().await = libraries;
        self.refresh().await
    }

    /// Stops listing a directory. Files and generated artifacts stay on disk, so
    /// adding the folder again restores each paper's analysis.
    pub async fn remove_library_directory(&self, id: &str) -> Result<LibraryResponse> {
        let _guard = self.import_write.lock().await;
        let mut libraries = self.extra_libraries.read().await.clone();
        let before = libraries.len();
        libraries.retain(|extra| extra.id != id);
        if libraries.len() == before {
            return Err(Error::InvalidRequest(format!(
                "no such library folder: {id}"
            )));
        }
        self.store.save_extra_libraries(&libraries).await?;
        *self.extra_libraries.write().await = libraries;
        self.refresh().await
    }

    async fn import_remote_pdf(&self, value: &str) -> Result<ImportPdfResponse> {
        let original = parse_remote_pdf_url(value)?;
        let import_guard = self.import_write.lock().await;
        let download = download_public_resource(&original, &self.library_root, false)
            .await
            .map_err(crate::remote::DownloadError::into_error);
        let (final_url, byte_length, temporary_path) = match download {
            Ok(PublicResource::Pdf {
                final_url,
                temp_path,
                bytes,
                ..
            }) => match Url::parse(&final_url) {
                Ok(parsed) => (parsed, bytes, temp_path),
                Err(error) => {
                    remove_temporary_import(&temp_path).await;
                    return Err(Error::RemoteImport(format!("invalid final URL: {error}")));
                }
            },
            Ok(PublicResource::Html { .. }) => {
                return Err(Error::InvalidRequest(
                    "the downloaded resource is not a PDF".to_owned(),
                ));
            }
            Err(error) => return Err(error),
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

    async fn upload_pdf(&self, preferred: &str, bytes: &[u8]) -> Result<UploadPdfResponse> {
        if !bytes.starts_with(b"%PDF-") {
            return Err(Error::InvalidRequest(
                "the uploaded file is not a PDF".to_owned(),
            ));
        }
        let _guard = self.import_write.lock().await;
        let filename = available_import_filename(
            self.library_root.as_ref(),
            &sanitize_import_filename(preferred),
        )
        .await?;
        let final_path = self.library_root.join(&filename);
        let temporary_path = self.library_root.join(format!(".{filename}.upload"));
        tokio::fs::write(&temporary_path, bytes)
            .await
            .map_err(|error| Error::io(&temporary_path, error))?;
        if let Err(error) = tokio::fs::rename(&temporary_path, &final_path).await {
            remove_temporary_import(&temporary_path).await;
            return Err(Error::io(&final_path, error));
        }
        let id = PaperId::from_relative_path(Path::new(&filename));
        let library = self.refresh().await?;
        let paper = library
            .papers
            .iter()
            .find(|paper| paper.id == id)
            .cloned()
            .ok_or_else(|| {
                Error::Task("uploaded PDF was not discovered after rescan".to_owned())
            })?;
        Ok(UploadPdfResponse { paper, library })
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
        let mut overview = self
            .catalog
            .read()
            .await
            .get(id)
            .map(|entry| entry.overview.clone())
            .ok_or_else(|| Error::PaperNotFound(id.to_string()))?;
        let analysis = self.store.load_analysis(id).await?;
        let metadata = if let Some(metadata) = self.store.load_extraction_metadata(id).await? {
            Some(metadata)
        } else if analysis.is_some() {
            let _guard = self.tools_extract.lock().await;
            Some(
                self.load_or_extract(id)
                    .await
                    .map_err(|(_, error)| error)?
                    .metadata,
            )
        } else {
            None
        };
        if let Some(metadata) = metadata {
            overview.metadata = metadata.clone();
            if let Some(entry) = self.catalog.write().await.get_mut(id) {
                entry.overview.metadata = metadata;
            }
        }
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

    #[allow(clippy::too_many_lines)] // Keep reservation, component work, and status restoration together.
    pub async fn refresh_component(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
        _force: bool,
        component: crate::domain::AnalysisComponent,
    ) -> Result<PaperView> {
        use crate::domain::AnalysisComponent;
        let (kind, task) = match component {
            AnalysisComponent::Abstract => (AnalysisJobKind::AbstractRefresh, "abstract"),
            AnalysisComponent::Context => (AnalysisJobKind::ContextRefresh, "context"),
            AnalysisComponent::Structure => (AnalysisJobKind::StructureRefresh, "structure"),
        };
        let previous = {
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
            let previous = entry.overview.clone();
            entry.overview.status = ProcessingStatus::Analyzing { provider };
            drop(catalog);
            previous
        };
        let outcome = async {
            self.jobs
                .begin(
                    id.clone(),
                    previous.metadata.title.clone(),
                    provider,
                    kind,
                    None,
                )
                .await?;
            self.jobs
                .transition(id, ProcessingStage::Extraction, "extract")
                .await?;
            let paper = self.load_or_extract(id).await.map_err(|(_, error)| error)?;
            self.jobs.task_completed(id, "extract").await?;
            self.jobs
                .transition(id, ProcessingStage::Analysis, task)
                .await?;
            let mut analysis = self.store.load_analysis(id).await?;
            match component {
                AnalysisComponent::Structure | AnalysisComponent::Abstract => {
                    let analysis = analysis.as_mut().ok_or_else(|| {
                        Error::InvalidRequest("Analyze this paper before refreshing it".to_owned())
                    })?;
                    let pdf = self.source_path(id).await?;
                    let (progress, forwarder) = self.jobs.progress_reporter(id, task);
                    let refreshed = self
                        .analysis
                        .analyze_core_artifacts(
                            provider,
                            &paper,
                            &pdf,
                            &self.store.paper_dir(id),
                            Some(progress),
                        )
                        .await;
                    forwarder.stop().await;
                    let refreshed = refreshed?;
                    match component {
                        AnalysisComponent::Structure => {
                            analysis.sections.clone_from(&refreshed.analysis.sections);
                            analysis.generated_at = refreshed.analysis.generated_at;
                        }
                        AnalysisComponent::Abstract => {
                            analysis
                                .author_abstract
                                .clone_from(&refreshed.analysis.author_abstract);
                            analysis.abstract_extraction = refreshed.analysis.abstract_extraction;
                        }
                        AnalysisComponent::Context => unreachable!(),
                    }
                }
                AnalysisComponent::Context => {
                    return Err(Error::InvalidRequest(
                        "Context research is disabled until its node is rebuilt without web"
                            .to_owned(),
                    ));
                }
            }
            self.jobs.task_completed(id, task).await?;
            self.jobs
                .transition(id, ProcessingStage::Persistence, "persist")
                .await?;
            let _guard = self.highlight_write.lock().await;
            if let Some(analysis) = analysis {
                if component == AnalysisComponent::Structure {
                    self.store.save_analysis(id, &analysis).await?;
                } else {
                    self.store.save_analysis_projection(id, &analysis).await?;
                }
            }
            self.jobs.task_completed(id, "persist").await?;
            self.jobs.complete(id, false).await?;
            Ok::<(), Error>(())
        }
        .await;
        if let Some(entry) = self.catalog.write().await.get_mut(id) {
            entry.overview.status = previous.status;
        }
        if let Err(error) = outcome {
            self.jobs
                .fail(id, ProcessingStage::Analysis, &error)
                .await?;
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
            .transition(id, ProcessingStage::Analysis, "core")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        let (progress, forwarder) = self.jobs.progress_reporter(id, "core");
        let outcome = self
            .run_analysis_backend(id, provider, &paper, reset_stages, progress)
            .await;
        forwarder.stop().await;
        let outcome = outcome.map_err(|error| (ProcessingStage::Analysis, error))?;
        self.jobs
            .task_completed(id, "core")
            .await
            .map_err(|error| (ProcessingStage::Persistence, error))?;
        self.jobs
            .task_active(
                id,
                "verify",
                Some("Self-checked the draft and admitted quotations".to_owned()),
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

    /// Route a model reader through the new `paper-artifacts` backend, keeping
    /// the plan harnesses on the existing pipeline.
    async fn run_analysis_backend(
        &self,
        id: &PaperId,
        provider: AnalysisProvider,
        paper: &crate::domain::ExtractedPaper,
        reset_stages: bool,
        progress: crate::analysis::openrouter::ProgressSink,
    ) -> Result<crate::analysis::AnalysisOutcome> {
        match provider {
            AnalysisProvider::Heuristic => {
                self.analysis
                    .analyze_heuristic(provider, paper, &self.store.paper_dir(id), reset_stages)
                    .await
            }
            AnalysisProvider::Codex | AnalysisProvider::Claude | AnalysisProvider::OpenRouter => {
                let pdf = self.source_path(id).await?;
                self.analysis
                    .analyze_core_artifacts(
                        provider,
                        paper,
                        &pdf,
                        &self.store.paper_dir(id),
                        Some(progress),
                    )
                    .await
            }
        }
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

    async fn processing_queue(&self) -> Result<ProcessingQueue> {
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

    pub async fn prepare_experiment_only(
        &self,
        id: &PaperId,
        request: &StartExperimentRequest,
    ) -> Result<ExperimentRun> {
        self.prepare_experiment(id, request).await
    }

    pub async fn import_experiment_outputs(
        &self,
        id: &PaperId,
        run_id: &str,
        outputs: [LearningRamp; 2],
    ) -> Result<ExperimentView> {
        let mut run = self
            .store
            .load_experiment_run(id, run_id)
            .await?
            .ok_or_else(|| Error::InvalidRequest("experiment run was not found".to_owned()))?;
        if run.paper_id != *id || run.arms.len() != 2 {
            return Err(Error::InvalidRequest(
                "experiment import does not match a two-arm paper run".to_owned(),
            ));
        }
        if self
            .store
            .load_experiment_judgments(id)
            .await?
            .iter()
            .any(|judgment| judgment.run_id == run.id)
        {
            return Err(Error::InvalidRequest(
                "cannot replace an experiment after blind judgment".to_owned(),
            ));
        }
        let paper = self.load_or_extract(id).await.map_err(|(_, error)| error)?;
        for (arm, output) in run.arms.iter_mut().zip(outputs) {
            let normalized = crate::analysis::normalize_learning_ramp(output, &paper)?;
            if arm.variant_id == "direct"
                && normalized
                    .concepts
                    .iter()
                    .any(|concept| concept.bridge.is_some() || concept.bridge_limit.is_some())
            {
                return Err(Error::InvalidAnalysis(
                    "direct conceptual-bridge control contained an analogy".to_owned(),
                ));
            }
            arm.output = Some(normalized);
            arm.error = None;
        }
        run.status = ExperimentStatus::Completed;
        run.completed_at = Some(Utc::now());
        self.store.save_experiment_run(&run).await?;
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
                AnalysisProvider::OpenRouter => "deepseek/deepseek-v4.1-flash".to_owned(),
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
        let guard = self.tools_extract.lock().await;
        let paper = self.load_or_extract(id).await.map_err(|(_, error)| error)?;
        drop(guard);
        let analysis = self.store.load_analysis(id).await?;
        if let Some(section_id) = &request.section_id
            && !analysis
                .as_ref()
                .is_some_and(|a| a.sections.iter().any(|section| &section.id == section_id))
        {
            return Err(Error::InvalidRequest(format!(
                "unknown section: {section_id}"
            )));
        }
        self.analysis
            .clarify(
                request.provider,
                &paper,
                analysis.as_ref(),
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

fn directory_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Articles")
        .to_owned()
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
        .merge(reader_tools::routes())
        .merge(citation_graph::routes())
        .merge(literature::routes())
        .merge(acquisition::routes())
        .merge(expansion::routes())
        .merge(ramps::routes())
        .merge(provenance::routes())
        .merge(curation::routes())
        .merge(labels::routes())
        .merge(source_index::routes())
        .merge(notes::routes())
        .merge(vimrc::routes())
        .route("/api/health", get(health))
        .route("/api/library", get(library))
        .route("/api/library/scan", post(scan_library))
        .route("/api/library/import", post(import_pdf))
        .route("/api/library/directories", post(add_library_directory))
        .route(
            "/api/library/directories/{id}",
            delete(remove_library_directory),
        )
        .route(
            "/api/library/upload",
            post(upload_pdf).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/api/queue", get(processing_queue))
        .route("/api/experiments", get(experiment_catalog))
        .route("/api/works/{id}", get(get_work))
        .route("/api/works/{id}/citations", get(get_work_citations))
        .route(
            "/api/graph/neighbors/{id}",
            get(neighbors::get_graph_neighbors),
        )
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
        .route("/api/papers/{id}/abstract/refresh", post(refresh_abstract))
        .route("/api/papers/{id}/context/refresh", post(refresh_context))
        .route(
            "/api/papers/{id}/structure/refresh",
            post(refresh_structure),
        )
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
        .route("/api", any(unknown_api))
        .route("/api/{*path}", any(unknown_api))
        .route("/", get(frontend_index))
        .route("/{*asset}", get(frontend_asset))
        .with_state(state)
}

async fn unknown_api() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "unknown_api_route",
            "message": "This API endpoint is unavailable. Restart the Lysilogy backend after updating the app."
        })),
    )
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

async fn get_work(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    let work_id = match parse_work_id(&id) {
        Ok(work_id) => work_id,
        Err(response) => return *response,
    };
    match fetch_work(graph, work_id).await {
        Ok(work) => (StatusCode::OK, Json(work)).into_response(),
        Err(response) => *response,
    }
}

async fn get_work_citations(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let graph = match graph_or_unavailable(&state) {
        Ok(graph) => graph,
        Err(response) => return *response,
    };
    let work_id = match parse_work_id(&id) {
        Ok(work_id) => work_id,
        Err(response) => return *response,
    };
    let work = match fetch_work(graph, work_id).await {
        Ok(work) => work,
        Err(response) => return *response,
    };
    let outgoing = match graph.citations.outgoing(work.id).await {
        Ok(works) => works,
        Err(error) => return graph_query_failure(&error),
    };
    let incoming = match graph.citations.incoming(work.id).await {
        Ok(works) => works,
        Err(error) => return graph_query_failure(&error),
    };
    let pairs: Vec<(WorkId, WorkId)> = outgoing
        .iter()
        .map(|cited| (work.id, cited.id))
        .chain(incoming.iter().map(|citing| (citing.id, work.id)))
        .collect();
    let edges = match labels::edge_payloads(&state, &pairs).await {
        Ok(edges) => edges,
        Err(response) => return *response,
    };
    let work_ids: Vec<WorkId> = std::iter::once(&work)
        .chain(&outgoing)
        .chain(&incoming)
        .map(|work| work.id)
        .collect();
    let local_papers = match literature::local_papers_for(&state, &work_ids).await {
        Ok(local_papers) => local_papers,
        Err(response) => return *response,
    };
    let acquisitions = match acquisition::states_for(&state, &work_ids).await {
        Ok(acquisitions) => acquisitions,
        Err(response) => return *response,
    };
    let (marks, work_tags) = match curation::curation_for(&state, &work_ids).await {
        Ok(curation) => curation,
        Err(response) => return *response,
    };
    (
        StatusCode::OK,
        Json(CitationsResponse {
            work,
            outgoing,
            incoming,
            edges,
            local_papers,
            acquisitions,
            marks,
            work_tags,
        }),
    )
        .into_response()
}

type GraphError = Box<Response>;

fn graph_or_unavailable(state: &AppState) -> std::result::Result<&GraphState, GraphError> {
    state.graph.as_ref().ok_or_else(|| {
        Box::new(graph_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "graph_unavailable",
            "The literature graph database is not configured or unavailable.",
        ))
    })
}

fn parse_work_id(text: &str) -> std::result::Result<WorkId, GraphError> {
    text.parse().map_err(|_| {
        Box::new(graph_error(
            StatusCode::BAD_REQUEST,
            "invalid_work_id",
            "Work identifiers look like 'w' followed by 22 characters.",
        ))
    })
}

async fn fetch_work(graph: &GraphState, work_id: WorkId) -> std::result::Result<Work, GraphError> {
    let work_id = graph
        .works
        .resolve_redirect(work_id)
        .await
        .map_err(|error| Box::new(graph_query_failure(&error)))?;
    match graph.works.get(work_id).await {
        Ok(Some(work)) => Ok(work),
        Ok(None) => Err(Box::new(graph_error(
            StatusCode::NOT_FOUND,
            "work_not_found",
            "No such work.",
        ))),
        Err(error) => Err(Box::new(graph_query_failure(&error))),
    }
}

fn graph_query_failure(error: &lysilogy_db::DbError) -> Response {
    tracing::error!(%error, "graph query failed");
    graph_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "graph_error",
        "The graph query failed.",
    )
}

fn graph_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": code, "message": message })),
    )
        .into_response()
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

async fn add_library_directory(
    State(state): State<AppState>,
    Json(request): Json<AddDirectoryRequest>,
) -> Result<(StatusCode, Json<LibraryResponse>)> {
    let library = state.add_library_directory(&request.path).await?;
    Ok((StatusCode::CREATED, Json(library)))
}

async fn remove_library_directory(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<LibraryResponse>> {
    state.remove_library_directory(&id).await.map(Json)
}

async fn upload_pdf(
    State(state): State<AppState>,
    Query(query): Query<UploadPdfQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<UploadPdfResponse>)> {
    let uploaded = state.upload_pdf(&query.filename, &body).await?;
    Ok((StatusCode::CREATED, Json(uploaded)))
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

async fn refresh_abstract(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AnalyzeRequest>,
) -> Result<Json<PaperView>> {
    state
        .refresh_component(
            &parse_id(&id)?,
            request.provider,
            request.force,
            crate::domain::AnalysisComponent::Abstract,
        )
        .await
        .map(Json)
}

async fn refresh_context(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AnalyzeRequest>,
) -> Result<Json<PaperView>> {
    state
        .refresh_component(
            &parse_id(&id)?,
            request.provider,
            request.force,
            crate::domain::AnalysisComponent::Context,
        )
        .await
        .map(Json)
}

async fn refresh_structure(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<AnalyzeRequest>,
) -> Result<Json<PaperView>> {
    state
        .refresh_component(
            &parse_id(&id)?,
            request.provider,
            request.force,
            crate::domain::AnalysisComponent::Structure,
        )
        .await
        .map(Json)
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
    async fn unknown_api_routes_return_json_instead_of_the_frontend() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let web = tempdir().map_err(|error| Error::io("web", error))?;
        tokio::fs::write(
            web.path().join("index.html"),
            "<!doctype html><title>SPA</title>",
        )
        .await
        .map_err(|error| Error::io(web.path(), error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let app = build_router(state, Some(web.path()));
        for method in ["GET", "POST"] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(method)
                        .uri("/api/papers/1234567890abcdef/unknown-feature")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
            let body: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())?;
            assert_eq!(body["error"], "unknown_api_route");
        }
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/a-reader-route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        Ok(())
    }

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
    async fn paper_details_refresh_author_metadata_without_rescanning() -> Result<()> {
        let library = tempdir().map_err(|e| Error::io("library", e))?;
        let data = tempdir().map_err(|e| Error::io("data", e))?;
        let filename = "Irving et al. - 2018 - AI safety via debate.pdf";
        tokio::fs::write(library.path().join(filename), b"metadata fixture")
            .await
            .map_err(|e| Error::io("fixture", e))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let id = PaperId::from_relative_path(Path::new(filename));
        let mut metadata = state.paper(&id).await?.paper.metadata;
        metadata.authors = vec![
            "Geoffrey Irving".into(),
            "Paul Christiano".into(),
            "Dario Amodei".into(),
        ];
        state
            .store
            .save_extraction(
                &id,
                &ExtractedPaper {
                    metadata: metadata.clone(),
                    pages: Vec::new(),
                    layout: DocumentLayout::default(),
                },
            )
            .await?;
        assert_eq!(
            state.paper(&id).await?.paper.metadata.authors,
            metadata.authors
        );
        assert_eq!(
            state.library().await.papers[0].metadata.authors,
            metadata.authors
        );
        Ok(())
    }

    #[tokio::test]
    async fn answers_source_questions_without_creating_an_analysis() -> Result<()> {
        let library = tempdir().map_err(|e| Error::io("library", e))?;
        let data = tempdir().map_err(|e| Error::io("data", e))?;
        let filename = "Ada - 1843 - Notes.pdf";
        let fixture = library.path().join(filename);
        tokio::fs::write(&fixture, b"source already extracted")
            .await
            .map_err(|e| Error::io(&fixture, e))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let id = PaperId::from_relative_path(Path::new(filename));
        state
            .store
            .save_extraction(
                &id,
                &ExtractedPaper {
                    metadata: PaperMetadata::default(),
                    pages: vec![ExtractedPage {
                        number: 1,
                        text: "The measurement depends on the sampling assumption.".to_owned(),
                    }],
                    layout: DocumentLayout::default(),
                },
            )
            .await?;
        let result = state
            .clarify(
                &id,
                &ClarifyRequest {
                    section_id: None,
                    selection: "The measurement depends on the sampling assumption.".to_owned(),
                    question: "What assumption?".to_owned(),
                    provider: AnalysisProvider::Heuristic,
                },
            )
            .await?;
        assert!(!result.answer.is_empty());
        assert!(state.store.load_analysis(&id).await?.is_none());
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

    async fn post_upload(
        state: &AppState,
        filename: &str,
        body: &'static [u8],
    ) -> Result<StatusCode> {
        let response = build_router(state.clone(), None)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/api/library/upload?filename={filename}"))
                    .header(header::CONTENT_TYPE, "application/pdf")
                    .body(Body::from(body))
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        Ok(response.status())
    }

    const FIXTURE_PDF: &[u8] = b"%PDF-1.7\nfixture\n%%EOF\n";

    fn write_pdf(directory: &Path, name: &str) -> Result<()> {
        std::fs::write(directory.join(name), FIXTURE_PDF).map_err(|error| Error::io(name, error))
    }

    #[tokio::test]
    async fn lists_papers_from_every_added_directory_under_distinct_ids() -> Result<()> {
        let primary = tempdir().map_err(|error| Error::io("primary", error))?;
        let extra = tempdir().map_err(|error| Error::io("extra", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        write_pdf(primary.path(), "same.pdf")?;
        write_pdf(extra.path(), "same.pdf")?;
        let state = AppState::new(primary.path(), data.path()).await?;
        let library = state
            .add_library_directory(&extra.path().display().to_string())
            .await?;
        assert_eq!(library.papers.len(), 2);
        assert_ne!(library.papers[0].id, library.papers[1].id);
        let counts = library
            .directories
            .iter()
            .map(|directory| (directory.primary, directory.papers))
            .collect::<Vec<_>>();
        assert_eq!(counts, [(true, 1), (false, 1)]);
        for paper in &library.papers {
            assert!(state.source_path(&paper.id).await?.exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn keeps_added_directories_across_restarts_and_skips_missing_ones() -> Result<()> {
        let primary = tempdir().map_err(|error| Error::io("primary", error))?;
        let extra = tempdir().map_err(|error| Error::io("extra", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        write_pdf(extra.path(), "kept.pdf")?;
        let state = AppState::new(primary.path(), data.path()).await?;
        state
            .add_library_directory(&extra.path().display().to_string())
            .await?;
        let restarted = AppState::new(primary.path(), data.path()).await?;
        assert_eq!(restarted.library().await.papers.len(), 1);
        drop(extra);
        let without_folder = AppState::new(primary.path(), data.path()).await?;
        let library = without_folder.library().await;
        assert!(library.papers.is_empty());
        assert!(!library.directories[1].available);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_directories_that_overlap_existing_ones() -> Result<()> {
        let primary = tempdir().map_err(|error| Error::io("primary", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let nested = primary.path().join("nested");
        std::fs::create_dir(&nested).map_err(|error| Error::io(&nested, error))?;
        let state = AppState::new(primary.path(), data.path()).await?;
        for path in [
            primary.path().display().to_string(),
            nested.display().to_string(),
            "relative/dir".to_owned(),
            String::new(),
        ] {
            assert!(
                matches!(
                    state.add_library_directory(&path).await,
                    Err(Error::InvalidRequest(_))
                ),
                "{path:?} should be rejected"
            );
        }
        let other = tempdir().map_err(|error| Error::io("other", error))?;
        let other_path = other.path().display().to_string();
        state.add_library_directory(&other_path).await?;
        assert!(state.add_library_directory(&other_path).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn removes_a_directory_without_deleting_its_files() -> Result<()> {
        let primary = tempdir().map_err(|error| Error::io("primary", error))?;
        let extra = tempdir().map_err(|error| Error::io("extra", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        write_pdf(extra.path(), "kept.pdf")?;
        let state = AppState::new(primary.path(), data.path()).await?;
        let added = state
            .add_library_directory(&extra.path().display().to_string())
            .await?;
        let id = added.directories[1].id.clone();
        let library = state.remove_library_directory(&id).await?;
        assert!(library.papers.is_empty());
        assert_eq!(library.directories.len(), 1);
        assert!(extra.path().join("kept.pdf").exists());
        assert!(state.remove_library_directory(&id).await.is_err());
        assert!(state.remove_library_directory("primary").await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn rejects_uploads_that_are_not_pdfs_without_writing_files() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let status = post_upload(&state, "notes.pdf", b"<html>not a pdf</html>").await?;
        assert_eq!(status, StatusCode::BAD_REQUEST);
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
    async fn keeps_both_files_when_an_uploaded_name_already_exists() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let pdf: &'static [u8] = b"%PDF-1.7\nfixture\n%%EOF\n";
        assert_eq!(
            post_upload(&state, "a%2Fb.pdf", pdf).await?,
            StatusCode::CREATED
        );
        assert_eq!(
            post_upload(&state, "a%2Fb.pdf", pdf).await?,
            StatusCode::CREATED
        );
        assert!(library.path().join("a_b.pdf").exists());
        assert!(library.path().join("a_b (2).pdf").exists());
        assert_eq!(state.library().await.papers.len(), 2);
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
    fn should_keep_import_behavior_when_download_moves_to_remote() {
        use crate::remote::reject_content_class;
        use lysilogy_ingest::acquire::classify_content_type;

        let html = classify_content_type(Some("text/html; charset=utf-8"));
        let rejection = reject_content_class(&html, false).expect("HTML is rejected for import");
        assert!(!rejection.transient);
        assert!(
            matches!(
                rejection.into_error(),
                Error::InvalidRequest(message) if message == "the downloaded resource is not a PDF"
            ),
            "import keeps its not-a-PDF message"
        );
        let pdf = classify_content_type(Some("application/pdf"));
        assert!(reject_content_class(&pdf, false).is_none());
        assert!(reject_content_class(&html, true).is_none());
    }

    #[test]
    fn validates_pdf_headers_and_portable_import_names() {
        use lysilogy_ingest::acquire::has_pdf_magic;

        assert!(has_pdf_magic(b"preamble\n%PDF-1.7\n"));
        assert!(!has_pdf_magic(b"<!doctype html>"));
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
                >= 4
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
        assert_eq!(catalog.experiments.len(), 6);
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

    #[tokio::test]
    async fn work_endpoint_reports_graph_unavailable_without_a_database() -> Result<()> {
        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let state = AppState::new(library.path(), data.path()).await?;
        let app = build_router(state, None);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/works/w0000000000000000000000")
                    .body(Body::empty())
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }

    #[tokio::test]
    async fn work_endpoint_returns_a_seeded_work() -> Result<()> {
        let Some(database_url) = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())
        else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return Ok(());
        };
        let graph = crate::graph::GraphState::connect(&database_url)
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        let work = graph
            .works
            .create(
                lysilogy_core::WorkKind::Article,
                lysilogy_core::Visibility::Private,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;

        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let state = AppState::new(library.path(), data.path())
            .await?
            .with_graph(graph);
        let app = build_router(state, None);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/works/{}", work.id))
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
        let json: serde_json::Value = serde_json::from_slice(&body)?;
        let expected = work.id.to_string();
        assert_eq!(json["id"].as_str(), Some(expected.as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn citations_and_neighbors_report_seeded_edges() -> Result<()> {
        let Some(database_url) = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())
        else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return Ok(());
        };
        let graph = crate::graph::GraphState::connect(&database_url)
            .await
            .map_err(|error| Error::Task(error.to_string()))?;

        let marker = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| Error::Task(error.to_string()))?
            .as_nanos();
        let article = lysilogy_core::WorkKind::Article;
        let private = lysilogy_core::Visibility::Private;
        let center = graph.works.create(article, private).await.map_err(db)?;
        let cited_a = graph.works.create(article, private).await.map_err(db)?;
        let cited_b = graph.works.create(article, private).await.map_err(db)?;
        let citer = graph.works.create(article, private).await.map_err(db)?;
        let source = graph
            .source_records
            .upsert(
                "test",
                &format!("k-{marker}"),
                lysilogy_core::Coverage::Complete,
                &serde_json::json!({}),
            )
            .await
            .map_err(db)?;
        graph
            .citations
            .add_assertion(center.id, cited_a.id, Some(source))
            .await
            .map_err(db)?;
        graph
            .citations
            .add_assertion(center.id, cited_b.id, Some(source))
            .await
            .map_err(db)?;
        graph
            .citations
            .add_assertion(citer.id, center.id, Some(source))
            .await
            .map_err(db)?;

        let library = tempdir().map_err(|error| Error::io("library", error))?;
        let data = tempdir().map_err(|error| Error::io("data", error))?;
        let state = AppState::new(library.path(), data.path())
            .await?
            .with_graph(graph);
        let app = build_router(state, None);

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/works/{}/citations", center.id))
                    .body(Body::empty())
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await?;
        assert_eq!(json["outgoing"].as_array().map(Vec::len), Some(2));
        assert_eq!(json["incoming"].as_array().map(Vec::len), Some(1));

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/graph/neighbors/{}?direction=both", center.id))
                    .body(Body::empty())
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await?;
        assert_eq!(json["center"], center.id.to_string());
        assert_eq!(json["direction"], "both");
        assert_eq!(json["nodes"].as_array().map(Vec::len), Some(4));
        assert_eq!(json["edges"].as_array().map(Vec::len), Some(3));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/api/graph/neighbors/{}?direction=bogus",
                        center.id
                    ))
                    .body(Body::empty())
                    .map_err(|error| Error::Task(error.to_string()))?,
            )
            .await
            .map_err(|error| Error::Task(error.to_string()))?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[allow(clippy::needless_pass_by_value)]
    fn db(error: lysilogy_db::DbError) -> Error {
        Error::Task(error.to_string())
    }

    async fn body_json(response: Response) -> Result<serde_json::Value> {
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|error| Error::Task(error.to_string()))?
            .to_bytes();
        serde_json::from_slice(&body).map_err(Error::from)
    }
}
