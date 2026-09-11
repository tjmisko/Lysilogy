use super::*;
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};

struct Fixture {
    responses: Mutex<VecDeque<GraphResult<Value>>>,
    requests: Mutex<Vec<Request>>,
}
impl Fixture {
    fn new(responses: Vec<GraphResult<Value>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
    fn json(responses: Vec<Value>) -> Self {
        Self::new(responses.into_iter().map(Ok).collect())
    }
}
impl Transport for Fixture {
    fn get(&self, request: Request) -> GraphFuture<'_, GraphResult<Value>> {
        self.requests.lock().unwrap().push(request);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected provider request");
        Box::pin(async move { response })
    }
}
fn doi() -> Identifier {
    Identifier::parse("doi:10.1234/example").unwrap()
}
fn oa(id: &str) -> Value {
    json!({"id": format!("https://openalex.org/{id}"), "display_name":"A work", "publication_year":2020, "authorships":[{"author":{"display_name":"Full Author"}}], "referenced_works":[]})
}
fn s2(id: &str) -> Value {
    json!({"paperId":id,"title":"A paper","year":2021,"externalIds":{"DOI":"10.1234/example","ArXiv":"2101.00001v2"},"authors":[{"name":"Full Author"}]})
}

#[test]
fn parses_doi_urls_colons_and_rejects_unknown_or_whitespace_ids() {
    let raw = "10.1002/(SICI)1099-0909(199601)8:1<1::AID>3.0.CO;2-A";
    assert_eq!(Identifier::parse(raw).unwrap().value, raw);
    assert_eq!(
        Identifier::parse(&format!("https://doi.org/{raw}"))
            .unwrap()
            .value,
        raw
    );
    for value in [
        "https://private.example/paper",
        "doi:abc",
        "doi:10.1/ a",
        "arxiv:",
        "file:/secret",
    ] {
        assert!(Identifier::parse(value).is_err(), "{value}");
    }
    assert_eq!(
        Identifier::parse("arxiv:1805.00899v2").unwrap().value,
        "1805.00899v2"
    );
}

#[tokio::test]
async fn openalex_batches_references_with_metadata_and_marks_cap() {
    let mut target = oa("W1");
    target["referenced_works"] = json!(["https://openalex.org/W2", "https://openalex.org/W3"]);
    let fixture = Fixture::json(vec![target, json!({"results":[oa("W2")]})]);
    let report = OpenAlex
        .fetch(&fixture, &doi(), Direction::References, 1)
        .await;
    assert_eq!(report.coverage, Coverage::Capped);
    assert_eq!(report.total_reported, Some(2));
    assert_eq!(report.edges[0].citing.identifiers[0].value, "W1");
    assert_eq!(report.edges[0].cited.identifiers[0].value, "W2");
    assert_eq!(report.edges[0].cited.authors, ["Full Author"]);
    assert_eq!(report.requests.len(), 2);
}

#[tokio::test]
async fn openalex_paginates_incoming_and_preserves_direction() {
    let fixture = Fixture::json(vec![
        oa("W1"),
        json!({"meta":{"count":2,"next_cursor":"page+2/="},"results":[oa("W2")]}),
        json!({"meta":{"count":2,"next_cursor":null},"results":[oa("W3")]}),
    ]);
    let report = OpenAlex
        .fetch(&fixture, &doi(), Direction::Citations, 100)
        .await;
    assert_eq!(report.coverage, Coverage::Complete);
    assert_eq!(report.edges.len(), 2);
    assert_eq!(report.edges[1].citing.identifiers[0].value, "W3");
    assert_eq!(report.edges[1].cited.identifiers[0].value, "W1");
    assert!(
        fixture.requests.lock().unwrap()[2]
            .url
            .query_pairs()
            .any(|(key, value)| key == "cursor" && value == "page+2/=")
    );
}

#[tokio::test]
async fn openalex_preserves_edges_when_a_later_page_fails() {
    let fixture = Fixture::new(vec![
        Ok(oa("W1")),
        Ok(json!({"meta":{"next_cursor":"next"},"results":[oa("W2")]})),
        Err(GraphFailure::new(FailureKind::RateLimited, "HTTP 429")),
    ]);
    let report = OpenAlex
        .fetch(&fixture, &doi(), Direction::Citations, 100)
        .await;
    assert_eq!(report.coverage, Coverage::Partial);
    assert_eq!(report.edges.len(), 1);
    assert_eq!(report.failure.unwrap().kind, FailureKind::RateLimited);
}

#[tokio::test]
async fn semantic_scholar_paginates_and_retains_passages_and_model_labels() {
    let fixture = Fixture::json(vec![
        s2("target"),
        json!({"offset":0,"next":1,"data":[{"citingPaper":s2("later"),"contexts":["We extend their method."],"intents":["methodology"],"isInfluential":true}]}),
        json!({"offset":1,"data":[{"citingPaper":s2("another"),"contexts":[]}]}),
    ]);
    let report = SemanticScholar
        .fetch(
            &fixture,
            &Identifier::parse("arxiv:2101.00001v2").unwrap(),
            Direction::Citations,
            100,
        )
        .await;
    assert_eq!(report.coverage, Coverage::Complete);
    assert_eq!(report.edges.len(), 2);
    let edge = &report.edges[0];
    assert_eq!(edge.contexts, ["We extend their method."]);
    assert_eq!(edge.intents, ["methodology"]);
    assert_eq!(edge.is_influential, Some(true));
    assert_eq!(edge.citing.identifiers[0].value, "later");
    assert_eq!(edge.cited.identifiers[0].value, "target");
    assert!(
        fixture.requests.lock().unwrap()[2]
            .url
            .query_pairs()
            .any(|(key, value)| key == "offset" && value == "1")
    );
}

#[tokio::test]
async fn semantic_scholar_retains_unresolved_refs_and_caps_with_next_offset() {
    let fixture = Fixture::json(vec![
        s2("target"),
        json!({"next":1,"data":[{"citedPaper":null,"contexts":["Original reference unavailable"]}]}),
    ]);
    let report = SemanticScholar
        .fetch(&fixture, &doi(), Direction::References, 1)
        .await;
    assert_eq!(report.coverage, Coverage::Capped);
    assert!(report.edges[0].cited.identifiers.is_empty());
    assert!(report.edges[0].reference.is_some());
}

#[tokio::test]
async fn repeated_cursors_and_malformed_response_are_not_complete_empty_graphs() {
    let fixture = Fixture::json(vec![
        s2("target"),
        json!({"next":0,"data":[{"citedPaper":s2("other")}]}),
    ]);
    let report = SemanticScholar
        .fetch(&fixture, &doi(), Direction::References, 100)
        .await;
    assert_eq!(report.coverage, Coverage::Partial);
    assert_eq!(report.failure.unwrap().kind, FailureKind::InvalidResponse);
    let fixture = Fixture::json(vec![oa("W1"), json!({"error":"upstream"})]);
    let report = OpenAlex
        .fetch(&fixture, &doi(), Direction::Citations, 100)
        .await;
    assert_eq!(report.coverage, Coverage::Unavailable);
}

#[tokio::test]
async fn opencitations_keeps_multiple_pids_and_oci_provenance() {
    let entry = json!({"oci":"0610-0618","citing":"[coci] => omid:br/0610 doi:10.1234/a pmid:42","cited":"omid:br/0618 doi:10.1234/b doi:10.1234/preprint","creation":"2021-03-10","author_sc":"no"});
    let fixture = Fixture::json(vec![json!([entry])]);
    let report = OpenCitations
        .fetch(&fixture, &doi(), Direction::References, 100)
        .await;
    assert_eq!(report.coverage, Coverage::Complete);
    assert_eq!(report.edges[0].cited.identifiers.len(), 3);
    assert_eq!(
        report.edges[0].provider_edge_id.as_deref(),
        Some("0610-0618")
    );
    assert_eq!(
        report.edges[0].reference.as_ref().unwrap()["author_sc"],
        "no"
    );
}

#[tokio::test]
async fn crossref_keeps_unresolved_reference_strings_and_full_authors() {
    let fixture = Fixture::json(vec![
        json!({"message":{"title":["Target"],"author":[{"given":"Ada","family":"Lovelace"},{"name":"Research Consortium"}],"published":{"date-parts":[[2020,1,1]]},"reference":[{"key":"r1","DOI":"10.1234/target","year":"2010"},{"key":"r2","unstructured":"An unresolved bibliography entry"}]}}),
    ]);
    let report = Crossref
        .fetch(&fixture, &doi(), Direction::References, 100)
        .await;
    assert_eq!(report.coverage, Coverage::Complete);
    assert_eq!(report.edges.len(), 2);
    assert_eq!(
        report.target.unwrap().authors,
        ["Ada Lovelace", "Research Consortium"]
    );
    assert_eq!(
        report.edges[1].reference.as_ref().unwrap()["unstructured"],
        "An unresolved bibliography entry"
    );
    assert!(report.edges[1].cited.identifiers.is_empty());
}

#[tokio::test]
async fn crossref_distinguishes_no_deposit_from_empty_deposited_list() {
    for (value, expected) in [
        (
            json!({"message":{"title":["Target"]}}),
            Coverage::Unavailable,
        ),
        (json!({"message":{"reference":[]}}), Coverage::Complete),
    ] {
        let fixture = Fixture::json(vec![value]);
        assert_eq!(
            Crossref
                .fetch(&fixture, &doi(), Direction::References, 100)
                .await
                .coverage,
            expected
        );
    }
}

#[tokio::test]
async fn unsupported_capabilities_do_not_make_network_requests() {
    let fixture = Fixture::json(vec![]);
    assert_eq!(
        Crossref
            .fetch(&fixture, &doi(), Direction::Citations, 100)
            .await
            .failure
            .unwrap()
            .kind,
        FailureKind::Unsupported
    );
    let id = Identifier::parse("arxiv:1805.00899v2").unwrap();
    assert_eq!(
        OpenAlex
            .fetch(&fixture, &id, Direction::References, 100)
            .await
            .failure
            .unwrap()
            .kind,
        FailureKind::Unsupported
    );
    assert_eq!(
        OpenCitations
            .fetch(&fixture, &id, Direction::Citations, 100)
            .await
            .failure
            .unwrap()
            .kind,
        FailureKind::Unsupported
    );
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reserved_identifier_characters_cannot_create_queries_or_fragments() {
    let fixture = Fixture::json(vec![json!({"message":{"reference":[]}})]);
    let id = Identifier::parse("doi:10.1234/a?api_key=bad#fragment").unwrap();
    Crossref
        .fetch(&fixture, &id, Direction::References, 100)
        .await;
    let requests = fixture.requests.lock().unwrap();
    assert!(requests[0].url.query().is_none());
    assert!(requests[0].url.fragment().is_none());
    assert!(requests[0].url.path().contains("%3F"));
    drop(requests);
}

#[tokio::test]
async fn graph_request_validates_caps_and_deduplicates_requested_providers() {
    let fixture = Fixture::json(vec![]);
    let mut request = GraphRequest {
        identifier: "doi:10.1234/example".to_owned(),
        providers: vec![Provider::Crossref, Provider::Crossref],
        directions: vec![Direction::Citations, Direction::Citations],
        limit: 100,
    };
    let snapshot = fetch("paper", &request, &fixture).await.unwrap();
    assert_eq!(snapshot.reports.len(), 1);
    request.limit = 501;
    assert!(fetch("paper", &request, &fixture).await.is_err());
}

#[tokio::test]
async fn saved_graph_is_bounded_discovery_only_and_missing_snapshot_is_offline() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        research_candidates(directory.path())
            .await
            .unwrap()
            .is_empty()
    );
    let mut report = ProviderReport::new(Provider::Crossref, Direction::References);
    for _ in 0..30 {
        report.edges.push(CitationEdge::new(
            &Work::identified(doi()),
            Work::default(),
            Direction::References,
        ));
    }
    let snapshot = GraphSnapshot {
        schema_version: 1,
        paper_id: "paper".to_owned(),
        identifier: doi(),
        retrieved_at: Utc::now(),
        reports: vec![report],
    };
    tokio::fs::write(
        directory.path().join(SNAPSHOT_FILE),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .await
    .unwrap();
    let prompt = research_candidates(directory.path()).await.unwrap();
    assert!(prompt.contains("not proof of influence"));
    assert!(prompt.contains("Verify target identity/version"));
    assert_eq!(prompt.matches("\"provider_edge_id\"").count(), 12);
}
