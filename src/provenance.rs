//! Field provenance for Works: which claim supplies each displayed field.
//!
//! [`AppState::work_provenance`] lays out, for one Work, every tracked claim
//! by field and source tier together with the value selection picks.
//! [`ProvenanceError`] carries stable codes shared by the API and the CLI.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use lysilogy_core::provenance::{
    ClaimAgreement, ClaimCandidate, ExclusionReason, FieldValue, ORIGIN_TRUST_ORDER,
    PROVENANCE_FIELDS, parse_override, select_field,
};
use lysilogy_core::{
    ClaimConfidence, ClaimOrigin, EntryId, FieldClaimId, FieldState, ProvenanceField, ReviewState,
    SourceRecordId, Work, WorkId,
};
use lysilogy_db::DbError;
use lysilogy_db::claim::{EntryBrief, SourceRecordBrief, StoredClaim};
use serde::Serialize;
use serde_json::{Value, json};

use crate::graph::GraphState;
use crate::{AppState, Error, Result};

/// Why a provenance operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ProvenanceError {
    /// `work_not_found` or `claim_not_found`.
    #[error("{}", not_found_message(.0))]
    NotFound(&'static str),
    /// The input was refused: a code such as `invalid_value` and a message.
    #[error("{1}")]
    Invalid(&'static str, String),
    /// `override_not_reviewable`.
    #[error("{}", conflict_message(.0))]
    Conflict(&'static str),
    /// The graph database is missing or a query failed.
    #[error("{0}")]
    Graph(Error),
}

fn not_found_message(code: &str) -> &'static str {
    if code == "claim_not_found" {
        "No such field claim."
    } else {
        "No such work."
    }
}

fn conflict_message(code: &str) -> &'static str {
    if code == "override_not_reviewable" {
        "A curator override is cleared, not reviewed."
    } else {
        "The change conflicts with the claim."
    }
}

impl ProvenanceError {
    /// The machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound(code) | Self::Invalid(code, _) | Self::Conflict(code) => code,
            Self::Graph(_) => "graph_error",
        }
    }
}

impl From<ProvenanceError> for Error {
    fn from(error: ProvenanceError) -> Self {
        match error {
            ProvenanceError::Graph(error) => error,
            other => Self::InvalidRequest(other.to_string()),
        }
    }
}

fn database(error: &DbError) -> ProvenanceError {
    ProvenanceError::Graph(Error::Literature(error.to_string()))
}

fn graph_of(state: &AppState) -> std::result::Result<&GraphState, ProvenanceError> {
    state.graph.as_ref().ok_or_else(|| {
        ProvenanceError::Graph(Error::InvalidRequest(
            "provenance needs a database; set LYSILOGY_DATABASE_URL".to_owned(),
        ))
    })
}

/// Parse an API or CLI field name.
///
/// # Errors
/// `Invalid("invalid_field", …)`.
pub fn parse_field(text: &str) -> std::result::Result<ProvenanceField, ProvenanceError> {
    text.parse().map_err(|_| {
        ProvenanceError::Invalid(
            "invalid_field",
            "The field must be one of title, year, authors, venue, doi.".to_owned(),
        )
    })
}

/// Parse a review state a curator may set.
///
/// # Errors
/// `Invalid("invalid_review_state", …)`.
pub fn parse_review_state(text: &str) -> std::result::Result<ReviewState, ProvenanceError> {
    match text {
        "accepted" => Ok(ReviewState::Accepted),
        "rejected" => Ok(ReviewState::Rejected),
        "unreviewed" => Ok(ReviewState::Unreviewed),
        _ => Err(ProvenanceError::Invalid(
            "invalid_review_state",
            "The review state must be accepted, rejected, or unreviewed.".to_owned(),
        )),
    }
}

/// The JSON override value for command-line text: years are integers, authors
/// split on `;`, everything else is the text as given.
///
/// # Errors
/// `Invalid("invalid_value", …)` when a year is not an integer.
pub fn cli_override_value(
    field: ProvenanceField,
    text: &str,
) -> std::result::Result<Value, ProvenanceError> {
    match field {
        ProvenanceField::Year => text
            .trim()
            .parse::<i64>()
            .map(|year| json!(year))
            .map_err(|_| {
                ProvenanceError::Invalid("invalid_value", "The year must be an integer.".to_owned())
            }),
        ProvenanceField::Authors => Ok(json!(
            text.split(';')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        )),
        ProvenanceField::Title | ProvenanceField::Venue | ProvenanceField::Doi => Ok(json!(text)),
    }
}

/// One source tier, in trust order.
#[derive(Clone, Debug, Serialize)]
pub struct ColumnView {
    pub key: ClaimOrigin,
    pub label: &'static str,
    /// Only the curator's own column can be edited.
    pub editable: bool,
}

/// The selected value of one field.
#[derive(Clone, Debug, Serialize)]
pub struct SelectedView {
    pub value: Option<Value>,
    pub display: Option<String>,
    pub state: FieldState,
    pub claim_id: Option<FieldClaimId>,
    pub origin: Option<ClaimOrigin>,
    pub confidence: Option<ClaimConfidence>,
}

/// A source record, for display.
#[derive(Clone, Debug, Serialize)]
pub struct SourceRecordView {
    pub id: SourceRecordId,
    pub kind: String,
    pub source_key: String,
    pub retrieved_at: DateTime<Utc>,
}

/// A bibliography entry a claim was read from.
#[derive(Clone, Debug, Serialize)]
pub struct EntryView {
    pub id: EntryId,
    pub entry_key: String,
    pub raw_text: String,
    pub citing_work_id: Option<WorkId>,
    pub citing_title: Option<String>,
}

/// One claim with how it relates to the selected value.
#[derive(Clone, Debug, Serialize)]
pub struct ClaimView {
    pub id: FieldClaimId,
    pub origin: ClaimOrigin,
    pub extractor: String,
    pub confidence: ClaimConfidence,
    pub value: Value,
    /// `None` when the stored value does not read as the field's type.
    pub display: Option<String>,
    pub review_state: ReviewState,
    pub retracted_at: Option<DateTime<Utc>>,
    /// `selected`, `agrees`, `disagrees` or `excluded`.
    pub agreement: &'static str,
    pub excluded_reason: Option<ExclusionReason>,
    pub source_record: Option<SourceRecordView>,
    pub locator: Option<String>,
    pub entry: Option<EntryView>,
    pub created_at: DateTime<Utc>,
}

/// One field: the selected value and every claim, oldest first.
#[derive(Clone, Debug, Serialize)]
pub struct FieldProvenance {
    pub field: ProvenanceField,
    pub selected: SelectedView,
    pub claims: Vec<ClaimView>,
}

/// Everything provenance knows about one Work.
#[derive(Clone, Debug, Serialize)]
pub struct ProvenanceView {
    pub work_id: WorkId,
    pub columns: Vec<ColumnView>,
    pub fields: Vec<FieldProvenance>,
}

const fn column_label(origin: ClaimOrigin) -> &'static str {
    match origin {
        ClaimOrigin::User => "You",
        ClaimOrigin::Publisher => "Publisher",
        ClaimOrigin::Paper => "The paper",
        ClaimOrigin::Openalex => "OpenAlex",
        ClaimOrigin::LibraryMetadata => "Library file",
        ClaimOrigin::CitingBibliography => "Citing papers",
    }
}

fn columns() -> Vec<ColumnView> {
    ORIGIN_TRUST_ORDER
        .iter()
        .map(|origin| ColumnView {
            key: *origin,
            label: column_label(*origin),
            editable: *origin == ClaimOrigin::User,
        })
        .collect()
}

fn candidate(claim: &StoredClaim) -> ClaimCandidate {
    ClaimCandidate {
        id: claim.id,
        origin: claim.origin,
        confidence: claim.confidence,
        value: FieldValue::parse(claim.field, &claim.value),
        review_state: claim.review_state,
        retracted: claim.retracted_at.is_some(),
        created_at: claim.created_at,
    }
}

/// Source records and entries behind a set of claims.
struct Lookups {
    records: HashMap<SourceRecordId, SourceRecordBrief>,
    entries: HashMap<EntryId, EntryBrief>,
    titles: HashMap<WorkId, Option<String>>,
}

async fn lookups(
    graph: &GraphState,
    claims: &[StoredClaim],
) -> std::result::Result<Lookups, ProvenanceError> {
    let mut record_ids: Vec<SourceRecordId> = claims
        .iter()
        .filter_map(|claim| claim.source_record_id)
        .collect();
    record_ids.sort();
    record_ids.dedup();
    let mut entry_ids: Vec<EntryId> = claims.iter().filter_map(|claim| claim.entry_id).collect();
    entry_ids.sort();
    entry_ids.dedup();
    let records = graph
        .claims
        .source_records(&record_ids)
        .await
        .map_err(|error| database(&error))?;
    let entries = graph
        .claims
        .entries(&entry_ids)
        .await
        .map_err(|error| database(&error))?;
    let mut titles = HashMap::new();
    for entry in &entries {
        let Some(citing) = entry.citing_work_id else {
            continue;
        };
        if titles.contains_key(&citing) {
            continue;
        }
        let work = graph
            .works
            .get(citing)
            .await
            .map_err(|error| database(&error))?;
        titles.insert(citing, work.and_then(|work| work.display_title));
    }
    Ok(Lookups {
        records: records.into_iter().map(|brief| (brief.id, brief)).collect(),
        entries: entries.into_iter().map(|brief| (brief.id, brief)).collect(),
        titles,
    })
}

fn claim_view(
    claim: &StoredClaim,
    agreement: Option<ClaimAgreement>,
    lookups: &Lookups,
) -> ClaimView {
    let (label, reason) = match agreement {
        Some(ClaimAgreement::Selected) => ("selected", None),
        Some(ClaimAgreement::Agrees) => ("agrees", None),
        Some(ClaimAgreement::Disagrees) | None => ("disagrees", None),
        Some(ClaimAgreement::Excluded(reason)) => ("excluded", Some(reason)),
    };
    ClaimView {
        id: claim.id,
        origin: claim.origin,
        extractor: claim.extractor.clone(),
        confidence: claim.confidence,
        value: claim.value.clone(),
        display: FieldValue::parse(claim.field, &claim.value).map(|value| value.display()),
        review_state: claim.review_state,
        retracted_at: claim.retracted_at,
        agreement: label,
        excluded_reason: reason,
        source_record: claim
            .source_record_id
            .and_then(|id| lookups.records.get(&id))
            .map(|brief| SourceRecordView {
                id: brief.id,
                kind: brief.kind.clone(),
                source_key: brief.source_key.clone(),
                retrieved_at: brief.retrieved_at,
            }),
        locator: claim.locator.clone(),
        entry: claim
            .entry_id
            .and_then(|id| lookups.entries.get(&id))
            .map(|brief| EntryView {
                id: brief.id,
                entry_key: brief.entry_key.clone(),
                raw_text: brief.raw_text.clone(),
                citing_work_id: brief.citing_work_id,
                citing_title: brief
                    .citing_work_id
                    .and_then(|work| lookups.titles.get(&work).cloned().flatten()),
            }),
        created_at: claim.created_at,
    }
}

fn field_provenance(
    field: ProvenanceField,
    claims: &[&StoredClaim],
    lookups: &Lookups,
) -> FieldProvenance {
    let candidates: Vec<ClaimCandidate> = claims.iter().map(|claim| candidate(claim)).collect();
    let selection = select_field(field, &candidates);
    let winner = selection
        .winner
        .and_then(|id| claims.iter().find(|claim| claim.id == id));
    FieldProvenance {
        field,
        selected: SelectedView {
            value: selection.value.as_ref().map(FieldValue::to_json),
            display: selection.value.as_ref().map(FieldValue::display),
            state: selection.state,
            claim_id: selection.winner,
            origin: winner.map(|claim| claim.origin),
            confidence: winner.map(|claim| claim.confidence),
        },
        claims: claims
            .iter()
            .map(|claim| claim_view(claim, selection.agreement.get(&claim.id).copied(), lookups))
            .collect(),
    }
}

async fn existing_work(
    graph: &GraphState,
    work: WorkId,
) -> std::result::Result<Work, ProvenanceError> {
    let resolved = graph
        .works
        .resolve_redirect(work)
        .await
        .map_err(|error| database(&error))?;
    graph
        .works
        .get(resolved)
        .await
        .map_err(|error| database(&error))?
        .ok_or(ProvenanceError::NotFound("work_not_found"))
}

async fn view_of(
    graph: &GraphState,
    work: WorkId,
) -> std::result::Result<ProvenanceView, ProvenanceError> {
    let claims = graph
        .claims
        .claims_for(work)
        .await
        .map_err(|error| database(&error))?;
    let lookups = lookups(graph, &claims).await?;
    let mut by_field: BTreeMap<usize, Vec<&StoredClaim>> = BTreeMap::new();
    for claim in &claims {
        if let Some(position) = PROVENANCE_FIELDS
            .iter()
            .position(|field| *field == claim.field)
        {
            by_field.entry(position).or_default().push(claim);
        }
    }
    let fields = PROVENANCE_FIELDS
        .iter()
        .enumerate()
        .map(|(position, field)| {
            let members = by_field.remove(&position).unwrap_or_default();
            field_provenance(*field, &members, &lookups)
        })
        .collect();
    Ok(ProvenanceView {
        work_id: work,
        columns: columns(),
        fields,
    })
}

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

impl AppState {
    /// Every tracked claim for the Work (redirects followed), by field and
    /// source tier, with the value selection picks. Read-only: selection is
    /// computed on read.
    ///
    /// # Errors
    /// `NotFound("work_not_found")`, or `Graph` without a database.
    pub async fn work_provenance(
        &self,
        work: WorkId,
    ) -> std::result::Result<ProvenanceView, ProvenanceError> {
        let graph = graph_of(self)?;
        let work = existing_work(graph, work).await?;
        view_of(graph, work.id).await
    }

    /// Set the curator override for a field and return the new view.
    ///
    /// # Errors
    /// `NotFound`, `Invalid("invalid_value", …)`, or `Graph`.
    pub async fn set_field_override(
        &self,
        work: WorkId,
        field: ProvenanceField,
        value: &Value,
    ) -> std::result::Result<ProvenanceView, ProvenanceError> {
        let graph = graph_of(self)?;
        let work = existing_work(graph, work).await?;
        let parsed = parse_override(field, value)
            .map_err(|error| ProvenanceError::Invalid(error.code(), error.to_string()))?;
        graph
            .claims
            .set_override(work.id, field, &parsed)
            .await
            .map_err(|error| database(&error))?;
        view_of(graph, work.id).await
    }

    /// Clear the curator override for a field (a no-op without one) and
    /// return the new view.
    ///
    /// # Errors
    /// `NotFound` or `Graph`.
    pub async fn clear_field_override(
        &self,
        work: WorkId,
        field: ProvenanceField,
    ) -> std::result::Result<ProvenanceView, ProvenanceError> {
        let graph = graph_of(self)?;
        let work = existing_work(graph, work).await?;
        graph
            .claims
            .clear_override(work.id, field)
            .await
            .map_err(|error| database(&error))?;
        view_of(graph, work.id).await
    }

    /// Accept, reject or reset a claim and return its Work's view.
    ///
    /// # Errors
    /// `NotFound("claim_not_found")`, `Conflict("override_not_reviewable")`,
    /// or `Graph`.
    pub async fn review_field_claim(
        &self,
        claim: FieldClaimId,
        state: ReviewState,
    ) -> std::result::Result<ProvenanceView, ProvenanceError> {
        let graph = graph_of(self)?;
        let reviewed = match graph.claims.set_review_state(claim, state).await {
            Ok(Some(reviewed)) => reviewed,
            Ok(None) => return Err(ProvenanceError::NotFound("claim_not_found")),
            Err(DbError::Conflict(code)) if code == "override_not_reviewable" => {
                return Err(ProvenanceError::Conflict("override_not_reviewable"));
            }
            Err(error) => return Err(database(&error)),
        };
        let work = existing_work(graph, reviewed.work_id).await?;
        view_of(graph, work.id).await
    }
}

/// Render a view for the terminal.
#[must_use]
pub fn render_provenance(view: &ProvenanceView) -> String {
    let mut output = String::new();
    for field in &view.fields {
        let selected = &field.selected;
        let display = selected.display.as_deref().unwrap_or("(none)");
        let via = match (selected.origin, selected.confidence) {
            (Some(origin), Some(confidence)) => format!(" via {origin}/{confidence}"),
            _ => String::new(),
        };
        let _ = writeln!(
            output,
            "{}  {display} [{}]{via}",
            field.field, selected.state
        );
        for claim in &field.claims {
            let agreement = claim.excluded_reason.map_or_else(
                || claim.agreement.to_owned(),
                |reason| format!("excluded:{}", reason_text(reason)),
            );
            let mut source = Vec::new();
            if let Some(record) = &claim.source_record {
                source.push(record.kind.clone());
            }
            if let Some(locator) = &claim.locator {
                source.push(locator.clone());
            }
            if let Some(title) = claim
                .entry
                .as_ref()
                .and_then(|entry| entry.citing_title.as_deref())
            {
                source.push(format!("cited by {title}"));
            }
            let source = if source.is_empty() {
                String::new()
            } else {
                format!("  ({})", source.join(", "))
            };
            let _ = writeln!(
                output,
                "  {agreement}  {}/{} {}  {}{source}",
                claim.origin,
                claim.confidence,
                claim.extractor,
                claim.display.as_deref().unwrap_or("(unreadable)"),
            );
        }
    }
    output
}

const fn reason_text(reason: ExclusionReason) -> &'static str {
    match reason {
        ExclusionReason::Rejected => "rejected",
        ExclusionReason::Retracted => "retracted",
        ExclusionReason::Unreadable => "unreadable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_split_authors_on_semicolons_when_building_cli_override() {
        let value = cli_override_value(ProvenanceField::Authors, " A. One ;B. Two;; ").unwrap();
        assert_eq!(value, json!(["A. One", "B. Two"]));
    }

    #[test]
    fn should_parse_year_as_integer_when_building_cli_override() {
        assert_eq!(
            cli_override_value(ProvenanceField::Year, " 2017 ").unwrap(),
            json!(2017)
        );
        let error = cli_override_value(ProvenanceField::Year, "2017a").unwrap_err();
        assert_eq!(error.code(), "invalid_value");
    }

    #[test]
    fn should_keep_text_as_given_when_building_title_override() {
        assert_eq!(
            cli_override_value(ProvenanceField::Title, "A; B").unwrap(),
            json!("A; B")
        );
    }

    #[test]
    fn should_refuse_disputed_when_parsing_review_state() {
        assert!(parse_review_state("accepted").is_ok());
        assert_eq!(
            parse_review_state("disputed").unwrap_err().code(),
            "invalid_review_state"
        );
        assert_eq!(parse_field("authors").unwrap(), ProvenanceField::Authors);
        assert_eq!(parse_field("author").unwrap_err().code(), "invalid_field");
    }
}
