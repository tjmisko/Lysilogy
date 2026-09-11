use super::{
    CitationEdge, CitationProvider, Coverage, Direction, GraphFailure, GraphFuture, GraphResult,
    Identifier, Provider, ProviderReport, Request, Transport, Work, array, text, year,
};
use serde_json::Value;

pub struct OpenAlex;
impl CitationProvider for OpenAlex {
    fn provider(&self) -> Provider {
        Provider::Openalex
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
        "doi" => format!("https://doi.org/{}", id.value),
        "openalex" => id.value.clone(),
        "pmid" => format!("pmid:{}", id.value),
        _ => {
            return Err(GraphFailure::unsupported(
                "OpenAlex requires DOI, PMID, or OpenAlex work ID; resolve preprint identifiers first",
            ));
        }
    };
    let value = get(transport, request(&["works", &lookup]), report).await?;
    let target = work(&value)?;
    report.target = Some(target.clone());
    if direction == Direction::References {
        return references(transport, &value, &target, limit, report).await;
    }
    let target_id = target
        .identifiers
        .iter()
        .find(|id| id.scheme == "openalex")
        .ok_or_else(GraphFailure::malformed)?;
    let mut cursor = "*".to_owned();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..20 {
        let req = request(&["works"])
            .query("filter", &format!("cites:{}", target_id.value))
            .query("sort", "-publication_date")
            .query(
                "per_page",
                &(limit - report.edges.len()).min(100).to_string(),
            )
            .query("cursor", &cursor);
        let page = get(transport, req, report).await?;
        report.total_reported = page.pointer("/meta/count").and_then(Value::as_u64);
        let entries = array(&page, "results")?;
        for entry in entries {
            let neighbor = work(entry)?;
            if seen.insert(neighbor.identifiers[0].key()) && report.edges.len() < limit {
                report
                    .edges
                    .push(CitationEdge::new(&target, neighbor, direction));
            }
        }
        let next = page.pointer("/meta/next_cursor").and_then(Value::as_str);
        if (entries.is_empty() || next.is_none())
            && report
                .total_reported
                .is_some_and(|total| total > report.edges.len() as u64)
        {
            return Err(GraphFailure::new(
                super::FailureKind::InvalidResponse,
                "OpenAlex pagination ended before its reported count",
            ));
        }
        if entries.is_empty()
            || next.is_none()
            || report
                .total_reported
                .is_some_and(|total| total == report.edges.len() as u64)
        {
            return Ok(());
        }
        if report.edges.len() >= limit {
            report.coverage = Coverage::Capped;
            return Ok(());
        }
        let next = next.expect("checked cursor");
        if next == cursor {
            return Err(GraphFailure::new(
                super::FailureKind::InvalidResponse,
                "OpenAlex repeated its pagination cursor",
            ));
        }
        next.clone_into(&mut cursor);
    }
    report.coverage = Coverage::Capped;
    Ok(())
}
async fn references(
    transport: &dyn Transport,
    value: &Value,
    target: &Work,
    limit: usize,
    report: &mut ProviderReport,
) -> GraphResult<()> {
    let refs = array(value, "referenced_works")?;
    report.total_reported = Some(refs.len() as u64);
    report.coverage = if refs.len() > limit {
        Coverage::Capped
    } else {
        Coverage::Complete
    };
    // The complete outgoing edge list comes from the target; metadata enrichment is batched.
    for reference in refs.iter().take(limit) {
        let id = reference
            .as_str()
            .and_then(openalex_id)
            .ok_or_else(GraphFailure::malformed)?;
        report.edges.push(CitationEdge::new(
            target,
            Work::identified(id),
            Direction::References,
        ));
    }
    for chunk in report.edges.chunks_mut(100) {
        let ids = chunk
            .iter()
            .map(|edge| edge.cited.identifiers[0].value.as_str())
            .collect::<Vec<_>>()
            .join("|");
        let req = request(&["works"])
            .query("filter", &format!("openalex:{ids}"))
            .query("per_page", "100");
        report.requests.push(req.url.to_string());
        let metadata = transport.get(req).await?;
        for entry in array(&metadata, "results")? {
            let enriched = work(entry)?;
            if let Some(edge) = chunk.iter_mut().find(|edge| {
                edge.cited
                    .identifiers
                    .iter()
                    .any(|id| enriched.identifiers.contains(id))
            }) {
                edge.cited = enriched;
            }
        }
    }
    Ok(())
}
fn request(segments: &[&str]) -> Request {
    Request::new(Provider::Openalex, "https://api.openalex.org/", segments)
}
fn openalex_id(value: &str) -> Option<Identifier> {
    let value = value.strip_prefix("https://openalex.org/").unwrap_or(value);
    value.starts_with('W').then(|| Identifier {
        scheme: "openalex".to_owned(),
        value: value.to_owned(),
    })
}
fn work(value: &Value) -> GraphResult<Work> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .and_then(openalex_id)
        .ok_or_else(GraphFailure::malformed)?;
    let mut result = Work::identified(id);
    if let Some(doi) = text(value, "doi").and_then(|doi| Identifier::parse(&doi).ok()) {
        result.identifiers.push(doi);
    }
    result.title = text(value, "display_name").or_else(|| text(value, "title"));
    result.year = year(&value["publication_year"]);
    result.authors = value["authorships"]
        .as_array()
        .map_or_else(Vec::new, |authors| {
            authors
                .iter()
                .filter_map(|a| text(&a["author"], "display_name"))
                .collect()
        });
    Ok(result)
}
