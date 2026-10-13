//! Refreshing the per-workspace prominence cache around a set of Works,
//! optionally fetching `OpenAlex` cited-by counts first.
//!
//! A metrics failure never fails a refresh: it becomes a warning and the
//! refresh continues with whatever counts are already cached.

use std::collections::HashSet;

use lysilogy_core::{WorkId, WorkspaceId, prominence_score};
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::prominence::ProminenceRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;

use crate::Result;
use crate::openalex::{JsonTransport, OpenAlexClient, fetch_cited_by_counts_cached};

/// The repositories and optional provider client a refresh works through.
pub struct ProminenceContext<'a, T: JsonTransport> {
    pub works: &'a WorkRepository,
    pub citations: &'a CitationRepository,
    pub records: &'a SourceRecordRepository,
    pub prominence: &'a ProminenceRepository,
    pub openalex: Option<&'a OpenAlexClient<T>>,
}

/// What one refresh does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshOptions {
    /// Fetch cited-by counts for Works without a cached one (needs a client).
    pub fetch_metrics: bool,
    /// With `fetch_metrics`, refetch counts even when cached.
    pub refresh_metrics: bool,
    pub current_year: i32,
}

/// What one refresh did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshSummary {
    /// Works whose prominence rows were recomputed.
    pub works: usize,
    /// Of those, how many have a cited-by count.
    pub with_global_count: usize,
    /// Works that have a cited-by count after the fetch step.
    pub metrics_fetched: usize,
    pub metric_requests: usize,
    /// Stable `snake_case` codes with short context.
    pub warnings: Vec<String>,
}

/// The Work, the Works it cites, and the Works citing it, deduplicated with the
/// Work first.
pub async fn neighborhood_of(citations: &CitationRepository, work: WorkId) -> Result<Vec<WorkId>> {
    let mut seen = HashSet::from([work]);
    let mut neighborhood = vec![work];
    let outgoing = citations.outgoing(work).await?;
    let incoming = citations.incoming(work).await?;
    for neighbor in outgoing.into_iter().chain(incoming) {
        if seen.insert(neighbor.id) {
            neighborhood.push(neighbor.id);
        }
    }
    Ok(neighborhood)
}

/// Recompute and store prominence for `works`.
pub async fn refresh_prominence<T: JsonTransport>(
    context: &ProminenceContext<'_, T>,
    workspace: WorkspaceId,
    works: &[WorkId],
    options: RefreshOptions,
) -> Result<RefreshSummary> {
    let mut summary = RefreshSummary::default();
    if works.is_empty() {
        return Ok(summary);
    }
    if options.fetch_metrics
        && let Some(client) = context.openalex
    {
        fetch_metrics(
            context,
            client,
            works,
            options.refresh_metrics,
            &mut summary,
        )
        .await?;
    }
    let signals = context.prominence.compute_signals(workspace, works).await?;
    let scored: Vec<_> = signals
        .into_iter()
        .map(|(work, signals)| {
            let score = prominence_score(&signals, options.current_year);
            (work, signals, score)
        })
        .collect();
    context.prominence.store(workspace, &scored).await?;
    summary.works = scored.len();
    summary.with_global_count = scored
        .iter()
        .filter(|(_, signals, _)| signals.cited_by_global_count.is_some())
        .count();
    Ok(summary)
}

async fn fetch_metrics<T: JsonTransport>(
    context: &ProminenceContext<'_, T>,
    client: &OpenAlexClient<T>,
    works: &[WorkId],
    refresh: bool,
    summary: &mut RefreshSummary,
) -> Result<()> {
    let openalex_ids: Vec<String> = context
        .works
        .openalex_ids_for(works)
        .await?
        .into_iter()
        .map(|(_, id)| id.to_ascii_uppercase())
        .collect();
    if openalex_ids.is_empty() {
        return Ok(());
    }
    match fetch_cited_by_counts_cached(context.records, client, &openalex_ids, refresh).await {
        Ok(fetched) => {
            summary.metrics_fetched = fetched.counts.len();
            summary.metric_requests = fetched.requests;
        }
        Err(error) => summary
            .warnings
            .push(format!("prominence_metrics_failed:{error}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openalex::{ProviderError, ProviderFuture, fake::RecordingTransport};
    use lysilogy_core::{IdentifierScheme, WorkKind};
    use lysilogy_db::document::DocumentRepository;
    use serde_json::Value;

    const YEAR: i32 = 2026;

    struct Fixture {
        works: WorkRepository,
        citations: CitationRepository,
        records: SourceRecordRepository,
        prominence: ProminenceRepository,
        workspace: WorkspaceId,
    }

    impl Fixture {
        async fn connect() -> Option<Self> {
            let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
                .ok()
                .filter(|url| !url.is_empty())?;
            let pool = lysilogy_db::connect(&url).await.expect("connect");
            lysilogy_db::migrate(&pool).await.expect("migrate");
            let workspace = DocumentRepository::new(pool.clone())
                .ensure_default_workspace()
                .await
                .expect("workspace");
            Some(Self {
                works: WorkRepository::new(pool.clone()),
                citations: CitationRepository::new(pool.clone()),
                records: SourceRecordRepository::new(pool.clone()),
                prominence: ProminenceRepository::new(pool),
                workspace,
            })
        }

        fn context<'a, T: JsonTransport>(
            &'a self,
            openalex: Option<&'a OpenAlexClient<T>>,
        ) -> ProminenceContext<'a, T> {
            ProminenceContext {
                works: &self.works,
                citations: &self.citations,
                records: &self.records,
                prominence: &self.prominence,
                openalex,
            }
        }

        async fn work(&self, openalex_id: &str) -> WorkId {
            self.works
                .resolve_or_create(
                    IdentifierScheme::Openalex,
                    openalex_id,
                    WorkKind::Article,
                    None,
                )
                .await
                .expect("work")
                .0
                .id
        }

        async fn cites(&self, citing: WorkId, cited: WorkId) {
            self.citations
                .add_assertion(citing, cited, None)
                .await
                .expect("assertion");
        }

        async fn stored(&self, works: &[WorkId]) -> usize {
            self.prominence
                .cached(self.workspace, works)
                .await
                .expect("cached")
                .len()
        }
    }

    fn options(fetch_metrics: bool) -> RefreshOptions {
        RefreshOptions {
            fetch_metrics,
            refresh_metrics: false,
            current_year: YEAR,
        }
    }

    fn unique_id(tag: u32) -> String {
        format!("w91{}{tag}", std::process::id())
    }

    fn metrics_transport() -> RecordingTransport {
        RecordingTransport::new(|url| {
            let filter = url
                .split("filter=openalex:")
                .nth(1)
                .and_then(|rest| rest.split('&').next())
                .unwrap_or_default();
            let results: Vec<Value> = filter
                .split('|')
                .map(|id| serde_json::json!({"id": format!("https://openalex.org/{id}"), "cited_by_count": 11}))
                .collect();
            serde_json::json!({ "results": results })
        })
    }

    struct FailingTransport;

    impl JsonTransport for FailingTransport {
        fn get_json<'a>(
            &'a self,
            _url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async { Err(ProviderError::Status(503)) })
        }
    }

    #[tokio::test]
    async fn should_refresh_center_and_both_directions_when_refreshing_neighborhood() {
        let Some(fixture) = Fixture::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let center = fixture.work(&unique_id(1)).await;
        let cited = fixture.work(&unique_id(2)).await;
        let citing = fixture.work(&unique_id(3)).await;
        fixture.cites(center, cited).await;
        fixture.cites(citing, center).await;
        fixture.cites(center, cited).await;

        let neighborhood = neighborhood_of(&fixture.citations, center)
            .await
            .expect("neighborhood");
        assert_eq!(neighborhood.len(), 3);
        assert_eq!(neighborhood[0], center);

        let transport = metrics_transport();
        let client = OpenAlexClient::new(transport.clone());
        let context = fixture.context(Some(&client));
        let summary = refresh_prominence(&context, fixture.workspace, &neighborhood, options(true))
            .await
            .expect("refresh");
        assert_eq!(summary.works, 3);
        assert_eq!(summary.with_global_count, 3);
        assert_eq!(summary.metrics_fetched, 3);
        assert_eq!(summary.metric_requests, 1);
        assert!(summary.warnings.is_empty());
        assert_eq!(fixture.stored(&neighborhood).await, 3);
    }

    #[tokio::test]
    async fn should_skip_metrics_when_no_client_given() {
        let Some(fixture) = Fixture::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work(&unique_id(4)).await;
        let context = fixture.context::<RecordingTransport>(None);
        let summary = refresh_prominence(&context, fixture.workspace, &[work], options(true))
            .await
            .expect("refresh");
        assert_eq!(summary.works, 1);
        assert_eq!(summary.with_global_count, 0);
        assert_eq!(summary.metrics_fetched, 0);
        assert_eq!(summary.metric_requests, 0);
        assert!(summary.warnings.is_empty());
        assert_eq!(fixture.stored(&[work]).await, 1);
    }

    #[tokio::test]
    async fn should_keep_refreshing_with_cached_counts_when_metrics_fetch_fails() {
        let Some(fixture) = Fixture::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let cached = fixture.work(&unique_id(5)).await;
        let uncached = fixture.work(&unique_id(6)).await;
        let working = OpenAlexClient::new(metrics_transport());
        let prime = fixture.context(Some(&working));
        refresh_prominence(&prime, fixture.workspace, &[cached], options(true))
            .await
            .expect("prime");

        let failing = OpenAlexClient::new(FailingTransport);
        let context = fixture.context(Some(&failing));
        let summary = refresh_prominence(
            &context,
            fixture.workspace,
            &[cached, uncached],
            options(true),
        )
        .await
        .expect("refresh survives");
        assert_eq!(summary.works, 2);
        assert_eq!(summary.with_global_count, 1, "cached count still used");
        assert_eq!(summary.warnings.len(), 1);
        assert!(
            summary.warnings[0].starts_with("prominence_metrics_failed:"),
            "{:?}",
            summary.warnings
        );
        assert_eq!(fixture.stored(&[cached, uncached]).await, 2);
    }

    #[tokio::test]
    async fn should_make_no_requests_when_counts_already_cached() {
        let Some(fixture) = Fixture::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work(&unique_id(7)).await;
        let transport = metrics_transport();
        let client = OpenAlexClient::new(transport.clone());
        let context = fixture.context(Some(&client));
        refresh_prominence(&context, fixture.workspace, &[work], options(true))
            .await
            .expect("first");
        let second = refresh_prominence(&context, fixture.workspace, &[work], options(true))
            .await
            .expect("second");
        assert_eq!(second.metric_requests, 0);
        assert_eq!(second.metrics_fetched, 1);
        assert_eq!(transport.urls().len(), 1);
        let refreshed = refresh_prominence(
            &context,
            fixture.workspace,
            &[work],
            RefreshOptions {
                refresh_metrics: true,
                ..options(true)
            },
        )
        .await
        .expect("refresh");
        assert_eq!(refreshed.metric_requests, 1);
    }

    #[tokio::test]
    async fn should_return_empty_summary_when_no_works_given() {
        let Some(fixture) = Fixture::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let context = fixture.context::<RecordingTransport>(None);
        let summary = refresh_prominence(&context, fixture.workspace, &[], options(true))
            .await
            .expect("refresh");
        assert_eq!(summary, RefreshSummary::default());
    }
}
