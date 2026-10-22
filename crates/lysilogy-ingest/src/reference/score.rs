//! Multi-signal scoring and an explicit accept/ambiguous/reject decision.
//!
//! [`MatchScore::total`] is a bounded, monotone match score, not a calibrated
//! probability. It becomes a probability only after calibration against a
//! labeled reference test set; until then callers must treat the thresholds
//! here as policy, not truth.

use std::collections::BTreeSet;

use lysilogy_core::{IdentifierScheme, title_similarity};
use serde::{Deserialize, Serialize};

use super::candidate::CandidateRecord;
use super::record::ReferenceRecord;

/// The lowest title similarity that still counts as a possible match.
pub const TITLE_FLOOR: f64 = 0.55;
/// The total score at which a unique best candidate is accepted.
pub const ACCEPT_SCORE: f64 = 0.72;
/// The total score below which a candidate is not even a contender.
pub const CANDIDATE_SCORE: f64 = 0.45;
/// How far the best candidate must lead the runner-up to be accepted.
pub const ACCEPT_MARGIN: f64 = 0.06;

const TITLE_WEIGHT: f64 = 0.62;
const AUTHOR_WEIGHT: f64 = 0.22;
const YEAR_WEIGHT: f64 = 0.10;
const VENUE_WEIGHT: f64 = 0.06;

/// Whether a record and a candidate agree on a shared identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierAgreement {
    /// At least one shared scheme carries the same normalized value.
    Match,
    /// A shared scheme carries different values.
    Conflict,
    /// No shared scheme with values on both sides.
    Unknown,
}

/// The per-signal scores behind a match decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchScore {
    pub identifier: IdentifierAgreement,
    pub title: Option<f64>,
    pub authors: Option<f64>,
    pub year: Option<f64>,
    pub venue: Option<f64>,
    /// Weighted aggregate over the signals present; see the module docs.
    pub total: f64,
}

/// A candidate with its position in the caller's input list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoredCandidate {
    pub index: usize,
    pub score: MatchScore,
}

/// Why no candidate was accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    NoCandidates,
    NoTitle,
    IdentifierConflict,
    TitleDisagrees,
    YearDisagrees,
}

impl RejectionReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoCandidates => "no_candidates",
            Self::NoTitle => "no_title",
            Self::IdentifierConflict => "identifier_conflict",
            Self::TitleDisagrees => "title_disagrees",
            Self::YearDisagrees => "year_disagrees",
        }
    }
}

/// The outcome of comparing a record against a candidate list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum MatchDecision {
    Accepted {
        candidate: usize,
        score: MatchScore,
    },
    Ambiguous {
        candidates: Vec<usize>,
        scores: Vec<MatchScore>,
    },
    Rejected(RejectionReason),
}

/// Score one candidate against a record.
#[must_use]
pub fn score_candidate(record: &ReferenceRecord, candidate: &CandidateRecord) -> MatchScore {
    let identifier = identifier_agreement(record, candidate);
    let title = match (record.title.as_deref(), candidate.title.as_deref()) {
        (Some(left), Some(right)) => Some(title_similarity(left, right)),
        _ => None,
    };
    let authors = family_overlap(&record.author_families(), &candidate.author_families());
    let year = year_agreement(record.year, candidate.year);
    let venue = match (record.venue.as_deref(), candidate.venue.as_deref()) {
        (Some(left), Some(right)) => Some(title_similarity(left, right)),
        _ => None,
    };
    let total = match identifier {
        IdentifierAgreement::Match => 1.0,
        IdentifierAgreement::Conflict => 0.0,
        IdentifierAgreement::Unknown => weighted_mean(&[
            (TITLE_WEIGHT, title),
            (AUTHOR_WEIGHT, authors),
            (YEAR_WEIGHT, year),
            (VENUE_WEIGHT, venue),
        ]),
    };
    MatchScore {
        identifier,
        title,
        authors,
        year,
        venue,
        total,
    }
}

/// Score every candidate and sort by descending total.
#[must_use]
pub fn rank(record: &ReferenceRecord, candidates: &[CandidateRecord]) -> Vec<ScoredCandidate> {
    let mut scored: Vec<ScoredCandidate> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| ScoredCandidate {
            index,
            score: score_candidate(record, candidate),
        })
        .collect();
    scored.sort_by(|left, right| right.score.total.total_cmp(&left.score.total));
    scored
}

/// Decide whether one candidate is the cited Work, which candidates are
/// ambiguous, or why none qualifies.
#[must_use]
pub fn decide(record: &ReferenceRecord, candidates: &[CandidateRecord]) -> MatchDecision {
    if candidates.is_empty() {
        return MatchDecision::Rejected(RejectionReason::NoCandidates);
    }
    if record
        .title
        .as_deref()
        .is_none_or(|title| title.trim().is_empty())
    {
        return MatchDecision::Rejected(RejectionReason::NoTitle);
    }
    let ranked = rank(record, candidates);
    if let Some(found) = ranked
        .iter()
        .find(|scored| scored.score.identifier == IdentifierAgreement::Match)
    {
        return MatchDecision::Accepted {
            candidate: found.index,
            score: found.score.clone(),
        };
    }
    if ranked[0].score.identifier == IdentifierAgreement::Conflict {
        return MatchDecision::Rejected(RejectionReason::IdentifierConflict);
    }
    let year_conflict = ranked.iter().any(|scored| {
        scored.score.identifier != IdentifierAgreement::Conflict
            && scored.score.title.is_some_and(|title| title >= TITLE_FLOOR)
            && !year_compatible(record.year, candidates[scored.index].year)
    });
    let eligible: Vec<&ScoredCandidate> = ranked
        .iter()
        .filter(|scored| scored.score.identifier != IdentifierAgreement::Conflict)
        .filter(|scored| scored.score.title.is_some_and(|title| title >= TITLE_FLOOR))
        .filter(|scored| year_compatible(record.year, candidates[scored.index].year))
        .filter(|scored| scored.score.total >= CANDIDATE_SCORE)
        .collect();
    let Some(top) = eligible.first() else {
        return MatchDecision::Rejected(if year_conflict {
            RejectionReason::YearDisagrees
        } else {
            RejectionReason::TitleDisagrees
        });
    };
    let unique = eligible.len() == 1;
    let margin = unique || top.score.total - eligible[1].score.total >= ACCEPT_MARGIN;
    if top.score.total >= ACCEPT_SCORE && margin {
        MatchDecision::Accepted {
            candidate: top.index,
            score: top.score.clone(),
        }
    } else {
        MatchDecision::Ambiguous {
            candidates: eligible.iter().map(|scored| scored.index).collect(),
            scores: eligible.iter().map(|scored| scored.score.clone()).collect(),
        }
    }
}

fn identifier_agreement(
    record: &ReferenceRecord,
    candidate: &CandidateRecord,
) -> IdentifierAgreement {
    let mut matched = false;
    let mut conflict = false;
    for scheme in [IdentifierScheme::Doi, IdentifierScheme::Arxiv] {
        if let (Some(left), Some(right)) = (record.identifier(scheme), candidate.identifier(scheme))
        {
            if left == right {
                matched = true;
            } else {
                conflict = true;
            }
        }
    }
    if matched {
        IdentifierAgreement::Match
    } else if conflict {
        IdentifierAgreement::Conflict
    } else {
        IdentifierAgreement::Unknown
    }
}

/// A year difference above this many years disqualifies a candidate unless a
/// shared identifier already matched. It accommodates preprint-to-published
/// moves without letting an unrelated work through.
const YEAR_TOLERANCE: i32 = 2;

const fn year_compatible(record: Option<i32>, candidate: Option<i32>) -> bool {
    match (record, candidate) {
        (Some(left), Some(right)) => (left - right).abs() <= YEAR_TOLERANCE,
        _ => true,
    }
}

const fn year_agreement(record: Option<i32>, candidate: Option<i32>) -> Option<f64> {
    let (Some(left), Some(right)) = (record, candidate) else {
        return None;
    };
    Some(match (left - right).abs() {
        0 => 1.0,
        1 => 0.7,
        2 => 0.3,
        _ => 0.0,
    })
}

/// F1 over the two family-name sets; order-insensitive, so `et al.` is partial.
fn family_overlap(record: &[String], candidate: &[String]) -> Option<f64> {
    let record: BTreeSet<&String> = record.iter().collect();
    let candidate: BTreeSet<&String> = candidate.iter().collect();
    if record.is_empty() || candidate.is_empty() {
        return None;
    }
    let shared = record.intersection(&candidate).count();
    if shared == 0 {
        return Some(0.0);
    }
    #[allow(clippy::cast_precision_loss)]
    let shared = shared as f64;
    #[allow(clippy::cast_precision_loss)]
    let (record_len, candidate_len) = (record.len() as f64, candidate.len() as f64);
    let precision = shared / candidate_len;
    let recall = shared / record_len;
    Some(2.0 * precision * recall / (precision + recall))
}

fn weighted_mean(parts: &[(f64, Option<f64>)]) -> f64 {
    let mut sum = 0.0;
    let mut weight = 0.0;
    for &(part_weight, value) in parts {
        if let Some(value) = value {
            sum = part_weight.mul_add(value, sum);
            weight += part_weight;
        }
    }
    if weight > 0.0 { sum / weight } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::record::ReferenceIdentifier;
    use lysilogy_core::VersionKind;

    fn record(title: &str, authors: &[&str], year: Option<i32>) -> ReferenceRecord {
        ReferenceRecord {
            raw: String::new(),
            printed_key: None,
            title: Some(title.to_owned()),
            authors: authors.iter().map(|author| (*author).to_owned()).collect(),
            year,
            venue: None,
            identifiers: Vec::new(),
        }
    }

    fn with_doi(mut record: ReferenceRecord, value: &str) -> ReferenceRecord {
        record.identifiers = vec![ReferenceIdentifier {
            scheme: IdentifierScheme::Doi,
            value: value.to_owned(),
        }];
        record
    }

    fn candidate(id: &str, title: &str, authors: &[&str], year: i32) -> CandidateRecord {
        CandidateRecord {
            source: "test".to_owned(),
            source_id: id.to_owned(),
            title: Some(title.to_owned()),
            authors: authors.iter().map(|author| (*author).to_owned()).collect(),
            year: Some(year),
            venue: None,
            identifiers: Vec::new(),
            version_kind: VersionKind::Unknown,
            version_label: None,
            cited_by_count: None,
        }
    }

    fn doi_candidate(id: &str, value: &str) -> CandidateRecord {
        let mut candidate = candidate(id, "Whatever", &[], 2000);
        candidate.identifiers = vec![ReferenceIdentifier {
            scheme: IdentifierScheme::Doi,
            value: value.to_owned(),
        }];
        candidate
    }

    #[test]
    fn should_accept_on_identifier_even_when_title_is_garbled() {
        let record = with_doi(
            record("Phys. Rev. 108, 1175 (1957).", &["Deser, S."], Some(1957)),
            "10.1103/physrev.108.1175",
        );
        let decision = decide(&record, &[doi_candidate("W1", "10.1103/physrev.108.1175")]);
        assert!(matches!(decision, MatchDecision::Accepted { .. }));
    }

    #[test]
    fn should_reject_on_conflicting_identifier() {
        let record = with_doi(
            record("A Title", &["Smith, J."], Some(2010)),
            "10.1234/aaaa",
        );
        let decision = decide(&record, &[doi_candidate("W1", "10.1234/bbbb")]);
        assert_eq!(
            decision,
            MatchDecision::Rejected(RejectionReason::IdentifierConflict)
        );
    }

    #[test]
    fn should_accept_exact_title_author_and_year() {
        let record = record(
            "Concrete Problems in AI Safety",
            &["Smith, J.", "Jones, A."],
            Some(2016),
        );
        let decision = decide(
            &record,
            &[candidate(
                "W1",
                "Concrete Problems in AI Safety",
                &["J. Smith", "A. Jones"],
                2016,
            )],
        );
        let MatchDecision::Accepted { score, .. } = decision else {
            panic!("expected acceptance, got {decision:?}");
        };
        assert!(score.total >= ACCEPT_SCORE);
        assert_eq!(score.identifier, IdentifierAgreement::Unknown);
    }

    #[test]
    fn should_reject_when_the_title_is_only_the_venue() {
        let record = record("Phys. Rev. 108, 1175 (1957).", &["Deser, S."], Some(1957));
        let decision = decide(
            &record,
            &[candidate(
                "W1",
                "General Relativity and the Divergence Problem in Quantum Field Theory",
                &["Stanley Deser"],
                1957,
            )],
        );
        assert_eq!(
            decision,
            MatchDecision::Rejected(RejectionReason::TitleDisagrees)
        );
    }

    #[test]
    fn should_be_ambiguous_when_two_candidates_are_near_identical() {
        let record = record("A Study of Things", &["Smith, J."], Some(2010));
        let decision = decide(
            &record,
            &[
                candidate("W1", "A Study of Things", &["J. Smith"], 2010),
                candidate("W2", "A Study of Things", &["J. Smith"], 2010),
            ],
        );
        assert!(matches!(decision, MatchDecision::Ambiguous { .. }));
    }

    #[test]
    fn should_reject_without_a_title() {
        let mut record = record("A Title", &[], None);
        record.title = None;
        let decision = decide(&record, &[candidate("W1", "Anything", &[], 2000)]);
        assert_eq!(decision, MatchDecision::Rejected(RejectionReason::NoTitle));
    }

    #[test]
    fn should_reject_when_there_are_no_candidates() {
        assert_eq!(
            decide(&record("A Title", &[], None), &[]),
            MatchDecision::Rejected(RejectionReason::NoCandidates)
        );
    }

    #[test]
    fn should_rank_the_exact_title_first() {
        let record = record("The China Syndrome", &["Smith, J."], Some(2013));
        let ranked = rank(
            &record,
            &[
                candidate("W1", "The China Syndrome Ails America Too", &[], 2013),
                candidate("W2", "The China Syndrome", &["J. Smith"], 2013),
            ],
        );
        assert_eq!(ranked[0].index, 1);
    }

    #[test]
    fn should_treat_et_al_as_partial_author_agreement() {
        let record = record("A Title", &["Smith, J.", "et al."], Some(2010));
        let scored = score_candidate(
            &record,
            &candidate("W1", "A Title", &["J. Smith", "B. Jones", "C. Lee"], 2010),
        );
        let authors = scored.authors.expect("author score");
        assert!(authors > 0.0 && authors < 1.0, "{authors}");
    }

    #[test]
    fn should_penalize_a_year_that_is_far_off() {
        let record = record("A Title", &["Smith, J."], Some(2010));
        let near = score_candidate(&record, &candidate("W1", "A Title", &["J. Smith"], 2011));
        let far = score_candidate(&record, &candidate("W2", "A Title", &["J. Smith"], 1990));
        assert!(near.total > far.total);
    }

    #[test]
    fn should_reject_a_matching_title_with_a_wildly_different_year() {
        let record = record("A Study of Things", &["Smith, J."], Some(2019));
        let decision = decide(
            &record,
            &[candidate("W1", "A Study of Things", &["J. Smith"], 2008)],
        );
        assert_eq!(
            decision,
            MatchDecision::Rejected(RejectionReason::YearDisagrees)
        );
    }
}
