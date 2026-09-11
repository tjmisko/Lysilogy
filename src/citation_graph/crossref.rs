use super::{
    CitationEdge, CitationProvider, Coverage, Direction, GraphFailure, GraphFuture, GraphResult,
    Identifier, Provider, ProviderReport, Request, Transport, Work, text, year,
};
use serde_json::Value;

pub struct Crossref;
impl CitationProvider for Crossref {
    fn provider(&self) -> Provider {
        Provider::Crossref
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
    if direction == Direction::Citations {
        return Err(GraphFailure::unsupported(
            "Crossref public REST provides incoming counts, not the list of citing works; use a citation graph provider",
        ));
    }
    if id.scheme != "doi" {
        return Err(GraphFailure::unsupported("Crossref requires a DOI"));
    }
    let request = Request::new(
        Provider::Crossref,
        "https://api.crossref.org/",
        &["works", &id.value],
    );
    report.requests.push(request.url.to_string());
    let response = transport.get(request).await?;
    let record = response
        .get("message")
        .filter(|v| v.is_object())
        .ok_or_else(GraphFailure::malformed)?;
    let target = work(record, id);
    report.target = Some(target.clone());
    let Some(references) = record.get("reference") else {
        report.coverage = Coverage::Unavailable;
        report.failure = Some(GraphFailure::new(
            super::FailureKind::Unavailable,
            "Publisher did not deposit a reference list; this does not mean the paper has no references",
        ));
        return Ok(());
    };
    let references = references.as_array().ok_or_else(GraphFailure::malformed)?;
    report.total_reported = Some(references.len() as u64);
    if references.len() > limit {
        report.coverage = Coverage::Capped;
    }
    for reference in references.iter().take(limit) {
        let mut neighbor = text(reference, "DOI").map_or_else(Work::default, |value| {
            Work::identified(Identifier {
                scheme: "doi".to_owned(),
                value,
            })
        });
        neighbor.title =
            text(reference, "article-title").or_else(|| text(reference, "volume-title"));
        neighbor.year = reference.get("year").and_then(|v| {
            v.as_str()
                .and_then(|v| v.parse::<u16>().ok())
                .or_else(|| year(v))
        });
        neighbor.authors = text(reference, "author").into_iter().collect();
        let mut edge = CitationEdge::new(&target, neighbor, direction);
        edge.provider_edge_id = text(reference, "key");
        edge.reference = Some(reference.clone());
        report.edges.push(edge);
    }
    Ok(())
}
fn work(value: &Value, id: &Identifier) -> Work {
    let mut work = Work::identified(id.clone());
    work.title = value["title"]
        .as_array()
        .and_then(|values| values.first())
        .and_then(Value::as_str)
        .map(str::to_owned);
    work.authors = value["author"].as_array().map_or_else(Vec::new, |authors| {
        authors
            .iter()
            .filter_map(|author| {
                let name = [text(author, "given"), text(author, "family")]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
                if name.is_empty() {
                    text(author, "name")
                } else {
                    Some(name)
                }
            })
            .collect()
    });
    work.year = ["published", "published-print", "published-online", "issued"]
        .into_iter()
        .find_map(|key| year(&value[key]["date-parts"][0][0]));
    work
}
