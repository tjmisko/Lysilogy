use super::{
    CitationEdge, CitationProvider, Coverage, Direction, GraphFailure, GraphFuture, GraphResult,
    Identifier, Provider, ProviderReport, Request, Transport, Work, array, strings, text, year,
};
use serde_json::Value;

pub struct SemanticScholar;
impl CitationProvider for SemanticScholar {
    fn provider(&self) -> Provider {
        Provider::SemanticScholar
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
const FIELDS: &str = "title,authors,year,externalIds,url";
async fn get(
    transport: &dyn Transport,
    request: Request,
    report: &mut ProviderReport,
) -> GraphResult<Value> {
    report.requests.push(request.url.to_string());
    transport.get(request).await
}
async fn collect(
    transport: &dyn Transport,
    id: &Identifier,
    direction: Direction,
    limit: usize,
    report: &mut ProviderReport,
) -> GraphResult<()> {
    let lookup = match id.scheme.as_str() {
        "doi" => format!("DOI:{}", id.value),
        "arxiv" => format!("ARXIV:{}", id.value),
        "pmid" => format!("PMID:{}", id.value),
        "s2" => id.value.clone(),
        _ => {
            return Err(GraphFailure::unsupported(
                "Semantic Scholar requires DOI, arXiv, PMID, or Semantic Scholar paper ID",
            ));
        }
    };
    let metadata = get(
        transport,
        request(&["paper", &lookup]).query("fields", FIELDS),
        report,
    )
    .await?;
    let target = work(&metadata)?;
    if target.identifiers.is_empty() {
        return Err(GraphFailure::malformed());
    }
    report.target = Some(target.clone());
    let mut offset = 0;
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..20 {
        let req = request(&["paper", &lookup, direction.path()])
            .query(
                "fields",
                &format!("{FIELDS},contexts,intents,isInfluential"),
            )
            .query("offset", &offset.to_string())
            .query("limit", &(limit - report.edges.len()).min(100).to_string());
        let page = get(transport, req, report).await?;
        let entries = array(&page, "data")?;
        for entry in entries {
            let neighbor_key = if direction == Direction::References {
                "citedPaper"
            } else {
                "citingPaper"
            };
            // Unresolved provider records retain raw references instead of disappearing.
            let neighbor = if entry[neighbor_key].is_null() {
                Work::default()
            } else {
                work(&entry[neighbor_key])?
            };
            let key = neighbor
                .identifiers
                .first()
                .map_or_else(|| entry.to_string(), Identifier::key);
            if seen.insert(key) && report.edges.len() < limit {
                let mut edge = CitationEdge::new(&target, neighbor, direction);
                edge.contexts = strings(&entry["contexts"]);
                edge.intents = strings(&entry["intents"]);
                edge.is_influential = entry["isInfluential"].as_bool();
                if edge.citing.identifiers.is_empty() || edge.cited.identifiers.is_empty() {
                    edge.reference = Some(entry.clone());
                }
                report.edges.push(edge);
            }
        }
        let next = page.get("next").and_then(Value::as_u64);
        if entries.is_empty() && next.is_some() {
            return Err(GraphFailure::malformed());
        }
        if next.is_none() {
            return Ok(());
        }
        if report.edges.len() >= limit {
            report.coverage = Coverage::Capped;
            return Ok(());
        }
        let next = next.expect("checked offset");
        if next <= offset {
            return Err(GraphFailure::new(
                super::FailureKind::InvalidResponse,
                "Semantic Scholar repeated its pagination offset",
            ));
        }
        offset = next;
    }
    report.coverage = Coverage::Capped;
    Ok(())
}
fn request(segments: &[&str]) -> Request {
    Request::new(
        Provider::SemanticScholar,
        "https://api.semanticscholar.org/graph/v1/",
        segments,
    )
}
fn work(value: &Value) -> GraphResult<Work> {
    if !value.is_object() {
        return Err(GraphFailure::malformed());
    }
    let mut identifiers = Vec::new();
    if let Some(id) = text(value, "paperId") {
        identifiers.push(Identifier {
            scheme: "s2".to_owned(),
            value: id,
        });
    }
    for (field, scheme) in [("DOI", "doi"), ("ArXiv", "arxiv"), ("PubMed", "pmid")] {
        if let Some(id) = text(&value["externalIds"], field) {
            identifiers.push(Identifier {
                scheme: scheme.to_owned(),
                value: id,
            });
        }
    }
    Ok(Work {
        identifiers,
        title: text(value, "title"),
        authors: value["authors"]
            .as_array()
            .map_or_else(Vec::new, |authors| {
                authors
                    .iter()
                    .filter_map(|author| text(author, "name"))
                    .collect()
            }),
        year: year(&value["year"]),
        url: text(value, "url"),
    })
}
