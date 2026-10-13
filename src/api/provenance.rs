//! Field provenance endpoints: read every claim behind a Work's title, year,
//! authors, venue and DOI; set and clear curator overrides; review claims.
//!
//! Check order everywhere: path IDs, body, graph availability, existence.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use lysilogy_core::{FieldClaimId, ProvenanceField, WorkId};
use serde::Deserialize;
use serde_json::Value;

use super::{AppState, graph_error, graph_or_unavailable};
use crate::provenance::{ProvenanceError, parse_field, parse_review_state};

type Failure = Box<Response>;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/works/{id}/provenance", get(read_provenance))
        .route(
            "/api/works/{id}/provenance/{field}",
            put(set_override).delete(clear_override),
        )
        .route("/api/field-claims/{claim}/review", put(review_claim))
}

// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn failure(status: StatusCode, code: &str, message: &str) -> Failure {
    Box::new(graph_error(status, code, message))
}

// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn invalid_body(detail: &str) -> Failure {
    failure(
        StatusCode::BAD_REQUEST,
        "invalid_body",
        &format!("The request body is not valid: {detail}"),
    )
}

/// Parse a body that must be a JSON object with exactly the known fields.
fn parse_object<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, Failure> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_body(&error.to_string()))?;
    if !value.is_object() {
        return Err(invalid_body("expected a JSON object"));
    }
    serde_json::from_value(value).map_err(|error| invalid_body(&error.to_string()))
}

fn parse_work(text: &str) -> Result<WorkId, Failure> {
    text.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_work_id",
            "Work identifiers look like 'w' followed by 22 characters.",
        )
    })
}

fn parse_claim(text: &str) -> Result<FieldClaimId, Failure> {
    text.parse().map_err(|_| {
        failure(
            StatusCode::BAD_REQUEST,
            "invalid_claim_id",
            "Field claim identifiers look like 'a' followed by 22 characters.",
        )
    })
}

fn field_of(text: &str) -> Result<ProvenanceField, Failure> {
    parse_field(text).map_err(|error| service_failure(&error))
}

/// The HTTP form of a service error.
// Handlers short-circuit with `?`, so the failure is boxed to keep `Result` small.
#[allow(clippy::unnecessary_box_returns)]
fn service_failure(error: &ProvenanceError) -> Failure {
    let status = match error {
        ProvenanceError::NotFound(_) => StatusCode::NOT_FOUND,
        ProvenanceError::Invalid(..) => StatusCode::BAD_REQUEST,
        ProvenanceError::Conflict(_) => StatusCode::CONFLICT,
        ProvenanceError::Graph(inner) => {
            tracing::error!(error = %inner, "provenance request failed");
            return failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "graph_error",
                "The graph query failed.",
            );
        }
    };
    failure(status, error.code(), &error.to_string())
}

fn respond<T: serde::Serialize>(value: &T) -> Response {
    (StatusCode::OK, Json(value)).into_response()
}

fn finish(result: Result<Response, Failure>) -> Response {
    match result {
        Ok(response) => response,
        Err(response) => *response,
    }
}

async fn read_provenance(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    finish(read_provenance_inner(&state, &id).await)
}

async fn read_provenance_inner(state: &AppState, id: &str) -> Result<Response, Failure> {
    let work = parse_work(id)?;
    graph_or_unavailable(state)?;
    let view = state
        .work_provenance(work)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(&view))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OverrideBody {
    value: Value,
}

async fn set_override(
    State(state): State<AppState>,
    AxumPath((id, field)): AxumPath<(String, String)>,
    body: Bytes,
) -> Response {
    finish(set_override_inner(&state, &id, &field, &body).await)
}

async fn set_override_inner(
    state: &AppState,
    id: &str,
    field: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let work = parse_work(id)?;
    let field = field_of(field)?;
    let body: OverrideBody = parse_object(body)?;
    graph_or_unavailable(state)?;
    let view = state
        .set_field_override(work, field, &body.value)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(&view))
}

async fn clear_override(
    State(state): State<AppState>,
    AxumPath((id, field)): AxumPath<(String, String)>,
) -> Response {
    finish(clear_override_inner(&state, &id, &field).await)
}

async fn clear_override_inner(
    state: &AppState,
    id: &str,
    field: &str,
) -> Result<Response, Failure> {
    let work = parse_work(id)?;
    let field = field_of(field)?;
    graph_or_unavailable(state)?;
    let view = state
        .clear_field_override(work, field)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(&view))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewBody {
    review_state: String,
}

async fn review_claim(
    State(state): State<AppState>,
    AxumPath(claim): AxumPath<String>,
    body: Bytes,
) -> Response {
    finish(review_claim_inner(&state, &claim, &body).await)
}

async fn review_claim_inner(
    state: &AppState,
    claim: &str,
    body: &Bytes,
) -> Result<Response, Failure> {
    let claim = parse_claim(claim)?;
    let body: ReviewBody = parse_object(body)?;
    let review_state =
        parse_review_state(&body.review_state).map_err(|error| service_failure(&error))?;
    graph_or_unavailable(state)?;
    let view = state
        .review_field_claim(claim, review_state)
        .await
        .map_err(|error| service_failure(&error))?;
    Ok(respond(&view))
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use lysilogy_core::provenance::FieldValue;
    use lysilogy_core::{ClaimConfidence, ClaimOrigin, Coverage, IdentifierScheme};
    use lysilogy_db::claim::NewClaim;
    use lysilogy_db::evidence::NewEntry;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        api::build_router,
        expansion::fixtures::{ScriptedTransport, marker, new_work, plain_state, test_graph},
        graph::GraphState,
    };

    async fn request(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let response = build_router(state.clone(), None)
            .oneshot(
                builder
                    .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn call(state: &AppState, method: &str, uri: &str, body: &Value) -> (StatusCode, Value) {
        request(state, method, uri, Some(&body.to_string())).await
    }

    async fn bare_state() -> AppState {
        let library = tempfile::tempdir().expect("library");
        let data = tempfile::tempdir().expect("data");
        let state = AppState::new(library.path(), data.path())
            .await
            .expect("state");
        std::mem::forget((library, data));
        state
    }

    struct Fixture {
        state: AppState,
        graph: GraphState,
        marker: String,
    }

    impl Fixture {
        async fn new() -> Option<Self> {
            let graph = test_graph().await?;
            let state = plain_state(graph.clone(), ScriptedTransport::serving(Vec::new())).await;
            Some(Self {
                state,
                graph,
                marker: marker(),
            })
        }

        async fn work(&self, label: &str) -> WorkId {
            new_work(
                &self.graph,
                IdentifierScheme::Doi,
                &format!("10.9999/{}-{label}", self.marker),
            )
            .await
        }

        /// Record one claim from `origin` for a Work.
        async fn claim(
            &self,
            work: WorkId,
            value: &FieldValue,
            origin: ClaimOrigin,
            confidence: ClaimConfidence,
        ) {
            self.graph
                .claims
                .record(&[NewClaim {
                    work_id: work,
                    field: value.field(),
                    value,
                    origin,
                    extractor: "test",
                    confidence,
                    source_record_id: None,
                    entry_id: None,
                    locator: None,
                }])
                .await
                .expect("claim");
        }

        async fn get(&self, work: WorkId) -> Value {
            let (status, json) = request(
                &self.state,
                "GET",
                &format!("/api/works/{work}/provenance"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{json}");
            json
        }
    }

    fn field<'a>(view: &'a Value, name: &str) -> &'a Value {
        view["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .find(|field| field["field"] == name)
            .unwrap_or_else(|| panic!("no field {name} in {view}"))
    }

    #[tokio::test]
    async fn should_return_400_when_provenance_ids_or_field_malformed() {
        let state = bare_state().await;
        let work = WorkId::new();
        let claim = FieldClaimId::new();
        let (status, json) = request(&state, "GET", "/api/works/nope/provenance", None).await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_work_id"))
        );
        for method in ["PUT", "DELETE"] {
            let (status, json) = call(
                &state,
                method,
                &format!("/api/works/{work}/provenance/bogus"),
                &json!({"value": "x"}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(json["error"], "invalid_field", "{method}");
        }
        let (status, json) = call(
            &state,
            "PUT",
            "/api/works/bad/provenance/title",
            &json!({"value": "x"}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_work_id"))
        );
        for body in [json!(["x"]), json!({"value": "x", "extra": 1}), json!({})] {
            let (status, json) = call(
                &state,
                "PUT",
                &format!("/api/works/{work}/provenance/title"),
                &body,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json["error"], "invalid_body", "{body}");
        }
        let (status, json) = call(
            &state,
            "PUT",
            "/api/field-claims/nope/review",
            &json!({"review_state": "rejected"}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_claim_id"))
        );
        for body in [
            json!({"review_state": "disputed"}),
            json!({"review_state": "bogus"}),
        ] {
            let (status, json) = call(
                &state,
                "PUT",
                &format!("/api/field-claims/{claim}/review"),
                &body,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json["error"], "invalid_review_state", "{body}");
        }
        let (status, json) = call(
            &state,
            "PUT",
            &format!("/api/field-claims/{claim}/review"),
            &json!({"review_state": "rejected", "extra": true}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_body"))
        );
    }

    #[tokio::test]
    async fn should_return_503_when_provenance_requested_without_graph() {
        let state = bare_state().await;
        let work = WorkId::new();
        let claim = FieldClaimId::new();
        let uri = format!("/api/works/{work}/provenance");
        let field_uri = format!("{uri}/title");
        let review_uri = format!("/api/field-claims/{claim}/review");
        let attempts = [
            request(&state, "GET", &uri, None).await,
            call(&state, "PUT", &field_uri, &json!({"value": "x"})).await,
            request(&state, "DELETE", &field_uri, None).await,
            call(
                &state,
                "PUT",
                &review_uri,
                &json!({"review_state": "accepted"}),
            )
            .await,
        ];
        for (status, json) in attempts {
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{json}");
            assert_eq!(json["error"], "graph_unavailable");
        }
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // one scenario read end to end
    async fn should_return_every_field_and_claim_with_agreement_when_reading_provenance() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("read").await;
        let title = format!("Openalex Title {}", fixture.marker);
        let other = format!("Citing Title {}", fixture.marker);
        fixture
            .claim(
                work,
                &FieldValue::Title(title.clone()),
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )
            .await;
        fixture
            .claim(
                work,
                &FieldValue::Title(other.clone()),
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
            )
            .await;
        fixture
            .claim(
                work,
                &FieldValue::Year(2017),
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
            )
            .await;
        fixture
            .claim(
                work,
                &FieldValue::Authors(vec!["Ada Adams".to_owned(), "Bo Brown".to_owned()]),
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )
            .await;
        let view = fixture.get(work).await;
        assert_eq!(view["work_id"], work.to_string());
        let columns: Vec<&str> = view["columns"]
            .as_array()
            .expect("columns")
            .iter()
            .map(|column| column["key"].as_str().expect("key"))
            .collect();
        assert_eq!(
            columns,
            [
                "user",
                "publisher",
                "paper",
                "openalex",
                "library_metadata",
                "citing_bibliography"
            ]
        );
        assert_eq!(view["columns"][0]["label"], "You");
        assert_eq!(view["columns"][0]["editable"], true);
        assert_eq!(view["columns"][1]["editable"], false);
        assert_eq!(view["columns"][2]["label"], "The paper");
        let names: Vec<&str> = view["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .map(|field| field["field"].as_str().expect("field"))
            .collect();
        assert_eq!(names, ["title", "year", "authors", "venue", "doi"]);

        let title_field = field(&view, "title");
        assert_eq!(title_field["selected"]["value"], title);
        assert_eq!(title_field["selected"]["display"], title);
        assert_eq!(title_field["selected"]["state"], "known");
        assert_eq!(title_field["selected"]["origin"], "openalex");
        assert_eq!(title_field["selected"]["confidence"], "explicit");
        let claims = title_field["claims"].as_array().expect("claims");
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0]["agreement"], "selected");
        assert_eq!(claims[0]["id"], title_field["selected"]["claim_id"]);
        assert_eq!(claims[1]["agreement"], "disagrees");
        assert_eq!(claims[1]["origin"], "citing_bibliography");
        assert_eq!(claims[1]["extractor"], "test");
        assert!(claims[1]["excluded_reason"].is_null());
        assert!(claims[1]["source_record"].is_null());

        assert_eq!(field(&view, "year")["selected"]["value"], 2017);
        assert_eq!(field(&view, "year")["selected"]["origin"], "publisher");
        assert_eq!(
            field(&view, "authors")["selected"]["value"],
            json!(["Ada Adams", "Bo Brown"])
        );
        assert_eq!(
            field(&view, "authors")["selected"]["display"],
            "Ada Adams; Bo Brown"
        );
        for empty in ["venue", "doi"] {
            let empty = field(&view, empty);
            assert_eq!(empty["selected"]["state"], "unknown");
            assert!(empty["selected"]["value"].is_null());
            assert_eq!(empty["claims"], json!([]));
        }

        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/works/{}/provenance", WorkId::new()),
            None,
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("work_not_found"))
        );
    }

    #[tokio::test]
    async fn should_include_entry_and_citing_title_when_claim_came_from_bibliography() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let citing = fixture.work("citing").await;
        let cited = fixture.work("cited").await;
        let citing_title = format!("The Citing Paper {}", fixture.marker);
        fixture
            .claim(
                citing,
                &FieldValue::Title(citing_title.clone()),
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )
            .await;
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL").expect("test database");
        let pool = &lysilogy_db::connect(&url).await.expect("connect");
        let workspace: uuid::Uuid = sqlx::query_scalar(
            "insert into reader.workspace (name) values ('provenance-api-test') returning id",
        )
        .fetch_one(pool)
        .await
        .expect("workspace");
        let document: uuid::Uuid = sqlx::query_scalar(
            "insert into reader.document (workspace_id, work_id) values ($1, $2) returning id",
        )
        .bind(workspace)
        .bind(citing.as_uuid())
        .fetch_one(pool)
        .await
        .expect("document");
        let version: uuid::Uuid = sqlx::query_scalar(
            "insert into reader.document_version (document_id, sha256) values ($1, $2) returning id",
        )
        .bind(document)
        .bind(format!("sha-{}", fixture.marker))
        .fetch_one(pool)
        .await
        .expect("version");
        let version = lysilogy_core::DocumentVersionId::from_uuid(version);
        let record = fixture
            .graph
            .source_records
            .upsert(
                "local_bibliography",
                &format!("prov-api|{}", fixture.marker),
                Coverage::Complete,
                &json!({}),
            )
            .await
            .expect("record");
        let raw = format!("Ada Adams. A cited study {}. 2015.", fixture.marker);
        let ids = fixture
            .graph
            .evidence
            .insert_entries(
                version,
                record,
                &[NewEntry {
                    entry_key: "bib-v1:0".to_owned(),
                    ordinal: 0,
                    raw_text: raw.clone(),
                    printed_key: None,
                    fields: json!({}),
                    page: 1,
                    text_start: 0,
                    text_end: 1,
                    member_spans: json!([]),
                    region: None,
                }],
            )
            .await
            .expect("entries");
        let year = FieldValue::Year(2015);
        fixture
            .graph
            .claims
            .record(&[NewClaim {
                work_id: cited,
                field: year.field(),
                value: &year,
                origin: ClaimOrigin::CitingBibliography,
                extractor: "bib-v1",
                confidence: ClaimConfidence::Heuristic,
                source_record_id: Some(record),
                entry_id: Some(ids[0]),
                locator: Some("bib-v1:0"),
            }])
            .await
            .expect("claim");
        let view = fixture.get(cited).await;
        let claim = &field(&view, "year")["claims"][0];
        assert_eq!(claim["locator"], "bib-v1:0");
        assert_eq!(claim["source_record"]["kind"], "local_bibliography");
        assert_eq!(claim["entry"]["entry_key"], "bib-v1:0");
        assert_eq!(claim["entry"]["raw_text"], raw);
        assert_eq!(claim["entry"]["citing_work_id"], citing.to_string());
        assert_eq!(claim["entry"]["citing_title"], citing_title);
    }

    #[tokio::test]
    async fn should_mask_field_and_return_view_when_override_put() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("mask").await;
        fixture
            .claim(
                work,
                &FieldValue::Title(format!("Registry Title {}", fixture.marker)),
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
            )
            .await;
        let cases = [
            (
                "title",
                json!(format!("My Title {}", fixture.marker)),
                "known",
            ),
            ("year", json!(1999), "known"),
            ("authors", json!(["Zed Zeta", "Yan Yu"]), "known"),
            ("venue", json!("Journal of Tests"), "known"),
            ("doi", json!("https://doi.org/10.1234/ABC"), "known"),
        ];
        for (name, value, state) in cases {
            let (status, view) = call(
                &fixture.state,
                "PUT",
                &format!("/api/works/{work}/provenance/{name}"),
                &json!({ "value": value }),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{name}: {view}");
            let selected = &field(&view, name)["selected"];
            assert_eq!(selected["origin"], "user", "{name}");
            assert_eq!(selected["confidence"], "explicit", "{name}");
            assert_eq!(selected["state"], state, "{name}");
            let claims = field(&view, name)["claims"].as_array().expect("claims");
            let user: Vec<_> = claims
                .iter()
                .filter(|claim| claim["origin"] == "user")
                .collect();
            assert_eq!(user.len(), 1, "{name}");
            assert_eq!(user[0]["extractor"], "curator");
            assert_eq!(user[0]["agreement"], "selected");
        }
        let fresh = fixture.get(work).await;
        assert_eq!(
            field(&fresh, "title")["selected"]["value"],
            format!("My Title {}", fixture.marker)
        );
        assert_eq!(
            field(&fresh, "doi")["selected"]["value"]["value"],
            "10.1234/abc"
        );
        let row = fixture
            .graph
            .works
            .get(work)
            .await
            .expect("get")
            .expect("work");
        assert_eq!(
            row.display_title.as_deref(),
            Some(format!("My Title {}", fixture.marker).as_str())
        );
        assert_eq!(row.display_year, Some(1999));
        assert_eq!(
            row.display_authors,
            Some(vec!["Zed Zeta".to_owned(), "Yan Yu".to_owned()])
        );

        // A second override retracts the first; one stays active.
        let (status, view) = call(
            &fixture.state,
            "PUT",
            &format!("/api/works/{work}/provenance/year"),
            &json!({"value": 2001}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(field(&view, "year")["selected"]["value"], 2001);
        let retracted = field(&view, "year")["claims"]
            .as_array()
            .expect("claims")
            .iter()
            .filter(|claim| !claim["retracted_at"].is_null())
            .count();
        assert_eq!(retracted, 1);
    }

    #[tokio::test]
    async fn should_restore_tiered_value_when_override_deleted() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("restore").await;
        let registry = format!("Registry Title {}", fixture.marker);
        fixture
            .claim(
                work,
                &FieldValue::Title(registry.clone()),
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
            )
            .await;
        let uri = format!("/api/works/{work}/provenance/title");
        let (status, _) = call(&fixture.state, "PUT", &uri, &json!({"value": "Mine"})).await;
        assert_eq!(status, StatusCode::OK);
        let (status, view) = request(&fixture.state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(field(&view, "title")["selected"]["value"], registry);
        assert_eq!(field(&view, "title")["selected"]["origin"], "publisher");
        let row = fixture
            .graph
            .works
            .get(work)
            .await
            .expect("get")
            .expect("work");
        assert_eq!(row.display_title.as_deref(), Some(registry.as_str()));
        // Clearing again is a no-op that still answers with the view.
        let (status, again) = request(&fixture.state, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(field(&again, "title")["selected"]["value"], registry);
        let (status, json) = request(
            &fixture.state,
            "DELETE",
            &format!("/api/works/{}/provenance/title", WorkId::new()),
            None,
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("work_not_found"))
        );
    }

    #[tokio::test]
    async fn should_return_400_when_override_value_invalid() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("invalid").await;
        let long = "x".repeat(501);
        let many: Vec<String> = (0..101).map(|number| format!("Name {number}")).collect();
        let cases = [
            ("title", json!("")),
            ("title", json!(long)),
            ("title", json!(5)),
            ("title", json!(null)),
            ("year", json!(1399)),
            ("year", json!(2201)),
            ("year", json!("2020")),
            ("authors", json!([])),
            ("authors", json!(many)),
            ("authors", json!([""])),
            ("authors", json!("A; B")),
            ("venue", json!("   ")),
            ("doi", json!("not a doi")),
        ];
        for (name, value) in cases {
            let (status, json) = call(
                &fixture.state,
                "PUT",
                &format!("/api/works/{work}/provenance/{name}"),
                &json!({ "value": value }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{name} {value}: {json}");
            assert_eq!(json["error"], "invalid_value", "{name} {value}");
        }
        let view = fixture.get(work).await;
        for name in ["title", "year", "authors", "venue", "doi"] {
            assert_eq!(field(&view, name)["claims"], json!([]), "{name}");
        }
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // one scenario read end to end
    async fn should_exclude_claim_when_review_rejected_and_return_409_for_override() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = fixture.work("review").await;
        let publisher = format!("Registry Title {}", fixture.marker);
        let openalex = format!("Openalex Title {}", fixture.marker);
        fixture
            .claim(
                work,
                &FieldValue::Title(publisher.clone()),
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
            )
            .await;
        fixture
            .claim(
                work,
                &FieldValue::Title(openalex.clone()),
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )
            .await;
        let view = fixture.get(work).await;
        let publisher_claim = field(&view, "title")["claims"]
            .as_array()
            .expect("claims")
            .iter()
            .find(|claim| claim["origin"] == "publisher")
            .expect("publisher claim")["id"]
            .as_str()
            .expect("id")
            .to_owned();
        let uri = format!("/api/field-claims/{publisher_claim}/review");
        let (status, view) = call(
            &fixture.state,
            "PUT",
            &uri,
            &json!({"review_state": "rejected"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(field(&view, "title")["selected"]["value"], openalex);
        let rejected = field(&view, "title")["claims"]
            .as_array()
            .expect("claims")
            .iter()
            .find(|claim| claim["id"] == publisher_claim.as_str())
            .expect("claim")
            .clone();
        assert_eq!(rejected["agreement"], "excluded");
        assert_eq!(rejected["excluded_reason"], "rejected");
        assert_eq!(rejected["review_state"], "rejected");

        let (status, view) = call(
            &fixture.state,
            "PUT",
            &uri,
            &json!({"review_state": "unreviewed"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(field(&view, "title")["selected"]["value"], publisher);

        let (status, view) = call(
            &fixture.state,
            "PUT",
            &format!("/api/works/{work}/provenance/title"),
            &json!({"value": "Mine"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let user_claim = field(&view, "title")["claims"]
            .as_array()
            .expect("claims")
            .iter()
            .find(|claim| claim["origin"] == "user")
            .expect("override")["id"]
            .as_str()
            .expect("id")
            .to_owned();
        let (status, json) = call(
            &fixture.state,
            "PUT",
            &format!("/api/field-claims/{user_claim}/review"),
            &json!({"review_state": "rejected"}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::CONFLICT, Some("override_not_reviewable"))
        );
        let (status, json) = call(
            &fixture.state,
            "PUT",
            &format!("/api/field-claims/{}/review", FieldClaimId::new()),
            &json!({"review_state": "accepted"}),
        )
        .await;
        assert_eq!(
            (status, json["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("claim_not_found"))
        );
    }

    #[tokio::test]
    async fn should_follow_redirect_when_reading_provenance_of_merged_work() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let merged = fixture.work("merged").await;
        let survivor = fixture.work("survivor").await;
        let title = format!("Survivor Title {}", fixture.marker);
        fixture
            .claim(
                survivor,
                &FieldValue::Title(title.clone()),
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )
            .await;
        fixture
            .graph
            .works
            .merge_work(merged, survivor, "duplicate", "test")
            .await
            .expect("merge");
        let view = fixture.get(merged).await;
        assert_eq!(view["work_id"], survivor.to_string());
        assert_eq!(field(&view, "title")["selected"]["value"], title);
        let (status, view) = call(
            &fixture.state,
            "PUT",
            &format!("/api/works/{merged}/provenance/year"),
            &json!({"value": 2010}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["work_id"], survivor.to_string());
        assert_eq!(field(&view, "year")["selected"]["origin"], "user");
    }

    #[tokio::test]
    async fn should_include_display_authors_when_reading_neighbors() {
        let Some(fixture) = Fixture::new().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let center = fixture.work("center").await;
        let neighbor = fixture.work("neighbor").await;
        fixture
            .graph
            .citations
            .add_assertion(center, neighbor, None)
            .await
            .expect("assertion");
        fixture
            .claim(
                neighbor,
                &FieldValue::Authors(vec!["Ada Adams".to_owned(), "Bo Brown".to_owned()]),
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
            )
            .await;
        let (status, json) = request(
            &fixture.state,
            "GET",
            &format!("/api/graph/neighbors/{center}?direction=both"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let nodes = json["nodes"].as_array().expect("nodes");
        let find = |id: WorkId| {
            nodes
                .iter()
                .find(|node| node["id"] == id.to_string())
                .unwrap_or_else(|| panic!("no node {id} in {json}"))
        };
        assert_eq!(
            find(neighbor)["display_authors"],
            json!(["Ada Adams", "Bo Brown"])
        );
        assert!(find(center)["display_authors"].is_null());
    }
}
