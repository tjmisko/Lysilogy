//! The core reading's relations as the first label on an edge.
//!
//! The core model call returns `relations` (for the cited works the paper's
//! argument leans on: a reference key as printed, a relation from the same
//! vocabulary, a why, a verbatim quote and a confidence) and `citations`
//! (citing sentences), and quote admission has already checked their quotes
//! against the page text. This module resolves each reference key to an edge
//! of the citation graph through the paper's own stored bibliography (the
//! printed key, then the first author and year, then the core reading's
//! bibliography entry by DOI or title), writes the relation as an unreviewed
//! `core_relation` claim under [`super::merge_decision`], and hands the
//! citing sentences of edges without stored mentions to the edge labeler.
//!
//! Nothing is dropped silently: every relation ends in exactly one counted
//! outcome, and every reference key that could not be resolved is listed
//! with the reason.

use std::collections::{BTreeMap, HashMap};

use lysilogy_core::{Coverage, DocumentVersionId, EntryId, MentionId, WorkId};
use lysilogy_db::evidence::StoredEntry;
use lysilogy_db::relationship::{LabelPath, NewAiClaim};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    ExistingLabel, LabelContext, MergeDecision, check_label_text, merge_decision,
    quote_has_enough_words,
};
use crate::Result;
use crate::bibliography::BibliographicFields;

/// Source record kind for a core relation written as a label.
pub const CORE_LABEL_RECORD_KIND: &str = "core_relation_label";

/// Most unresolved keys listed by name; the counts are always complete.
const MAX_LISTED_KEYS: usize = 50;

/// Fewest shared title tokens, as a share of the shorter title, for a title match.
const TITLE_MATCH: f64 = 0.8;

/// One relation from the core reading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreRelation {
    pub reference_key: String,
    pub relation: String,
    pub why: String,
    pub quote: String,
    pub page: Option<i64>,
    pub confidence: Option<String>,
}

/// One citing sentence from the core reading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreCitation {
    pub reference_key: String,
    pub context: String,
    pub page: Option<i64>,
}

/// One bibliography entry from the core reading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreReference {
    pub key: String,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub year: Option<i64>,
    pub doi: Option<String>,
}

/// The graph fields of one core reading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoreReading {
    pub relations: Vec<CoreRelation>,
    pub citations: Vec<CoreCitation>,
    pub references: Vec<CoreReference>,
    /// Identifies the run, e.g. its pipeline version and time, so a label
    /// from one run is written once and a newer run's labels replace it.
    pub run_key: String,
    /// The model key stored on the claims.
    pub model: String,
}

impl CoreReading {
    /// Read the graph fields from a record's `artifacts`. Entries that do not
    /// have the expected shape are skipped and counted in the second value.
    #[must_use]
    pub fn from_artifacts(artifacts: &Value, run_key: &str, model: &str) -> (Self, usize) {
        let mut malformed = 0;
        let reading = Self {
            relations: parse_list(artifacts, "relations", &mut malformed),
            citations: parse_list(artifacts, "citations", &mut malformed),
            references: parse_list(artifacts, "references", &mut malformed),
            run_key: run_key.to_owned(),
            model: model.to_owned(),
        };
        (reading, malformed)
    }
}

fn parse_list<T: serde::de::DeserializeOwned>(
    artifacts: &Value,
    field: &str,
    malformed: &mut usize,
) -> Vec<T> {
    artifacts
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| {
                    let parsed = serde_json::from_value(value.clone()).ok();
                    if parsed.is_none() {
                        *malformed += 1;
                    }
                    parsed
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Why a reference key did not lead to an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unresolved {
    /// The model returned an empty key (it could not match the bibliography).
    EmptyKey,
    /// No stored bibliography entry matches the key.
    NoMatchingEntry,
    /// More than one entry matches and nothing tells them apart.
    AmbiguousEntry,
    /// An entry matches, but it is not resolved to a Work in the graph, or
    /// the paper has no edge to that Work.
    EntryNotInGraph,
}

impl Unresolved {
    /// The stable code used in summaries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmptyKey => "empty_key",
            Self::NoMatchingEntry => "no_matching_entry",
            Self::AmbiguousEntry => "ambiguous_entry",
            Self::EntryNotInGraph => "entry_not_in_graph",
        }
    }
}

/// A key that did not resolve, for the run report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedKey {
    pub key: String,
    pub reason: Unresolved,
    /// `relation` or `citation`.
    pub field: String,
}

/// A core relation that met a label it did not replace, or replaced one
/// that named another relation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disagreement {
    pub cited_work: String,
    pub reference_key: String,
    pub core_relation: String,
    pub existing_relation: String,
    pub existing_path: LabelPath,
    /// Whether the core relation replaced the existing label.
    pub replaced: bool,
}

/// What seeding did. Every relation lands in exactly one of `invalid`,
/// `low_confidence`, the `unresolved` counts, `duplicate_edge`, `inserted`,
/// `replaced`, `unchanged` or `kept_existing`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreSeedSummary {
    pub relations: usize,
    pub citations: usize,
    /// Entries of the core reading that could not be read at all.
    pub malformed: usize,
    /// Relations whose relation, why or quote fail the label checks.
    pub invalid: BTreeMap<String, usize>,
    /// Relations the model marked low confidence: left for the edge labeler.
    pub low_confidence: usize,
    /// Relation keys that did not resolve to an edge, by reason.
    pub unresolved: BTreeMap<String, usize>,
    /// Citation keys that did not resolve to an edge, by reason.
    pub unresolved_citations: BTreeMap<String, usize>,
    /// The first unresolved keys, by name.
    pub unresolved_keys: Vec<UnresolvedKey>,
    /// A second relation for an edge that already took one in this reading.
    pub duplicate_edge: usize,
    pub inserted: usize,
    pub replaced: usize,
    /// This run's label is already the current one.
    pub unchanged: usize,
    /// A reviewed or edge-labeler label was kept.
    pub kept_existing: usize,
    pub disagreements: Vec<Disagreement>,
    /// Citing sentences handed to the edge labeler for edges without
    /// stored mentions.
    pub citation_passages: usize,
}

/// What the edge labeler needs from seeding.
#[derive(Clone, Debug, Default)]
pub struct CoreSeedOutcome {
    pub summary: CoreSeedSummary,
    /// Core citing sentences for edges with no stored mention, by cited Work.
    pub extra_passages: HashMap<WorkId, Vec<String>>,
}

/// The parts of a stored entry that key matching reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyedEntry {
    pub printed_key: Option<String>,
    pub raw_text: String,
    pub title: Option<String>,
    pub year: Option<String>,
    pub first_author: Option<String>,
    pub doi: Option<String>,
}

impl KeyedEntry {
    /// The matching view of a stored entry.
    #[must_use]
    pub fn of(entry: &StoredEntry) -> Self {
        let fields = serde_json::from_value::<BibliographicFields>(entry.fields.clone()).ok();
        let fields = fields.as_ref();
        Self {
            printed_key: entry
                .printed_key
                .clone()
                .or_else(|| fields.and_then(|fields| fields.printed_key.value.clone())),
            raw_text: entry.raw_text.clone(),
            title: fields.and_then(|fields| fields.title.value.clone()),
            year: fields.and_then(|fields| fields.year.value.clone()),
            first_author: fields
                .and_then(|fields| fields.authors.value.as_ref())
                .and_then(|authors| authors.first().cloned()),
            doi: fields.and_then(|fields| fields.doi.value.clone()),
        }
    }
}

fn fold(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    text.nfkd()
        .filter(char::is_ascii)
        .collect::<String>()
        .to_lowercase()
}

/// Lowercase, accents and punctuation removed, `&` read as `and`, `et al.`
/// removed, whitespace collapsed.
fn normalize_key(key: &str) -> String {
    let folded = fold(&key.replace('&', " and "));
    let words: Vec<String> = folded
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let mut kept = Vec::with_capacity(words.len());
    let mut index = 0;
    while index < words.len() {
        if words[index] == "et" && words.get(index + 1).is_some_and(|next| next == "al") {
            index += 2;
            continue;
        }
        kept.push(words[index].clone());
        index += 1;
    }
    kept.join(" ")
}

/// The digits of a purely numeric key such as `12` or `[12]`.
fn numeric_key(key: &str) -> Option<String> {
    let inner = key
        .trim()
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')', '.'])
        .trim();
    (!inner.is_empty() && inner.chars().all(|character| character.is_ascii_digit()))
        .then(|| inner.trim_start_matches('0').to_owned())
}

/// The first four-digit year in `text` (1500-2099) with any letter suffix.
fn year_in(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len().saturating_sub(3) {
        let window = &bytes[start..start + 4];
        if !window.iter().all(u8::is_ascii_digit) {
            continue;
        }
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            continue;
        }
        if bytes.get(start + 4).is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let year = &text[start..start + 4];
        if !(year.starts_with("15")
            || year.starts_with("16")
            || year.starts_with("17")
            || year.starts_with("18")
            || year.starts_with("19")
            || year.starts_with("20"))
        {
            continue;
        }
        let suffix = bytes
            .get(start + 4)
            .filter(|byte| byte.is_ascii_lowercase())
            .filter(|_| !bytes.get(start + 5).is_some_and(u8::is_ascii_alphabetic))
            .map(|byte| char::from(*byte));
        return Some(suffix.map_or_else(|| year.to_owned(), |letter| format!("{year}{letter}")));
    }
    None
}

/// The first surname in an author-year key: the first word that starts with
/// a capital letter and is not a joining word.
fn key_surname(key: &str) -> Option<String> {
    key.split(|character: char| !character.is_alphabetic() && character != '\'' && character != '-')
        .find(|word| {
            word.chars().next().is_some_and(char::is_uppercase)
                && !matches!(*word, "And" | "Et" | "Al" | "In" | "See" | "E" | "G")
        })
        .map(fold)
}

/// The surname of an entry's first author, from its fields or its text.
fn entry_surname(entry: &KeyedEntry) -> Option<String> {
    let author = entry
        .first_author
        .as_deref()
        .map(str::trim)
        .filter(|author| !author.is_empty());
    let name = author.map_or_else(
        || {
            entry
                .raw_text
                .trim_start_matches(|character: char| !character.is_alphabetic())
                .split(|character: char| {
                    !character.is_alphabetic() && character != '-' && character != '\''
                })
                .next()
                .unwrap_or_default()
                .to_owned()
        },
        |author| match author.split_once(',') {
            Some((surname, _)) => surname.trim().to_owned(),
            None => author
                .split_whitespace()
                .last()
                .unwrap_or(author)
                .to_owned(),
        },
    );
    let name = fold(&name);
    (!name.is_empty()).then_some(name)
}

fn title_tokens(title: &str) -> Vec<String> {
    let mut tokens: Vec<String> = fold(title)
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| word.len() > 2)
        .map(str::to_owned)
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn titles_match(left: &str, right: &str) -> bool {
    let (left, right) = (title_tokens(left), title_tokens(right));
    let shorter = left.len().min(right.len());
    if shorter < 2 {
        return false;
    }
    let shared = left.iter().filter(|token| right.contains(token)).count();
    #[allow(clippy::cast_precision_loss)] // Token counts are tiny.
    let share = shared as f64 / shorter as f64;
    share >= TITLE_MATCH
}

fn unique(matches: &[usize]) -> Option<std::result::Result<usize, Unresolved>> {
    match matches {
        [] => None,
        [only] => Some(Ok(*only)),
        _ => Some(Err(Unresolved::AmbiguousEntry)),
    }
}

/// The index of the entry a reference key names.
///
/// Tried in order: the printed key (normalized); a numeric key against
/// numeric printed keys; the first author's surname and the year (a letter
/// suffix must agree when both have one); and the core reading's own entry
/// for the key, matched by DOI and then by title. The first step that finds
/// anything decides: one match resolves, several are ambiguous.
///
/// # Errors
/// Why the key did not resolve to one entry.
pub fn match_entry(
    key: &str,
    entries: &[KeyedEntry],
    references: &[CoreReference],
) -> std::result::Result<usize, Unresolved> {
    let normalized = normalize_key(key);
    if normalized.is_empty() {
        return Err(Unresolved::EmptyKey);
    }
    let by_printed: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .printed_key
                .as_deref()
                .is_some_and(|printed| normalize_key(printed) == normalized)
        })
        .map(|(index, _)| index)
        .collect();
    if let Some(found) = unique(&by_printed) {
        return found;
    }
    if let Some(number) = numeric_key(key) {
        let by_number: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry
                    .printed_key
                    .as_deref()
                    .and_then(numeric_key)
                    .as_deref()
                    == Some(&number)
            })
            .map(|(index, _)| index)
            .collect();
        if let Some(found) = unique(&by_number) {
            return found;
        }
    } else if let (Some(surname), Some(year)) = (key_surname(key), year_in(key)) {
        let by_author: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry_surname(entry).as_deref() == Some(surname.as_str()))
            .filter(|(_, entry)| {
                let entry_year = entry
                    .year
                    .as_deref()
                    .and_then(year_in)
                    .or_else(|| year_in(&entry.raw_text));
                entry_year.is_some_and(|entry_year| years_agree(&year, &entry_year))
            })
            .map(|(index, _)| index)
            .collect();
        if let Some(found) = unique(&by_author) {
            return found;
        }
    }
    let reference = references
        .iter()
        .find(|reference| normalize_key(&reference.key) == normalized);
    if let Some(reference) = reference {
        if let Some(doi) = reference
            .doi
            .as_deref()
            .map(str::trim)
            .filter(|doi| !doi.is_empty())
        {
            let by_doi: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry
                        .doi
                        .as_deref()
                        .is_some_and(|entry_doi| entry_doi.trim().eq_ignore_ascii_case(doi))
                })
                .map(|(index, _)| index)
                .collect();
            if let Some(found) = unique(&by_doi) {
                return found;
            }
        }
        if let Some(title) = reference.title.as_deref() {
            let by_title: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry
                        .title
                        .as_deref()
                        .is_some_and(|entry_title| titles_match(title, entry_title))
                })
                .map(|(index, _)| index)
                .collect();
            if let Some(found) = unique(&by_title) {
                return found;
            }
        }
    }
    Err(Unresolved::NoMatchingEntry)
}

fn years_agree(key_year: &str, entry_year: &str) -> bool {
    if key_year.len() > 4 && entry_year.len() > 4 {
        return key_year == entry_year;
    }
    key_year[..4] == entry_year[..4]
}

/// One local edge as seeding sees it.
#[derive(Clone, Debug)]
pub struct SeedEdge {
    pub cited: WorkId,
    pub assertion: lysilogy_core::AssertionId,
    /// `(mention id, passage)` for each stored mention.
    pub mentions: Vec<(MentionId, String)>,
}

/// One label seeding wants written, after the merge rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeedAction {
    Insert,
    Replace(lysilogy_core::RelationshipClaimId),
}

/// Resolve a key to an edge: entry, then the entry's accepted Work, then
/// the paper's edge to that Work.
///
/// # Errors
/// Why the key did not lead to an edge.
pub fn resolve_edge<'e, S: std::hash::BuildHasher, T: std::hash::BuildHasher>(
    key: &str,
    entries: &[(EntryId, KeyedEntry)],
    resolved: &HashMap<EntryId, WorkId, S>,
    edges: &'e HashMap<WorkId, SeedEdge, T>,
    references: &[CoreReference],
) -> std::result::Result<&'e SeedEdge, Unresolved> {
    let keyed: Vec<KeyedEntry> = entries.iter().map(|(_, entry)| entry.clone()).collect();
    let index = match_entry(key, &keyed, references)?;
    let entry_id = entries[index].0;
    resolved
        .get(&entry_id)
        .and_then(|work| edges.get(work))
        .ok_or(Unresolved::EntryNotInGraph)
}

fn note_unresolved(summary: &mut CoreSeedSummary, key: &str, reason: Unresolved, field: &str) {
    let counts = if field == "relation" {
        &mut summary.unresolved
    } else {
        &mut summary.unresolved_citations
    };
    *counts.entry(reason.as_str().to_owned()).or_default() += 1;
    if summary.unresolved_keys.len() < MAX_LISTED_KEYS {
        summary.unresolved_keys.push(UnresolvedKey {
            key: key.to_owned(),
            reason,
            field: field.to_owned(),
        });
    }
}

/// Write the core reading's relations as `core_relation` labels on the
/// edges of `citing_work` in `version`, and collect core citing sentences
/// for edges without stored mentions.
///
/// # Errors
/// Database failures.
#[allow(clippy::too_many_lines)] // One pass over the relations, every outcome counted in place.
pub async fn seed_core_labels(
    ctx: &LabelContext<'_>,
    citing_work: WorkId,
    version: DocumentVersionId,
    reading: &CoreReading,
    malformed: usize,
) -> Result<CoreSeedOutcome> {
    let mut summary = CoreSeedSummary {
        relations: reading.relations.len(),
        citations: reading.citations.len(),
        malformed,
        ..CoreSeedSummary::default()
    };
    let entries: Vec<(EntryId, KeyedEntry)> = ctx
        .evidence
        .entries_for_version(version)
        .await?
        .iter()
        .map(|entry| (entry.id, KeyedEntry::of(entry)))
        .collect();
    let resolved: HashMap<EntryId, WorkId> = ctx
        .evidence
        .resolved_entries(version)
        .await?
        .into_iter()
        .map(|(entry, work)| (entry.id, work))
        .collect();
    let edges: HashMap<WorkId, SeedEdge> = ctx
        .evidence
        .local_edges(citing_work, version)
        .await?
        .into_iter()
        .map(|(cited, assertion, mentions)| {
            let mentions = mentions
                .into_iter()
                .map(|mention| (mention.id, mention.passage))
                .collect();
            (
                cited,
                SeedEdge {
                    cited,
                    assertion,
                    mentions,
                },
            )
        })
        .collect();
    let assertions: Vec<_> = edges.values().map(|edge| edge.assertion).collect();
    let current = ctx
        .relationships
        .current_claims_by_assertion(&assertions)
        .await?;

    let mut seeded = std::collections::HashSet::new();
    for relation in &reading.relations {
        let quote = relation.quote.trim();
        let checked = if quote.is_empty() || !quote_has_enough_words(quote) {
            Err("quote_too_short")
        } else {
            check_label_text(&relation.relation, None, &relation.why)
                .map_err(super::GroundingRejection::as_str)
        };
        if let Err(code) = checked {
            *summary.invalid.entry(code.to_owned()).or_default() += 1;
            continue;
        }
        if relation.confidence.as_deref() == Some("low") {
            summary.low_confidence += 1;
            continue;
        }
        let edge = match resolve_edge(
            &relation.reference_key,
            &entries,
            &resolved,
            &edges,
            &reading.references,
        ) {
            Ok(edge) => edge,
            Err(reason) => {
                note_unresolved(&mut summary, &relation.reference_key, reason, "relation");
                continue;
            }
        };
        if !seeded.insert(edge.assertion) {
            summary.duplicate_edge += 1;
            continue;
        }
        let existing = current.get(&edge.assertion);
        let decision = merge_decision(
            existing.map(ExistingLabel::of),
            LabelPath::CoreRelation,
            &relation.relation,
        );
        let action = match (decision, existing) {
            (MergeDecision::Keep { disagrees }, Some(existing)) => {
                summary.kept_existing += 1;
                if disagrees {
                    summary.disagreements.push(Disagreement {
                        cited_work: edge.cited.to_string(),
                        reference_key: relation.reference_key.clone(),
                        core_relation: relation.relation.clone(),
                        existing_relation: existing.relation.clone(),
                        existing_path: existing.path(),
                        replaced: false,
                    });
                }
                continue;
            }
            (MergeDecision::Replace { disagrees }, Some(existing)) => {
                if disagrees {
                    summary.disagreements.push(Disagreement {
                        cited_work: edge.cited.to_string(),
                        reference_key: relation.reference_key.clone(),
                        core_relation: relation.relation.clone(),
                        existing_relation: existing.relation.clone(),
                        existing_path: existing.path(),
                        replaced: true,
                    });
                }
                SeedAction::Replace(existing.id)
            }
            _ => SeedAction::Insert,
        };
        let payload = json!({
            "run_key": reading.run_key,
            "relation": relation,
        });
        let digest = format!(
            "{:x}",
            Sha256::digest(
                json!([relation.relation, relation.why, relation.quote])
                    .to_string()
                    .as_bytes()
            )
        );
        let source_key = format!(
            "{}|{}|{}",
            reading.run_key,
            normalize_key(&relation.reference_key),
            &digest[..16]
        );
        if existing.and_then(|claim| claim.source_record).is_some()
            && let Some((record, _)) = ctx
                .records
                .find(CORE_LABEL_RECORD_KIND, &source_key)
                .await?
            && existing.and_then(|claim| claim.source_record) == Some(record)
        {
            summary.unchanged += 1;
            continue;
        }
        let record = ctx
            .records
            .upsert(
                CORE_LABEL_RECORD_KIND,
                &source_key,
                Coverage::Complete,
                &payload,
            )
            .await?;
        let mention = edge
            .mentions
            .iter()
            .find(|(_, passage)| passage.contains(quote))
            .map(|(id, _)| *id);
        let claim = NewAiClaim {
            assertion: edge.assertion,
            mention,
            relation: &relation.relation,
            reason: relation.why.trim(),
            quote,
            model: &reading.model,
            record,
            path: LabelPath::CoreRelation,
            engagement: None,
            prompt_version: None,
        };
        match action {
            SeedAction::Insert => {
                if ctx.relationships.insert_ai_claim(&claim).await?.is_some() {
                    summary.inserted += 1;
                } else {
                    summary.unchanged += 1;
                }
            }
            SeedAction::Replace(old) => match ctx.relationships.replace_ai_claim(old, &claim).await
            {
                Ok(Some(_)) => summary.replaced += 1,
                Ok(None) => summary.unchanged += 1,
                Err(lysilogy_db::DbError::Conflict(_)) => summary.kept_existing += 1,
                Err(error) => return Err(error.into()),
            },
        }
    }

    let mut extra_passages: HashMap<WorkId, Vec<String>> = HashMap::new();
    for citation in &reading.citations {
        let context = citation.context.trim();
        if context.is_empty() {
            continue;
        }
        match resolve_edge(
            &citation.reference_key,
            &entries,
            &resolved,
            &edges,
            &reading.references,
        ) {
            Ok(edge) if edge.mentions.is_empty() => {
                let passages = extra_passages.entry(edge.cited).or_default();
                if !passages.iter().any(|passage| passage == context) {
                    passages.push(context.to_owned());
                    summary.citation_passages += 1;
                }
            }
            Ok(_) => {}
            Err(reason) => {
                note_unresolved(&mut summary, &citation.reference_key, reason, "citation");
            }
        }
    }
    Ok(CoreSeedOutcome {
        summary,
        extra_passages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        printed: Option<&str>,
        raw: &str,
        author: Option<&str>,
        year: Option<&str>,
    ) -> KeyedEntry {
        KeyedEntry {
            printed_key: printed.map(str::to_owned),
            raw_text: raw.to_owned(),
            first_author: author.map(str::to_owned),
            year: year.map(str::to_owned),
            ..KeyedEntry::default()
        }
    }

    #[test]
    fn should_match_numeric_keys_with_or_without_brackets() {
        let entries = vec![
            entry(Some("1"), "[1] A. Smith. One. 2001.", None, None),
            entry(Some("12"), "[12] B. Jones. Twelve. 2012.", None, None),
        ];
        assert_eq!(match_entry("12", &entries, &[]), Ok(1));
        assert_eq!(match_entry("[12]", &entries, &[]), Ok(1));
        assert_eq!(match_entry("[1]", &entries, &[]), Ok(0));
        assert_eq!(
            match_entry("7", &entries, &[]),
            Err(Unresolved::NoMatchingEntry)
        );
        assert_eq!(match_entry("  ", &entries, &[]), Err(Unresolved::EmptyKey));
    }

    #[test]
    fn should_match_author_year_keys_by_surname_and_year() {
        let entries = vec![
            entry(
                None,
                "Smith, A. and Jones, B. (2004). Reef cover.",
                Some("Smith, A."),
                Some("2004"),
            ),
            entry(
                None,
                "M\u{fc}ller, K. (1999a). Coastal waters.",
                Some("K. M\u{fc}ller"),
                Some("1999a"),
            ),
            entry(
                None,
                "M\u{fc}ller, K. (1999b). More waters.",
                Some("K. M\u{fc}ller"),
                Some("1999b"),
            ),
            entry(None, "Okafor, T. 2010. Tides.", None, None),
        ];
        assert_eq!(match_entry("Smith and Jones (2004)", &entries, &[]), Ok(0));
        assert_eq!(match_entry("Smith & Jones, 2004", &entries, &[]), Ok(0));
        assert_eq!(
            match_entry("Muller 1999b", &entries, &[]),
            Ok(2),
            "accents fold, suffix decides"
        );
        assert_eq!(
            match_entry("M\u{fc}ller (1999)", &entries, &[]),
            Err(Unresolved::AmbiguousEntry),
            "two 1999 entries and no suffix"
        );
        assert_eq!(
            match_entry("Okafor et al., 2010", &entries, &[]),
            Ok(3),
            "author from the raw text"
        );
        assert_eq!(
            match_entry("Smith (2005)", &entries, &[]),
            Err(Unresolved::NoMatchingEntry)
        );
    }

    #[test]
    fn should_fall_back_to_the_core_bibliography_by_doi_then_title() {
        let mut by_doi = entry(None, "Anon. Report.", None, None);
        by_doi.doi = Some("10.1000/XYZ".to_owned());
        let mut by_title = entry(
            None,
            "Committee. Annual survey of reef fish populations.",
            None,
            None,
        );
        by_title.title = Some("Annual survey of reef fish populations".to_owned());
        let entries = vec![by_doi, by_title];
        let references = vec![
            CoreReference {
                key: "Anon (n.d.)".to_owned(),
                doi: Some("10.1000/xyz".to_owned()),
                ..CoreReference::default()
            },
            CoreReference {
                key: "Committee".to_owned(),
                title: Some("An annual survey of reef-fish populations".to_owned()),
                ..CoreReference::default()
            },
        ];
        assert_eq!(match_entry("Anon (n.d.)", &entries, &references), Ok(0));
        assert_eq!(match_entry("Committee", &entries, &references), Ok(1));
        assert_eq!(
            match_entry("Nobody", &entries, &references),
            Err(Unresolved::NoMatchingEntry)
        );
    }

    #[test]
    fn should_report_an_entry_that_is_not_in_the_graph() {
        let first = EntryId::new();
        let second = EntryId::new();
        let entries = vec![
            (first, entry(Some("1"), "[1] One.", None, None)),
            (second, entry(Some("2"), "[2] Two.", None, None)),
        ];
        let work = WorkId::new();
        let resolved = HashMap::from([(first, work)]);
        let edges = HashMap::from([(
            work,
            SeedEdge {
                cited: work,
                assertion: lysilogy_core::AssertionId::new(),
                mentions: Vec::new(),
            },
        )]);
        assert_eq!(
            resolve_edge("1", &entries, &resolved, &edges, &[]).map(|edge| edge.cited),
            Ok(work)
        );
        assert_eq!(
            resolve_edge("2", &entries, &resolved, &edges, &[]).map(|edge| edge.cited),
            Err(Unresolved::EntryNotInGraph)
        );
        assert_eq!(
            resolve_edge("3", &entries, &resolved, &edges, &[]).map(|edge| edge.cited),
            Err(Unresolved::NoMatchingEntry)
        );
    }

    #[test]
    fn should_read_the_graph_fields_of_a_record_and_count_malformed_entries() {
        let artifacts = json!({
            "relations": [
                {"reference_key": "3", "relation": "uses_method", "why": "Takes their index.",
                 "quote": "we use the index of", "page": 4, "confidence": "high"},
                "not an object"
            ],
            "citations": [{"reference_key": "3", "context": "As shown in [3], it holds.", "page": 2}],
            "references": [{"key": "3", "title": "An index", "authors": ["A"], "year": 2001,
                            "venue": null, "doi": null, "page": 9}]
        });
        let (reading, malformed) = CoreReading::from_artifacts(&artifacts, "run", "m");
        assert_eq!(malformed, 1);
        assert_eq!(reading.relations.len(), 1);
        assert_eq!(reading.relations[0].confidence.as_deref(), Some("high"));
        assert_eq!(reading.citations[0].page, Some(2));
        assert_eq!(reading.references[0].year, Some(2001));
        let (empty, none) = CoreReading::from_artifacts(&json!({}), "run", "m");
        assert!(empty.relations.is_empty() && none == 0);
    }

    #[test]
    fn should_find_years_and_surnames_in_keys() {
        assert_eq!(year_in("Smith (2004a)").as_deref(), Some("2004a"));
        assert_eq!(year_in("Smith 2004, p. 3").as_deref(), Some("2004"));
        assert_eq!(year_in("12345"), None);
        assert_eq!(year_in("in 2004an"), Some("2004".to_owned()));
        assert_eq!(
            key_surname("see Smith and Jones (2004)").as_deref(),
            Some("smith")
        );
        assert_eq!(normalize_key("Smith et al., 2020"), "smith 2020");
        assert_eq!(numeric_key("[07]").as_deref(), Some("7"));
    }
}
