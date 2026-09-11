use super::{
    CitationEdge, CitationProvider, Coverage, Direction, GraphFailure, GraphFuture, GraphResult,
    Identifier, Provider, ProviderReport, Request, Transport, Work, text,
};

pub struct OpenCitations;
impl CitationProvider for OpenCitations {
    fn provider(&self) -> Provider {
        Provider::Opencitations
    }
    fn fetch<'a>(
        &'a self,
        transport: &'a dyn Transport,
        id: &'a Identifier,
        direction: Direction,
        limit: usize,
    ) -> GraphFuture<'a, ProviderReport> {
        Box::pin(async move {
            let mut report = ProviderReport::new(self.provider(), direction);
            if let Err(error) = collect(transport, id, direction, limit, &mut report).await {
                report.fail(error);
            }
            report
        })
    }
}
async fn collect(
    transport: &dyn Transport,
    id: &Identifier,
    direction: Direction,
    limit: usize,
    report: &mut ProviderReport,
) -> GraphResult<()> {
    if !matches!(id.scheme.as_str(), "doi" | "pmid" | "omid") {
        return Err(GraphFailure::unsupported(
            "OpenCitations requires DOI, PMID, or OMID",
        ));
    }
    report.target = Some(Work::identified(id.clone()));
    let request = Request::new(
        Provider::Opencitations,
        "https://api.opencitations.net/index/v2/",
        &[direction.path(), &id.key()],
    );
    report.requests.push(request.url.to_string());
    let value = transport.get(request).await?;
    let entries = value.as_array().ok_or_else(GraphFailure::malformed)?;
    report.total_reported = Some(entries.len() as u64);
    if entries.len() > limit {
        report.coverage = Coverage::Capped;
    }
    // Index v2 has no cursor/offset contract: one bounded response, explicitly capped locally.
    for entry in entries.iter().take(limit) {
        let citing = work(&text(entry, "citing").ok_or_else(GraphFailure::malformed)?)?;
        let cited = work(&text(entry, "cited").ok_or_else(GraphFailure::malformed)?)?;
        report.edges.push(CitationEdge {
            citing,
            cited,
            provider_edge_id: text(entry, "oci"),
            contexts: vec![],
            intents: vec![],
            is_influential: None,
            reference: Some(entry.clone()),
        });
    }
    Ok(())
}
fn work(value: &str) -> GraphResult<Work> {
    // Multiple identifiers (including multiple DOIs) and source index prefixes are retained.
    let identifiers = value
        .split_whitespace()
        .filter_map(|part| Identifier::parse(part.trim_end_matches(';')).ok())
        .collect::<Vec<_>>();
    if identifiers.is_empty() {
        return Err(GraphFailure::malformed());
    }
    let url = identifiers
        .iter()
        .find(|id| id.scheme == "doi")
        .and_then(|id| Work::identified(id.clone()).url);
    Ok(Work {
        identifiers,
        url,
        ..Work::default()
    })
}
