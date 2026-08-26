use std::{io, path::PathBuf};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("paper not found: {0}")]
    PaperNotFound(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("a task is already running for paper {0}")]
    AlreadyProcessing(String),
    #[error("could not access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{program} failed with status {status}: {stderr}")]
    CommandFailed {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("required program is unavailable: {0}")]
    ProgramUnavailable(String),
    #[error("PDF extraction produced no readable text for {0}")]
    EmptyExtraction(PathBuf),
    #[error("structured analysis was invalid: {0}")]
    InvalidAnalysis(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("task failed: {0}")]
    Task(String),
}

impl Error {
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, kind) = match &self {
            Self::PaperNotFound(_) => (StatusCode::NOT_FOUND, "paper_not_found"),
            Self::InvalidRequest(_) | Self::InvalidAnalysis(_) => {
                (StatusCode::BAD_REQUEST, "invalid_request")
            }
            Self::AlreadyProcessing(_) => (StatusCode::CONFLICT, "already_processing"),
            Self::ProgramUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "program_unavailable"),
            Self::Io { .. }
            | Self::CommandFailed { .. }
            | Self::EmptyExtraction(_)
            | Self::Json(_)
            | Self::Task(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        let body = ErrorBody {
            error: kind,
            message: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}
