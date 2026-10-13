//! Ingestion and provider adapters for the Lysilogy literature viewer.
//!
//! Library scanning, paper object extraction, bibliography extraction, and
//! citation-provider clients live here.

pub mod bibliography;
pub mod identify;
pub mod ingest;
pub mod literature;
pub mod local_evidence;
pub mod openalex;
pub mod scan;
pub mod title_match;

/// Errors returned by ingestion.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] lysilogy_db::DbError),
    #[error(transparent)]
    Provider(#[from] crate::openalex::ProviderError),
}

/// Result alias for ingestion.
pub type Result<T> = std::result::Result<T, IngestError>;
