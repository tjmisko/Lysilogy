//! Claims: citations a person or model asserts by name, and the pure decision that says
//! whether gathered evidence supports them.
//!
//! Gathering the evidence (local lookup, identifier resolution, title search) is I/O and
//! lives in `lysilogy-ingest`; [`decide_claim`] only weighs what was gathered, so the same
//! evidence always yields the same verdict.

use serde::{Deserialize, Serialize};

use crate::names::citation_family;
use crate::{ClaimState, WorkId, title_key, title_similarity};

/// Most provider candidates kept on an ambiguous claim.
pub const CLAIM_CANDIDATES_MAX: usize = 5;
/// A provider title below this similarity to the claimed title disagrees with it.
pub const TITLE_DISAGREE_BELOW: f64 = 0.6;
/// A title search result at or above this similarity is a plausible match.
pub const TITLE_PLAUSIBLE_MIN: f64 = 0.8;
/// A title that is a word-prefix of the other (a dropped subtitle) needs this many words.
const SUBTITLE_MIN_WORDS: usize = 3;

/// What was asserted about a paper. Everything but the title is optional.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub venue: Option<String>,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub arxiv: Option<String>,
}

/// A Work already in the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalWork {
    pub work_id: WorkId,
    pub title: String,
    pub year: Option<i32>,
}

/// A record from a provider (`OpenAlex`, Crossref).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub provider_id: String,
    pub title: String,
    pub year: Option<i32>,
    pub authors: Vec<String>,
}

/// The result of resolving one claimed DOI or arXiv ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentifierLookup {
    Resolved(ProviderRecord),
    NotFound,
    /// Timeout, 5xx or any other failure that says nothing about the identifier.
    TransportError,
}

/// The result of searching providers by title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TitleSearch {
    NotRun,
    Results(Vec<ProviderRecord>),
    TransportError,
}

/// Everything gathered for one claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimEvidence {
    /// Works in the graph whose title key might match; the decision re-checks them.
    pub local: Vec<LocalWork>,
    /// One lookup per claimed identifier, in the order they should be trusted.
    pub identifiers: Vec<IdentifierLookup>,
    pub title_search: TitleSearch,
}

/// How a claim came out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// An existing Work.
    Matched(WorkId),
    /// A provider record that is the claimed paper; the caller creates or finds its Work.
    Verified(ProviderRecord),
    /// Two or more plausible matches; see [`ClaimVerdict::candidates`].
    Ambiguous,
    /// An identifier resolved to a different paper and the title found nothing.
    Contradicted,
    /// Nothing supports the claim.
    Unverified,
    /// A lookup failed in transit; retry later.
    Pending,
}

impl ClaimOutcome {
    /// The stored claim state, or `None` when the claim became a Work.
    #[must_use]
    pub const fn claim_state(&self) -> Option<ClaimState> {
        match self {
            Self::Matched(_) | Self::Verified(_) => None,
            Self::Ambiguous => Some(ClaimState::Ambiguous),
            Self::Contradicted => Some(ClaimState::Contradicted),
            Self::Unverified => Some(ClaimState::Unverified),
            Self::Pending => Some(ClaimState::Pending),
        }
    }
}

/// The decision plus what the caller needs to store or report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimVerdict {
    pub outcome: ClaimOutcome,
    /// At most [`CLAIM_CANDIDATES_MAX`] records, best first; filled when `Ambiguous`.
    pub candidates: Vec<ProviderRecord>,
    /// True when a claimed identifier was set aside (it resolved to another paper, or did
    /// not resolve) and the outcome rests on the title alone.
    pub identifier_dropped: bool,
}

const fn verdict(outcome: ClaimOutcome, identifier_dropped: bool) -> ClaimVerdict {
    ClaimVerdict {
        outcome,
        candidates: Vec::new(),
        identifier_dropped,
    }
}

const fn years_agree(left: Option<i32>, right: Option<i32>) -> bool {
    match (left, right) {
        (Some(a), Some(b)) => (a - b).abs() <= 1,
        _ => true,
    }
}

fn titles_disagree(claimed: &str, found: &str) -> bool {
    if claimed.trim().is_empty() || found.trim().is_empty() {
        return false;
    }
    title_similarity(claimed, found) < TITLE_DISAGREE_BELOW
}

fn family_key(author: &str) -> Option<String> {
    let family = citation_family(author).or_else(|| {
        author
            .split_whitespace()
            .next_back()
            .map(|word| word.trim_end_matches('.').to_owned())
    })?;
    let key = title_key(&family);
    (!key.is_empty()).then_some(key)
}

fn authors_agree(claimed: &[String], found: &[String]) -> bool {
    let wanted: Vec<String> = claimed.iter().filter_map(|a| family_key(a)).collect();
    let present: Vec<String> = found.iter().filter_map(|a| family_key(a)).collect();
    if wanted.is_empty() || present.is_empty() {
        return true;
    }
    wanted.iter().any(|family| present.contains(family))
}

/// Whether one key is the other cut off at a word boundary (a dropped subtitle).
fn is_word_prefix(left: &str, right: &str) -> bool {
    let (short, long) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    short.split_whitespace().count() >= SUBTITLE_MIN_WORDS
        && long.starts_with(short)
        && long[short.len()..].starts_with(' ')
}

struct Scored<'a> {
    record: &'a ProviderRecord,
    exact: bool,
    similarity: f64,
}

/// Rule 3: pick plausible records from a title search.
fn plausible<'a>(claim: &Claim, records: &'a [ProviderRecord]) -> Vec<Scored<'a>> {
    let key = title_key(&claim.title);
    let mut seen = std::collections::HashSet::new();
    let mut found: Vec<Scored<'a>> = records
        .iter()
        .filter(|record| seen.insert(record.provider_id.as_str()))
        .filter(|record| years_agree(claim.year, record.year))
        .filter(|record| authors_agree(&claim.authors, &record.authors))
        .filter_map(|record| {
            let record_key = title_key(&record.title);
            let exact = record_key == key;
            let similarity = title_similarity(&claim.title, &record.title);
            let close =
                exact || similarity >= TITLE_PLAUSIBLE_MIN || is_word_prefix(&key, &record_key);
            close.then_some(Scored {
                record,
                exact,
                similarity,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        b.exact
            .cmp(&a.exact)
            .then_with(|| b.similarity.total_cmp(&a.similarity))
            .then_with(|| a.record.provider_id.cmp(&b.record.provider_id))
    });
    found
}

/// Decide a claim from gathered evidence. The first decisive rule wins:
///
/// 1. **Local exact**: a Work with the same title key and a year within one (or no year on
///    either side) is `Matched`; the smallest Work ID if several.
/// 2. **Identifier**: a lookup that resolves to a record whose title does not disagree is
///    `Verified`. A record whose title disagrees marks the claim contradicted, and an
///    identifier that does not resolve is set aside; both fall through to rule 3.
/// 3. **Title search**: records within a year of the claim, sharing a claimed author surname
///    when both sides name authors, with the same title key or a near-identical or
///    subtitle-trimmed one. One plausible record (or one exact title among several) is
///    `Verified`; otherwise two or more are `Ambiguous` with the top five kept.
/// 4. **Transport errors**: when nothing decided and any lookup failed in transit the claim is
///    `Pending`, never `Unverified`. Otherwise it is `Contradicted` if an identifier pointed
///    at another paper, else `Unverified`.
///
/// A claim with no usable title skips rules 1 and 3.
#[must_use]
pub fn decide_claim(claim: &Claim, evidence: &ClaimEvidence) -> ClaimVerdict {
    let key = title_key(&claim.title);
    let searchable = !claim.title.trim().is_empty() && key != "untitled";

    if searchable {
        let local = evidence
            .local
            .iter()
            .filter(|work| title_key(&work.title) == key && years_agree(claim.year, work.year))
            .map(|work| work.work_id)
            .min();
        if let Some(work_id) = local {
            return verdict(ClaimOutcome::Matched(work_id), false);
        }
    }

    let mut contradicted = false;
    let mut transport_failed = false;
    let mut identifier_dropped = false;
    for lookup in &evidence.identifiers {
        match lookup {
            IdentifierLookup::Resolved(record) => {
                if !titles_disagree(&claim.title, &record.title) {
                    return verdict(ClaimOutcome::Verified(record.clone()), false);
                }
                contradicted = true;
                identifier_dropped = true;
            }
            IdentifierLookup::NotFound => identifier_dropped = true,
            IdentifierLookup::TransportError => transport_failed = true,
        }
    }

    match &evidence.title_search {
        TitleSearch::TransportError => transport_failed = true,
        TitleSearch::Results(records) if searchable => {
            let found = plausible(claim, records);
            let exact = found.iter().filter(|scored| scored.exact).count();
            match found.as_slice() {
                [] => {}
                [only] => {
                    return verdict(
                        ClaimOutcome::Verified(only.record.clone()),
                        identifier_dropped,
                    );
                }
                [first, ..] if exact == 1 && first.exact => {
                    return verdict(
                        ClaimOutcome::Verified(first.record.clone()),
                        identifier_dropped,
                    );
                }
                many => {
                    return ClaimVerdict {
                        outcome: ClaimOutcome::Ambiguous,
                        candidates: many
                            .iter()
                            .take(CLAIM_CANDIDATES_MAX)
                            .map(|scored| scored.record.clone())
                            .collect(),
                        identifier_dropped,
                    };
                }
            }
        }
        TitleSearch::NotRun | TitleSearch::Results(_) => {}
    }

    let outcome = if transport_failed {
        ClaimOutcome::Pending
    } else if contradicted {
        ClaimOutcome::Contradicted
    } else {
        ClaimOutcome::Unverified
    };
    verdict(outcome, identifier_dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn claim(title: &str) -> Claim {
        Claim {
            title: title.into(),
            ..Claim::default()
        }
    }

    fn record(id: &str, title: &str, year: Option<i32>, authors: &[&str]) -> ProviderRecord {
        ProviderRecord {
            provider_id: id.into(),
            title: title.into(),
            year,
            authors: authors.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    fn local(number: u128, title: &str, year: Option<i32>) -> LocalWork {
        LocalWork {
            work_id: WorkId::from_uuid(Uuid::from_u128(number)),
            title: title.into(),
            year,
        }
    }

    fn evidence(
        local: Vec<LocalWork>,
        identifiers: Vec<IdentifierLookup>,
        title_search: TitleSearch,
    ) -> ClaimEvidence {
        ClaimEvidence {
            local,
            identifiers,
            title_search,
        }
    }

    const TITLE: &str = "Learning to summarize from human feedback";

    #[test]
    fn should_match_existing_work_when_title_key_and_year_agree() {
        let mut asked = claim("Learning to Summarize from Human Feedback!");
        asked.year = Some(2021);
        let ev = evidence(
            vec![local(9, TITLE, Some(2020)), local(3, TITLE, None)],
            vec![],
            TitleSearch::TransportError,
        );
        let result = decide_claim(&asked, &ev);
        assert_eq!(
            result.outcome,
            ClaimOutcome::Matched(WorkId::from_uuid(Uuid::from_u128(3)))
        );
        assert_eq!(result.outcome.claim_state(), None);
    }

    #[test]
    fn should_not_match_local_work_when_year_differs_by_more_than_one() {
        let mut asked = claim(TITLE);
        asked.year = Some(2020);
        let ev = evidence(
            vec![
                local(1, TITLE, Some(2023)),
                local(2, "Another title entirely", None),
            ],
            vec![],
            TitleSearch::Results(vec![]),
        );
        assert_eq!(decide_claim(&asked, &ev).outcome, ClaimOutcome::Unverified);
    }

    #[test]
    fn should_verify_when_identifier_resolves_to_agreeing_title() {
        let found = record("W1", TITLE, Some(2020), &["Nisan Stiennon"]);
        let mut asked = claim("Learning to summarise from human feedback");
        asked.doi = Some("10.1/x".into());
        let ev = evidence(
            vec![],
            vec![IdentifierLookup::Resolved(found.clone())],
            TitleSearch::TransportError,
        );
        let result = decide_claim(&asked, &ev);
        assert_eq!(result.outcome, ClaimOutcome::Verified(found));
        assert!(!result.identifier_dropped);
    }

    #[test]
    fn should_use_first_agreeing_identifier_when_several_resolve() {
        let wrong = record(
            "W9",
            "Quantum chromodynamics on the lattice",
            Some(1999),
            &[],
        );
        let right = record("W1", TITLE, Some(2020), &[]);
        let ev = evidence(
            vec![],
            vec![
                IdentifierLookup::Resolved(wrong),
                IdentifierLookup::Resolved(right.clone()),
            ],
            TitleSearch::NotRun,
        );
        assert_eq!(
            decide_claim(&claim(TITLE), &ev).outcome,
            ClaimOutcome::Verified(right)
        );
    }

    #[test]
    fn should_fall_to_title_search_when_identifier_title_disagrees() {
        let other = record(
            "W9",
            "Quantum chromodynamics on the lattice",
            Some(1999),
            &[],
        );
        let real = record("W1", TITLE, Some(2020), &["Nisan Stiennon"]);
        let ev = evidence(
            vec![],
            vec![IdentifierLookup::Resolved(other)],
            TitleSearch::Results(vec![real.clone()]),
        );
        let result = decide_claim(&claim(TITLE), &ev);
        assert_eq!(result.outcome, ClaimOutcome::Verified(real));
        assert!(result.identifier_dropped);
    }

    #[test]
    fn should_be_contradicted_when_identifier_disagrees_and_title_search_finds_nothing() {
        let other = record(
            "W9",
            "Quantum chromodynamics on the lattice",
            Some(1999),
            &[],
        );
        let ev = evidence(
            vec![],
            vec![IdentifierLookup::Resolved(other)],
            TitleSearch::Results(vec![record("W5", "Unrelated study of soil", None, &[])]),
        );
        let result = decide_claim(&claim(TITLE), &ev);
        assert_eq!(result.outcome, ClaimOutcome::Contradicted);
        assert_eq!(result.outcome.claim_state(), Some(ClaimState::Contradicted));
    }

    #[test]
    fn should_set_aside_identifier_when_it_does_not_resolve() {
        let real = record("W1", TITLE, Some(2020), &[]);
        let ev = evidence(
            vec![],
            vec![IdentifierLookup::NotFound],
            TitleSearch::Results(vec![real.clone()]),
        );
        let result = decide_claim(&claim(TITLE), &ev);
        assert_eq!(result.outcome, ClaimOutcome::Verified(real));
        assert!(result.identifier_dropped);
        let none = evidence(
            vec![],
            vec![IdentifierLookup::NotFound],
            TitleSearch::Results(vec![]),
        );
        assert_eq!(
            decide_claim(&claim(TITLE), &none).outcome,
            ClaimOutcome::Unverified
        );
    }

    #[test]
    fn should_be_pending_when_a_lookup_fails_in_transit_and_nothing_else_decides() {
        let down = evidence(vec![], vec![], TitleSearch::TransportError);
        assert_eq!(
            decide_claim(&claim(TITLE), &down).outcome,
            ClaimOutcome::Pending
        );
        let id_down = evidence(
            vec![],
            vec![IdentifierLookup::TransportError],
            TitleSearch::Results(vec![]),
        );
        assert_eq!(
            decide_claim(&claim(TITLE), &id_down).outcome,
            ClaimOutcome::Pending
        );
        // A contradicted identifier is not final while the title search is unreachable.
        let other = record("W9", "Quantum chromodynamics on the lattice", None, &[]);
        let mixed = evidence(
            vec![],
            vec![IdentifierLookup::Resolved(other)],
            TitleSearch::TransportError,
        );
        let result = decide_claim(&claim(TITLE), &mixed);
        assert_eq!(result.outcome, ClaimOutcome::Pending);
        assert_eq!(result.outcome.claim_state(), Some(ClaimState::Pending));
    }

    #[test]
    fn should_verify_despite_transport_error_when_another_lookup_decides() {
        let real = record("W1", TITLE, Some(2020), &[]);
        let ev = evidence(
            vec![],
            vec![
                IdentifierLookup::TransportError,
                IdentifierLookup::Resolved(real.clone()),
            ],
            TitleSearch::NotRun,
        );
        assert_eq!(
            decide_claim(&claim(TITLE), &ev).outcome,
            ClaimOutcome::Verified(real)
        );
    }

    #[test]
    fn should_be_unverified_when_title_search_finds_nothing_plausible() {
        let mut asked = claim(TITLE);
        asked.year = Some(2020);
        let ev = evidence(
            vec![],
            vec![],
            TitleSearch::Results(vec![
                record("W2", TITLE, Some(2010), &[]),
                record(
                    "W3",
                    "Totally different title about graphs",
                    Some(2020),
                    &[],
                ),
            ]),
        );
        let result = decide_claim(&asked, &ev);
        assert_eq!(result.outcome, ClaimOutcome::Unverified);
        assert!(result.candidates.is_empty());
        assert_eq!(
            decide_claim(&asked, &evidence(vec![], vec![], TitleSearch::NotRun)).outcome,
            ClaimOutcome::Unverified
        );
    }

    #[test]
    fn should_require_a_shared_author_surname_when_authors_are_claimed() {
        let mut asked = claim(TITLE);
        asked.authors = vec!["N. Stiennon".into(), "Long Ouyang".into()];
        let swapped = record("W2", TITLE, Some(2020), &["Alice Smith", "Bob Jones"]);
        let ev = evidence(vec![], vec![], TitleSearch::Results(vec![swapped]));
        assert_eq!(decide_claim(&asked, &ev).outcome, ClaimOutcome::Unverified);
        let right = record("W1", TITLE, Some(2020), &["Nisan Stiennon", "Alice Smith"]);
        let ev = evidence(vec![], vec![], TitleSearch::Results(vec![right.clone()]));
        assert_eq!(
            decide_claim(&asked, &ev).outcome,
            ClaimOutcome::Verified(right)
        );
        // A record without authors cannot contradict the claim.
        let bare = record("W3", TITLE, None, &[]);
        let ev = evidence(vec![], vec![], TitleSearch::Results(vec![bare.clone()]));
        assert_eq!(
            decide_claim(&asked, &ev).outcome,
            ClaimOutcome::Verified(bare)
        );
    }

    #[test]
    fn should_verify_near_misses_when_title_has_a_typo_or_dropped_subtitle() {
        let typo = record("W1", TITLE, Some(2020), &[]);
        let ev = evidence(vec![], vec![], TitleSearch::Results(vec![typo.clone()]));
        assert_eq!(
            decide_claim(&claim("Learnnig to summarize from humen feedback"), &ev).outcome,
            ClaimOutcome::Verified(typo)
        );
        let full = record(
            "W2",
            "Attention is all you need: a transformer for sequence transduction",
            Some(2017),
            &[],
        );
        let ev = evidence(vec![], vec![], TitleSearch::Results(vec![full.clone()]));
        assert_eq!(
            decide_claim(&claim("Attention is all you need"), &ev).outcome,
            ClaimOutcome::Verified(full)
        );
    }

    #[test]
    fn should_be_ambiguous_with_top_five_when_several_records_are_plausible() {
        let records: Vec<ProviderRecord> = (0..7)
            .map(|n| {
                record(
                    &format!("W{n}"),
                    &format!("{TITLE} part {n}"),
                    Some(2020),
                    &[],
                )
            })
            .collect();
        let ev = evidence(vec![], vec![], TitleSearch::Results(records));
        let result = decide_claim(&claim(TITLE), &ev);
        assert_eq!(result.outcome, ClaimOutcome::Ambiguous);
        assert_eq!(result.candidates.len(), 5);
        assert_eq!(result.outcome.claim_state(), Some(ClaimState::Ambiguous));
    }

    #[test]
    fn should_prefer_the_single_exact_title_when_a_longer_variant_also_appears() {
        let exact = record("W1", TITLE, Some(2020), &[]);
        let longer = record("W2", &format!("{TITLE}: a sequel"), Some(2021), &[]);
        let ev = evidence(
            vec![],
            vec![],
            TitleSearch::Results(vec![longer, exact.clone()]),
        );
        assert_eq!(
            decide_claim(&claim(TITLE), &ev).outcome,
            ClaimOutcome::Verified(exact)
        );
    }

    #[test]
    fn should_be_ambiguous_when_two_distinct_records_share_the_exact_title() {
        let ev = evidence(
            vec![],
            vec![],
            TitleSearch::Results(vec![
                record("W2", TITLE, Some(2020), &[]),
                record("W1", TITLE, Some(2021), &[]),
            ]),
        );
        let result = decide_claim(&claim(TITLE), &ev);
        assert_eq!(result.outcome, ClaimOutcome::Ambiguous);
        assert_eq!(result.candidates[0].provider_id, "W1");
    }

    #[test]
    fn should_count_a_duplicated_provider_record_once_when_searching() {
        let dup = record("W1", TITLE, Some(2020), &[]);
        let ev = evidence(
            vec![],
            vec![],
            TitleSearch::Results(vec![dup.clone(), dup.clone()]),
        );
        assert_eq!(
            decide_claim(&claim(TITLE), &ev).outcome,
            ClaimOutcome::Verified(dup)
        );
    }

    #[test]
    fn should_not_search_or_match_locally_when_title_is_blank_or_untitled() {
        for title in ["", "   ", "Untitled"] {
            let ev = evidence(
                vec![local(1, title, None)],
                vec![],
                TitleSearch::Results(vec![record("W1", "Untitled", None, &[])]),
            );
            assert_eq!(
                decide_claim(&claim(title), &ev).outcome,
                ClaimOutcome::Unverified
            );
        }
        // An identifier can still verify a title-less claim.
        let found = record("W1", TITLE, None, &[]);
        let ev = evidence(
            vec![],
            vec![IdentifierLookup::Resolved(found.clone())],
            TitleSearch::NotRun,
        );
        assert_eq!(
            decide_claim(&claim(""), &ev).outcome,
            ClaimOutcome::Verified(found)
        );
    }

    #[test]
    fn should_round_trip_claim_json_when_optional_fields_are_missing() {
        let parsed: Claim = serde_json::from_str(r#"{"title":"T","year":2020}"#).expect("json");
        assert_eq!(parsed.year, Some(2020));
        assert!(parsed.authors.is_empty());
        assert!(parsed.doi.is_none());
    }
}
