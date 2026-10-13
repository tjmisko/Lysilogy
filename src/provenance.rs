//! Field provenance for Works: which claim supplies each displayed field.

use lysilogy_core::WorkId;

use crate::{AppState, Error, Result};

/// Which Works a reselect covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReselectScope {
    /// One Work (redirects are followed).
    Work(WorkId),
    /// Every Work with at least one claim row.
    All,
}

impl AppState {
    /// Recompute the selected title, year and authors from claims. Returns how
    /// many Works were reselected.
    ///
    /// # Errors
    /// `InvalidRequest` without a database; `Literature` for database failures.
    pub async fn reselect_claims(&self, scope: ReselectScope) -> Result<usize> {
        let graph = self.graph.as_ref().ok_or_else(|| {
            Error::InvalidRequest(
                "provenance needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
            )
        })?;
        let database = |error: lysilogy_db::DbError| Error::Literature(error.to_string());
        match scope {
            ReselectScope::Work(work) => {
                let work = graph.works.resolve_redirect(work).await.map_err(database)?;
                graph.claims.reselect(work).await.map_err(database)?;
                Ok(1)
            }
            ReselectScope::All => graph.claims.reselect_all().await.map_err(database),
        }
    }
}
