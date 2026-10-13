//! The optional Postgres-backed literature graph.
//!
//! Attached to the API when a database is configured. A missing or unavailable
//! database never stops the reader; graph routes report an explicit unavailable
//! state instead.

use lysilogy_db::PgPool;
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::document::DocumentRepository;
use lysilogy_db::evidence::EvidenceRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;

/// Repositories backing the graph API.
#[derive(Clone)]
pub struct GraphState {
    pub works: WorkRepository,
    pub documents: DocumentRepository,
    pub citations: CitationRepository,
    pub source_records: SourceRecordRepository,
    pub evidence: EvidenceRepository,
}

impl std::fmt::Debug for GraphState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GraphState")
    }
}

impl GraphState {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            works: WorkRepository::new(pool.clone()),
            documents: DocumentRepository::new(pool.clone()),
            citations: CitationRepository::new(pool.clone()),
            source_records: SourceRecordRepository::new(pool.clone()),
            evidence: EvidenceRepository::new(pool),
        }
    }

    /// Connect to Postgres and apply pending migrations.
    pub async fn connect(url: &str) -> Result<Self, lysilogy_db::DbError> {
        let pool = lysilogy_db::connect(url).await?;
        lysilogy_db::migrate(&pool).await?;
        Ok(Self::new(pool))
    }
}
