use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use tokio::{fs::File, sync::RwLock};
use tokio_util::io::ReaderStream;

use crate::{
    Result,
    analysis::AnalysisService,
    domain::{
        AnalysisProvider, AnalyzeRequest, Clarification, ClarifyRequest, PaperId, PaperOverview,
        PaperView, ProcessingStage, ProcessingStatus,
    },
    error::Error,
    extract::PdfExtractor,
    library::LibraryCatalog,
    store::ArtifactStore,
};

#[derive(Clone, Debug)]
pub struct AppState {
    catalog: Arc<RwLock<LibraryCatalog>>,
    library_root: Arc<PathBuf>,
    store: ArtifactStore,
    extractor: PdfExtractor,
    analysis: AnalysisService,
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
        Ok(Self {
            catalog: Arc::new(RwLock::new(catalog)),
            library_root: Arc::new(library_root),
            store,
            extractor,
            analysis,
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
        {
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
        }

        if let Err((stage, error)) = self.run_analysis(id, provider).await {
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

        let state = self.clone();
        tokio::spawn(async move {
            if let Err((stage, error)) = state.run_analysis(&id, provider).await {
                tracing::error!(paper_id = %id, %stage, %error, "paper analysis failed");
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

        let paper = match self.store.load_extraction(id).await {
            Ok(Some(paper)) => paper,
            Ok(None) => {
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
                paper
            }
            Err(error) => return Err((ProcessingStage::Persistence, error)),
        };

        {
            let mut catalog = self.catalog.write().await;
            if let Some(entry) = catalog.get_mut(id) {
                entry.overview.metadata = paper.metadata.clone();
                entry.overview.status = ProcessingStatus::Analyzing { provider };
            }
        }
        let analysis = self
            .analysis
            .analyze(provider, &paper, &self.store.paper_dir(id))
            .await
            .map_err(|error| (ProcessingStage::Analysis, error))?;
        self.store
            .save_analysis(id, &analysis)
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
        .route("/api/papers/{id}", get(paper))
        .route("/api/papers/{id}/source", get(paper_source))
        .route("/api/papers/{id}/analyze", post(analyze_paper))
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
}
