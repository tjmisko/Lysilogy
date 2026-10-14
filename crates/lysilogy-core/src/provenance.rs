//! Pure trust-tiered selection of a Work's bibliographic fields from claims.
//!
//! Claims are append-only evidence. Which claim supplies a Work's displayed
//! title, year or authors is decided here, never in the database: the winning
//! tier is the set of eligible claims with the best [`SelectionRank`], and the
//! field is `conflicting` only when claims inside that tier disagree.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use crate::id::FieldClaimId;
use crate::identifier;
use crate::names::citation_family;
use crate::state::{
    ClaimConfidence, ClaimOrigin, FieldKind, FieldState, IdentifierScheme, ProvenanceField,
    ReviewState,
};
use crate::title::title_key;

/// Origins from most to least trusted.
pub const ORIGIN_TRUST_ORDER: [ClaimOrigin; 6] = [
    ClaimOrigin::User,
    ClaimOrigin::Publisher,
    ClaimOrigin::Paper,
    ClaimOrigin::Openalex,
    ClaimOrigin::LibraryMetadata,
    ClaimOrigin::CitingBibliography,
];

/// Every tracked field in display order.
pub const PROVENANCE_FIELDS: [ProvenanceField; 5] = [
    ProvenanceField::Title,
    ProvenanceField::Year,
    ProvenanceField::Authors,
    ProvenanceField::Venue,
    ProvenanceField::Doi,
];

const TITLE_CHARS_MAX: usize = 500;
const AUTHORS_MAX: usize = 100;
const AUTHOR_CHARS_MAX: usize = 200;
const YEAR_MIN: i64 = 1400;
const YEAR_MAX: i64 = 2200;

impl ProvenanceField {
    /// The `graph.field_claim.field` kind that stores this field.
    #[must_use]
    pub const fn field_kind(self) -> FieldKind {
        match self {
            Self::Title => FieldKind::Title,
            Self::Year => FieldKind::Year,
            Self::Authors => FieldKind::Author,
            Self::Venue => FieldKind::Venue,
            Self::Doi => FieldKind::Identifier,
        }
    }

    /// Whether the selected value is written onto the Work row.
    #[must_use]
    pub const fn selected_on_work(self) -> bool {
        matches!(self, Self::Title | Self::Year | Self::Authors)
    }
}

/// Where a claim sits in the trust order; lower wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SelectionRank {
    pub origin: u8,
    pub confidence: u8,
}

/// The rank of a claim from its origin and confidence.
#[must_use]
pub const fn selection_rank(origin: ClaimOrigin, confidence: ClaimConfidence) -> SelectionRank {
    let origin = match origin {
        ClaimOrigin::User => 0,
        ClaimOrigin::Publisher => 1,
        ClaimOrigin::Paper => 2,
        ClaimOrigin::Openalex => 3,
        ClaimOrigin::LibraryMetadata => 4,
        ClaimOrigin::CitingBibliography => 5,
    };
    let confidence = match confidence {
        ClaimConfidence::Explicit => 0,
        ClaimConfidence::Heuristic => 1,
        ClaimConfidence::Model => 2,
    };
    SelectionRank { origin, confidence }
}

/// A parsed claim value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Title(String),
    Year(i32),
    Authors(Vec<String>),
    Venue(String),
    Doi(String),
}

impl FieldValue {
    /// Parse a stored claim value for a field; `None` when it does not match.
    #[must_use]
    pub fn parse(field: ProvenanceField, value: &Value) -> Option<Self> {
        match field {
            ProvenanceField::Title => nonempty_string(value).map(Self::Title),
            ProvenanceField::Venue => nonempty_string(value).map(Self::Venue),
            ProvenanceField::Year => {
                let year = value.as_i64()?;
                i32::try_from(year).ok().map(Self::Year)
            }
            ProvenanceField::Authors => {
                let names = value
                    .as_array()?
                    .iter()
                    .map(nonempty_string)
                    .collect::<Option<Vec<_>>>()?;
                if names.is_empty() {
                    return None;
                }
                Some(Self::Authors(names))
            }
            ProvenanceField::Doi => {
                let object = value.as_object()?;
                if object.get("scheme")?.as_str()? != "doi" {
                    return None;
                }
                let doi =
                    identifier::normalize(IdentifierScheme::Doi, object.get("value")?.as_str()?)?;
                Some(Self::Doi(doi))
            }
        }
    }

    /// The claim JSON for this value (inverse of [`FieldValue::parse`]).
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Title(text) | Self::Venue(text) => json!(text),
            Self::Year(year) => json!(year),
            Self::Authors(names) => json!(names),
            Self::Doi(doi) => json!({ "scheme": "doi", "value": doi }),
        }
    }

    /// Human-readable text; authors are joined with `"; "`.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Title(text) | Self::Venue(text) | Self::Doi(text) => text.clone(),
            Self::Year(year) => year.to_string(),
            Self::Authors(names) => names.join("; "),
        }
    }

    /// The field this value belongs to.
    #[must_use]
    pub const fn field(&self) -> ProvenanceField {
        match self {
            Self::Title(_) => ProvenanceField::Title,
            Self::Year(_) => ProvenanceField::Year,
            Self::Authors(_) => ProvenanceField::Authors,
            Self::Venue(_) => ProvenanceField::Venue,
            Self::Doi(_) => ProvenanceField::Doi,
        }
    }
}

fn nonempty_string(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_owned())
}

/// Normalized form of a printed author list for comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorListKey {
    /// Family-name keys in printed order.
    pub families: Vec<String>,
    /// The list ended in "et al." or similar, so more authors exist.
    pub truncated: bool,
}

const TRUNCATION_MARKERS: [&str; 3] = ["and others", "et al", "others"];

/// Strip trailing truncation markers from one printed author.
fn strip_truncation(author: &str) -> (String, bool) {
    let mut text = author.trim().to_owned();
    let mut truncated = false;
    loop {
        let lower = text.to_ascii_lowercase();
        let probe = lower.trim_end_matches(|c: char| c == '.' || c == ',' || c.is_whitespace());
        let cut = TRUNCATION_MARKERS.iter().find_map(|marker| {
            let start = probe.strip_suffix(marker)?.len();
            let boundary = probe[..start]
                .chars()
                .next_back()
                .is_none_or(|c| c == ',' || c.is_whitespace());
            boundary.then_some(start)
        });
        let Some(start) = cut else {
            return (text, truncated);
        };
        text.truncate(start);
        text = text
            .trim_end_matches(|c: char| c == ',' || c.is_whitespace())
            .to_string();
        truncated = true;
    }
}

/// Reduce a printed author list to ordered family-name keys.
#[must_use]
pub fn author_list_key(authors: &[String]) -> AuthorListKey {
    let mut families = Vec::with_capacity(authors.len());
    let mut truncated = false;
    for author in authors {
        let (name, cut) = strip_truncation(author);
        truncated |= cut;
        if name.is_empty() {
            continue;
        }
        let family = citation_family(&name).unwrap_or_else(|| name.clone());
        families.push(title_key(&family));
    }
    AuthorListKey {
        families,
        truncated,
    }
}

fn authors_agree(left: &[String], right: &[String]) -> bool {
    let left = author_list_key(left);
    let right = author_list_key(right);
    if left.families.is_empty() || right.families.is_empty() {
        return false;
    }
    if left.families == right.families {
        return true;
    }
    let prefix_of = |short: &AuthorListKey, long: &AuthorListKey| {
        short.truncated && long.families.starts_with(&short.families)
    };
    prefix_of(&left, &right) || prefix_of(&right, &left)
}

/// Whether two values of the same field say the same thing after normalizing.
#[must_use]
pub fn values_agree(left: &FieldValue, right: &FieldValue) -> bool {
    match (left, right) {
        (FieldValue::Title(a), FieldValue::Title(b))
        | (FieldValue::Venue(a), FieldValue::Venue(b)) => {
            let key = title_key(a);
            !key.is_empty() && key == title_key(b)
        }
        (FieldValue::Year(a), FieldValue::Year(b)) => a == b,
        (FieldValue::Doi(a), FieldValue::Doi(b)) => a == b,
        (FieldValue::Authors(a), FieldValue::Authors(b)) => authors_agree(a, b),
        _ => false,
    }
}

/// One claim as seen by selection.
#[derive(Clone, Debug)]
pub struct ClaimCandidate {
    pub id: FieldClaimId,
    pub origin: ClaimOrigin,
    pub confidence: ClaimConfidence,
    /// `None` when the stored JSON does not parse for the field.
    pub value: Option<FieldValue>,
    pub review_state: ReviewState,
    pub retracted: bool,
    pub created_at: DateTime<Utc>,
}

/// Why a claim took no part in selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    Rejected,
    Retracted,
    Unreadable,
}

/// How a claim relates to the selected value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "agreement", content = "reason", rename_all = "snake_case")]
pub enum ClaimAgreement {
    Selected,
    Agrees,
    Disagrees,
    Excluded(ExclusionReason),
}

/// The outcome of selecting one field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSelection {
    pub field: ProvenanceField,
    pub state: FieldState,
    pub value: Option<FieldValue>,
    pub winner: Option<FieldClaimId>,
    pub rank: Option<SelectionRank>,
    /// An entry for every candidate given.
    pub agreement: BTreeMap<FieldClaimId, ClaimAgreement>,
}

fn exclusion(candidate: &ClaimCandidate) -> Option<ExclusionReason> {
    if candidate.review_state == ReviewState::Rejected {
        return Some(ExclusionReason::Rejected);
    }
    if candidate.retracted {
        return Some(ExclusionReason::Retracted);
    }
    if candidate.value.is_none() {
        return Some(ExclusionReason::Unreadable);
    }
    None
}

/// Completeness of a value for tie-breaking: author count, -1 when truncated.
fn completeness(value: &FieldValue) -> i64 {
    let FieldValue::Authors(names) = value else {
        return 0;
    };
    let key = author_list_key(names);
    if key.truncated {
        return -1;
    }
    i64::try_from(key.families.len()).unwrap_or(i64::MAX)
}

/// Select the value of one field from its claims.
#[must_use]
pub fn select_field(field: ProvenanceField, candidates: &[ClaimCandidate]) -> FieldSelection {
    let mut agreement = BTreeMap::new();
    let mut eligible: Vec<(&ClaimCandidate, &FieldValue)> = Vec::new();
    for candidate in candidates {
        if let Some(reason) = exclusion(candidate) {
            agreement.insert(candidate.id, ClaimAgreement::Excluded(reason));
            continue;
        }
        if let Some(value) = candidate.value.as_ref() {
            eligible.push((candidate, value));
        }
    }
    let best_rank = eligible
        .iter()
        .map(|(candidate, _)| selection_rank(candidate.origin, candidate.confidence))
        .min();
    let Some(best_rank) = best_rank else {
        return FieldSelection {
            field,
            state: FieldState::Unknown,
            value: None,
            winner: None,
            rank: None,
            agreement,
        };
    };
    let tier: Vec<(&ClaimCandidate, &FieldValue)> = eligible
        .iter()
        .copied()
        .filter(|(candidate, _)| {
            selection_rank(candidate.origin, candidate.confidence) == best_rank
        })
        .collect();
    let conflicting = tier.iter().enumerate().any(|(index, (_, left))| {
        tier[index + 1..]
            .iter()
            .any(|(_, right)| !values_agree(left, right))
    });
    let support = |index: usize| -> usize {
        tier.iter()
            .enumerate()
            .filter(|(other, (_, value))| *other == index || values_agree(tier[index].1, value))
            .count()
    };
    let mut ordered: Vec<usize> = (0..tier.len()).collect();
    ordered.sort_by(|&left, &right| {
        let (left_claim, left_value) = tier[left];
        let (right_claim, right_value) = tier[right];
        support(right)
            .cmp(&support(left))
            .then_with(|| completeness(right_value).cmp(&completeness(left_value)))
            .then_with(|| left_claim.created_at.cmp(&right_claim.created_at))
            .then_with(|| left_claim.id.cmp(&right_claim.id))
    });
    let (winner, winner_value) = tier[ordered[0]];
    for (candidate, value) in &eligible {
        let relation = if candidate.id == winner.id {
            ClaimAgreement::Selected
        } else if values_agree(winner_value, value) {
            ClaimAgreement::Agrees
        } else {
            ClaimAgreement::Disagrees
        };
        agreement.insert(candidate.id, relation);
    }
    FieldSelection {
        field,
        state: if conflicting {
            FieldState::Conflicting
        } else {
            FieldState::Known
        },
        value: Some(winner_value.clone()),
        winner: Some(winner.id),
        rank: Some(best_rank),
        agreement,
    }
}

/// A rejected curator override.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OverrideError {
    #[error("invalid override value: {0}")]
    InvalidValue(&'static str),
}

impl OverrideError {
    /// The stable API error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "invalid_value"
    }
}

/// Trim, reject control characters, and collapse whitespace; `None` when invalid.
fn clean_text(raw: &str, chars_max: usize) -> Result<String, &'static str> {
    if raw.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err("contains control characters");
    }
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return Err("must not be empty");
    }
    if collapsed.chars().count() > chars_max {
        return Err("is too long");
    }
    Ok(collapsed)
}

/// Validate a curator-supplied value for a field.
///
/// # Errors
///
/// Returns [`OverrideError::InvalidValue`] when the JSON has the wrong shape or
/// is outside the allowed bounds.
pub fn parse_override(field: ProvenanceField, value: &Value) -> Result<FieldValue, OverrideError> {
    let invalid = OverrideError::InvalidValue;
    match field {
        ProvenanceField::Title | ProvenanceField::Venue => {
            let text = value.as_str().ok_or_else(|| invalid("must be a string"))?;
            let text = clean_text(text, TITLE_CHARS_MAX).map_err(invalid)?;
            Ok(if field == ProvenanceField::Title {
                FieldValue::Title(text)
            } else {
                FieldValue::Venue(text)
            })
        }
        ProvenanceField::Year => {
            let year = value
                .as_i64()
                .ok_or_else(|| invalid("must be an integer"))?;
            if !(YEAR_MIN..=YEAR_MAX).contains(&year) {
                return Err(invalid("year must be between 1400 and 2200"));
            }
            i32::try_from(year)
                .map(FieldValue::Year)
                .map_err(|_| invalid("year must be between 1400 and 2200"))
        }
        ProvenanceField::Authors => {
            let names = value
                .as_array()
                .ok_or_else(|| invalid("must be an array"))?;
            if names.is_empty() || names.len() > AUTHORS_MAX {
                return Err(invalid("must list 1 to 100 authors"));
            }
            let mut cleaned = Vec::with_capacity(names.len());
            for name in names {
                let name = name
                    .as_str()
                    .ok_or_else(|| invalid("authors must be strings"))?;
                cleaned.push(clean_text(name, AUTHOR_CHARS_MAX).map_err(invalid)?);
            }
            Ok(FieldValue::Authors(cleaned))
        }
        ProvenanceField::Doi => {
            let text = value.as_str().ok_or_else(|| invalid("must be a string"))?;
            identifier::normalize(IdentifierScheme::Doi, text)
                .map(FieldValue::Doi)
                .ok_or_else(|| invalid("not a valid DOI"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn id(seed: u128) -> FieldClaimId {
        FieldClaimId::from_uuid(Uuid::from_u128(seed))
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + seconds, 0).expect("timestamp")
    }

    fn claim(
        seed: u128,
        origin: ClaimOrigin,
        confidence: ClaimConfidence,
        value: FieldValue,
    ) -> ClaimCandidate {
        ClaimCandidate {
            id: id(seed),
            origin,
            confidence,
            value: Some(value),
            review_state: ReviewState::Unreviewed,
            retracted: false,
            created_at: at(i64::try_from(seed).expect("seed")),
        }
    }

    fn title(text: &str) -> FieldValue {
        FieldValue::Title(text.to_owned())
    }

    fn authors(names: &[&str]) -> FieldValue {
        FieldValue::Authors(names.iter().map(|name| (*name).to_owned()).collect())
    }

    fn by_origin(origin: ClaimOrigin, confidence: ClaimConfidence, seed: u128) -> ClaimCandidate {
        claim(
            seed,
            origin,
            confidence,
            title(&format!("Title from {origin}")),
        )
    }

    fn five_tiers() -> Vec<ClaimCandidate> {
        vec![
            by_origin(ClaimOrigin::User, ClaimConfidence::Explicit, 1),
            by_origin(ClaimOrigin::Publisher, ClaimConfidence::Explicit, 2),
            by_origin(ClaimOrigin::Paper, ClaimConfidence::Heuristic, 3),
            by_origin(ClaimOrigin::Openalex, ClaimConfidence::Explicit, 4),
            by_origin(
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
                5,
            ),
        ]
    }

    fn selected_text(candidates: &[ClaimCandidate]) -> Option<String> {
        select_field(ProvenanceField::Title, candidates)
            .value
            .map(|value| value.display())
    }

    #[test]
    fn should_select_highest_trust_claim_when_tiers_disagree() {
        let mut candidates = five_tiers();
        let expected = [
            "Title from user",
            "Title from publisher",
            "Title from paper",
            "Title from openalex",
            "Title from citing_bibliography",
        ];
        for text in expected {
            assert_eq!(selected_text(&candidates).as_deref(), Some(text));
            candidates.remove(0);
        }
        assert_eq!(selected_text(&candidates), None);

        let heuristic = by_origin(
            ClaimOrigin::CitingBibliography,
            ClaimConfidence::Heuristic,
            6,
        );
        let model = claim(
            7,
            ClaimOrigin::CitingBibliography,
            ClaimConfidence::Model,
            title("Model title"),
        );
        assert_eq!(
            selected_text(&[model, heuristic]).as_deref(),
            Some("Title from citing_bibliography")
        );
    }

    fn permutations(
        items: &mut Vec<ClaimCandidate>,
        start: usize,
        out: &mut Vec<Vec<ClaimCandidate>>,
    ) {
        if start == items.len() {
            out.push(items.clone());
            return;
        }
        for index in start..items.len() {
            items.swap(start, index);
            permutations(items, start + 1, out);
            items.swap(start, index);
        }
    }

    #[test]
    fn should_select_highest_trust_claim_when_input_order_is_shuffled() {
        let mut all = Vec::new();
        permutations(&mut five_tiers(), 0, &mut all);
        assert_eq!(all.len(), 120);
        for ordering in all {
            let selection = select_field(ProvenanceField::Title, &ordering);
            assert_eq!(selection.winner, Some(id(1)));
            assert_eq!(selection.state, FieldState::Known);
        }
    }

    #[test]
    fn should_mark_conflicting_when_winning_tier_claims_disagree() {
        let candidates = [
            claim(
                1,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("One title"),
            ),
            claim(
                2,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("Another title"),
            ),
        ];
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.state, FieldState::Conflicting);
        assert_eq!(selection.winner, Some(id(1)));
        assert_eq!(selection.agreement[&id(2)], ClaimAgreement::Disagrees);
    }

    #[test]
    fn should_not_mark_conflicting_when_only_lower_tiers_disagree() {
        let candidates = [
            claim(
                1,
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
                title("Winner"),
            ),
            claim(
                2,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("Second"),
            ),
            claim(
                3,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("Third"),
            ),
        ];
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.state, FieldState::Known);
        assert_eq!(selection.winner, Some(id(1)));
    }

    #[test]
    fn should_not_mark_conflicting_when_titles_differ_only_in_case_and_punctuation() {
        let candidates = [
            claim(
                1,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("Deep  Learning"),
            ),
            claim(
                2,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("deep learning"),
            ),
        ];
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.state, FieldState::Known);
        assert_eq!(selection.agreement[&id(2)], ClaimAgreement::Agrees);
    }

    #[test]
    fn should_treat_explicit_heuristic_and_model_as_separate_tiers_when_selecting() {
        let candidates = [
            claim(
                1,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Model,
                title("Model"),
            ),
            claim(
                2,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
                title("Heuristic"),
            ),
            claim(
                3,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Explicit,
                title("Explicit"),
            ),
        ];
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.winner, Some(id(3)));
        assert_eq!(selection.state, FieldState::Known);
        assert!(
            selection_rank(ClaimOrigin::Paper, ClaimConfidence::Model)
                > selection_rank(ClaimOrigin::Paper, ClaimConfidence::Heuristic)
        );
        assert!(
            selection_rank(ClaimOrigin::Paper, ClaimConfidence::Model)
                < selection_rank(ClaimOrigin::Openalex, ClaimConfidence::Explicit)
        );
    }

    #[test]
    fn should_mask_every_field_when_user_override_present() {
        let cases = [
            (ProvenanceField::Title, title("Tiered"), title("Mine")),
            (
                ProvenanceField::Year,
                FieldValue::Year(2001),
                FieldValue::Year(1999),
            ),
            (
                ProvenanceField::Authors,
                authors(&["A. One"]),
                authors(&["B. Two"]),
            ),
            (
                ProvenanceField::Venue,
                FieldValue::Venue("Venue A".to_owned()),
                FieldValue::Venue("Venue B".to_owned()),
            ),
            (
                ProvenanceField::Doi,
                FieldValue::Doi("10.1/a".to_owned()),
                FieldValue::Doi("10.1/b".to_owned()),
            ),
        ];
        for (field, tiered, mine) in cases {
            let candidates = [
                claim(1, ClaimOrigin::Publisher, ClaimConfidence::Explicit, tiered),
                claim(
                    2,
                    ClaimOrigin::User,
                    ClaimConfidence::Explicit,
                    mine.clone(),
                ),
            ];
            let selection = select_field(field, &candidates);
            assert_eq!(selection.value, Some(mine), "{field}");
            assert_eq!(selection.winner, Some(id(2)));
            assert_eq!(selection.state, FieldState::Known);
            assert_eq!(selection.agreement[&id(1)], ClaimAgreement::Disagrees);
        }
    }

    #[test]
    fn should_restore_tiered_value_when_override_retracted() {
        let mut candidates = [
            claim(
                1,
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
                title("Tiered"),
            ),
            claim(
                2,
                ClaimOrigin::User,
                ClaimConfidence::Explicit,
                title("Mine"),
            ),
        ];
        assert_eq!(selected_text(&candidates).as_deref(), Some("Mine"));
        candidates[1].retracted = true;
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.value, Some(title("Tiered")));
        assert_eq!(
            selection.agreement[&id(2)],
            ClaimAgreement::Excluded(ExclusionReason::Retracted)
        );
    }

    #[test]
    fn should_exclude_rejected_claim_when_selecting() {
        let mut candidates = [
            claim(
                1,
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
                title("Publisher"),
            ),
            claim(
                2,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("OpenAlex"),
            ),
        ];
        candidates[0].review_state = ReviewState::Rejected;
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.winner, Some(id(2)));
        assert_eq!(selection.state, FieldState::Known);
        assert_eq!(
            selection.agreement[&id(1)],
            ClaimAgreement::Excluded(ExclusionReason::Rejected)
        );
    }

    #[test]
    fn should_return_unknown_when_every_claim_is_excluded() {
        let mut candidates = [
            claim(
                1,
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
                title("A"),
            ),
            claim(2, ClaimOrigin::User, ClaimConfidence::Explicit, title("B")),
        ];
        candidates[0].review_state = ReviewState::Rejected;
        candidates[1].retracted = true;
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.state, FieldState::Unknown);
        assert_eq!(selection.value, None);
        assert_eq!(selection.winner, None);
        assert_eq!(selection.rank, None);
        assert_eq!(selection.agreement.len(), 2);
        let empty = select_field(ProvenanceField::Year, &[]);
        assert_eq!(empty.state, FieldState::Unknown);
    }

    #[test]
    fn should_exclude_unreadable_value_when_json_does_not_match_field() {
        assert_eq!(
            FieldValue::parse(ProvenanceField::Year, &json!("2020")),
            None
        );
        assert_eq!(FieldValue::parse(ProvenanceField::Title, &json!(5)), None);
        assert_eq!(
            FieldValue::parse(ProvenanceField::Authors, &json!([])),
            None
        );
        assert_eq!(
            FieldValue::parse(ProvenanceField::Authors, &json!(["A", 3])),
            None
        );
        assert_eq!(
            FieldValue::parse(
                ProvenanceField::Doi,
                &json!({"scheme": "arxiv", "value": "x"})
            ),
            None
        );
        let mut broken = claim(
            1,
            ClaimOrigin::Publisher,
            ClaimConfidence::Explicit,
            title("x"),
        );
        broken.value = FieldValue::parse(ProvenanceField::Title, &json!(["not", "a title"]));
        let good = claim(
            2,
            ClaimOrigin::Openalex,
            ClaimConfidence::Explicit,
            title("Good"),
        );
        let selection = select_field(ProvenanceField::Title, &[broken, good]);
        assert_eq!(selection.winner, Some(id(2)));
        assert_eq!(
            selection.agreement[&id(1)],
            ClaimAgreement::Excluded(ExclusionReason::Unreadable)
        );
    }

    #[test]
    fn should_compare_authors_by_ordered_family_names_when_normalizing() {
        assert!(values_agree(
            &authors(&["Christiano, P. F.", "Leike, J."]),
            &authors(&["Paul F. Christiano", "Jan Leike"]),
        ));
        assert!(!values_agree(
            &authors(&["Paul F. Christiano", "Jan Leike"]),
            &authors(&["Jan Leike", "Paul F. Christiano"]),
        ));
        assert!(!values_agree(
            &authors(&["Paul Christiano"]),
            &authors(&["Paul Christiano", "Jan Leike"]),
        ));
    }

    #[test]
    fn should_agree_when_truncated_author_list_is_prefix_of_full_list() {
        let full = authors(&["Paul Christiano", "Jan Leike", "Tom Brown"]);
        for printed in [
            "Christiano et al.",
            "Paul Christiano et al",
            "Christiano and others",
        ] {
            assert!(values_agree(&authors(&[printed]), &full), "{printed}");
            assert!(values_agree(&full, &authors(&[printed])), "{printed}");
        }
        assert!(values_agree(
            &authors(&["Paul Christiano", "Jan Leike", "et al."]),
            &full
        ));
        assert!(!values_agree(&authors(&["Leike et al."]), &full));
        assert!(!values_agree(&authors(&["et al."]), &full));
        let key = author_list_key(&["Smith, J.".to_owned(), "others".to_owned()]);
        assert!(key.truncated);
        assert_eq!(key.families, vec!["smith".to_owned()]);
    }

    #[test]
    fn should_prefer_complete_author_list_when_tier_agrees() {
        let candidates = [
            claim(
                1,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
                authors(&["Christiano et al."]),
            ),
            claim(
                2,
                ClaimOrigin::CitingBibliography,
                ClaimConfidence::Heuristic,
                authors(&["Paul Christiano", "Jan Leike"]),
            ),
        ];
        let selection = select_field(ProvenanceField::Authors, &candidates);
        assert_eq!(selection.winner, Some(id(2)));
        assert_eq!(selection.state, FieldState::Known);
        assert_eq!(selection.agreement[&id(1)], ClaimAgreement::Agrees);
    }

    #[test]
    fn should_mark_agreement_for_every_claim_when_selecting() {
        let mut candidates = vec![
            claim(
                1,
                ClaimOrigin::Publisher,
                ClaimConfidence::Explicit,
                title("Winner"),
            ),
            claim(
                2,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("winner"),
            ),
            claim(
                3,
                ClaimOrigin::Openalex,
                ClaimConfidence::Explicit,
                title("Other"),
            ),
            claim(
                4,
                ClaimOrigin::Paper,
                ClaimConfidence::Model,
                title("Rejected"),
            ),
            claim(
                5,
                ClaimOrigin::Paper,
                ClaimConfidence::Model,
                title("Retracted"),
            ),
            claim(
                6,
                ClaimOrigin::Paper,
                ClaimConfidence::Model,
                title("Unreadable"),
            ),
        ];
        candidates[3].review_state = ReviewState::Rejected;
        candidates[4].retracted = true;
        candidates[5].value = None;
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.agreement.len(), candidates.len());
        assert_eq!(selection.agreement[&id(1)], ClaimAgreement::Selected);
        assert_eq!(selection.agreement[&id(2)], ClaimAgreement::Agrees);
        assert_eq!(selection.agreement[&id(3)], ClaimAgreement::Disagrees);
        assert_eq!(
            selection.agreement[&id(4)],
            ClaimAgreement::Excluded(ExclusionReason::Rejected)
        );
        assert_eq!(
            selection.agreement[&id(5)],
            ClaimAgreement::Excluded(ExclusionReason::Retracted)
        );
        assert_eq!(
            selection.agreement[&id(6)],
            ClaimAgreement::Excluded(ExclusionReason::Unreadable)
        );
    }

    #[test]
    fn should_break_ties_by_support_then_age_then_id_when_tier_agrees() {
        let rank = (ClaimOrigin::Openalex, ClaimConfidence::Explicit);
        // Support: two agreeing "B" claims beat one older "A" claim.
        let mut candidates = [
            claim(1, rank.0, rank.1, title("A")),
            claim(2, rank.0, rank.1, title("B")),
            claim(3, rank.0, rank.1, title("b")),
        ];
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.winner, Some(id(2)));
        assert_eq!(selection.state, FieldState::Conflicting);
        // Age: with equal support the older claim wins.
        candidates[2].value = Some(title("C"));
        candidates[1].created_at = at(50);
        candidates[0].created_at = at(40);
        candidates[2].created_at = at(30);
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.winner, Some(id(3)));
        // Id: with equal age the lower id wins.
        candidates[0].created_at = at(10);
        candidates[1].created_at = at(10);
        candidates[2].created_at = at(10);
        let selection = select_field(ProvenanceField::Title, &candidates);
        assert_eq!(selection.winner, Some(id(1)));
    }

    #[test]
    fn should_reject_invalid_override_values_when_parsing() {
        let long_title = "a".repeat(501);
        let many_authors: Vec<String> = (0..101).map(|n| format!("Author {n}")).collect();
        let long_author = "a".repeat(201);
        let invalid = [
            (ProvenanceField::Title, json!("")),
            (ProvenanceField::Title, json!("   ")),
            (ProvenanceField::Title, json!(long_title)),
            (ProvenanceField::Title, json!("bad\u{0007}title")),
            (ProvenanceField::Title, json!(5)),
            (ProvenanceField::Venue, json!("")),
            (ProvenanceField::Year, json!(1399)),
            (ProvenanceField::Year, json!(2201)),
            (ProvenanceField::Year, json!("2020")),
            (ProvenanceField::Year, json!(2020.5)),
            (ProvenanceField::Authors, json!([])),
            (ProvenanceField::Authors, json!(many_authors)),
            (ProvenanceField::Authors, json!([""])),
            (ProvenanceField::Authors, json!([long_author])),
            (ProvenanceField::Authors, json!("A. One")),
            (ProvenanceField::Authors, json!([1])),
            (ProvenanceField::Doi, json!("not a doi")),
            (ProvenanceField::Doi, json!(10)),
        ];
        for (field, value) in invalid {
            let error = parse_override(field, &value).expect_err(&format!("{field} {value}"));
            assert_eq!(error.code(), "invalid_value");
        }
        assert_eq!(
            parse_override(ProvenanceField::Title, &json!("  Spaced \n  out  ")),
            Ok(title("Spaced out"))
        );
        assert_eq!(
            parse_override(ProvenanceField::Year, &json!(1400)),
            Ok(FieldValue::Year(1400))
        );
        assert_eq!(
            parse_override(ProvenanceField::Year, &json!(2200)),
            Ok(FieldValue::Year(2200))
        );
        assert_eq!(
            parse_override(ProvenanceField::Authors, &json!([" A. One ", "B. Two"])),
            Ok(authors(&["A. One", "B. Two"]))
        );
        assert_eq!(
            parse_override(ProvenanceField::Doi, &json!("https://doi.org/10.1000/ABC")),
            Ok(FieldValue::Doi("10.1000/abc".to_owned()))
        );
    }

    #[test]
    fn should_round_trip_claim_enums_and_field_claim_id_when_parsing() {
        for origin in ORIGIN_TRUST_ORDER {
            assert_eq!(
                origin.as_str().parse::<ClaimOrigin>().expect("origin"),
                origin
            );
        }
        for confidence in [
            ClaimConfidence::Explicit,
            ClaimConfidence::Heuristic,
            ClaimConfidence::Model,
        ] {
            assert_eq!(
                confidence
                    .as_str()
                    .parse::<ClaimConfidence>()
                    .expect("confidence"),
                confidence
            );
        }
        for field in PROVENANCE_FIELDS {
            assert_eq!(
                field.as_str().parse::<ProvenanceField>().expect("field"),
                field
            );
        }
        assert!("nobody".parse::<ClaimOrigin>().is_err());
        let claim_id = FieldClaimId::new();
        let text = claim_id.to_string();
        assert_eq!(text.chars().next(), Some('a'));
        assert_eq!(text.parse::<FieldClaimId>().expect("parse"), claim_id);
        assert!(
            crate::id::WorkId::new()
                .to_string()
                .parse::<FieldClaimId>()
                .is_err()
        );
        assert_eq!(ProvenanceField::Authors.field_kind(), FieldKind::Author);
        assert_eq!(ProvenanceField::Doi.field_kind(), FieldKind::Identifier);
        assert!(ProvenanceField::Year.selected_on_work());
        assert!(!ProvenanceField::Venue.selected_on_work());
        for value in [
            title("T"),
            FieldValue::Year(2020),
            authors(&["A. One"]),
            FieldValue::Venue("V".to_owned()),
            FieldValue::Doi("10.1/x".to_owned()),
        ] {
            assert_eq!(
                FieldValue::parse(value.field(), &value.to_json()),
                Some(value)
            );
        }
    }
}
