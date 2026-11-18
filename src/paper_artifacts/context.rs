//! The context node: why the paper matters, written after the core fields.
//!
//! The call reads the paper, a brief of the core draft (thesis, position,
//! relations, references) and the neighbour dossier, and returns `context`:
//! a lead, before, contribution, after and a read-alongside list (see
//! `prompts/context.md` and `docs/context-node.md`). Every claim names its
//! source class. Quotes in `paper` claims go through the ordinary quote
//! admission because they sit in `evidence` arrays; [`settle`] then runs
//! after admission and checks the rest deterministically:
//!
//! - a `paper` claim left with no admitted quote becomes `background`,
//!   marked `quote_not_found`;
//! - work ids not in the dossier are removed, and a `dossier` claim left
//!   with none becomes `background`, marked `unknown_work`;
//! - background claims are kept and shown labelled, never withheld;
//! - read-alongside entries are matched to the dossier for their ids, held
//!   papers and authoritative titles, and capped.
//!
//! The counts go on the context itself, so a review can see how much of the
//! answer checked out.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use super::dossier::Dossier;
use crate::domain::ContextCounts;

/// The field the context node fills.
pub const FIELD: &str = "context";

/// The claim lists, in reading order.
pub const CLAIM_LISTS: [&str; 4] = ["lead", "before", "contribution", "after"];

/// The most reading suggestions kept.
pub const READ_ALONGSIDE_LIMIT: usize = 6;

/// The most bibliography entries the core brief carries.
const BRIEF_REFERENCE_LIMIT: usize = 150;

/// Settle the context in `draft` against the dossier, after quote admission.
/// Returns what it changed; `None` when the draft has no context.
pub fn settle(draft: &mut Value, dossier: &Dossier) -> Option<ContextCounts> {
    let context = draft.get_mut(FIELD)?.as_object_mut()?;
    let mut counts = ContextCounts {
        dossier_works: dossier.len(),
        ..ContextCounts::default()
    };
    for list in CLAIM_LISTS {
        let claims = context
            .get_mut(list)
            .and_then(Value::as_array_mut)
            .map(std::mem::take)
            .unwrap_or_default();
        let settled = claims
            .into_iter()
            .filter_map(|claim| settle_claim(claim, dossier, &mut counts))
            .collect::<Vec<_>>();
        context.insert(list.to_owned(), Value::Array(settled));
    }
    let suggestions = context
        .get_mut("read_alongside")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut kept = Vec::new();
    for suggestion in suggestions {
        match settle_suggestion(suggestion, dossier, &mut counts) {
            Some(suggestion) if kept.len() < READ_ALONGSIDE_LIMIT => {
                let key = suggestion
                    .get("work_id")
                    .and_then(Value::as_str)
                    .map_or_else(
                        || {
                            suggestion
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_lowercase()
                        },
                        str::to_owned,
                    );
                if seen.insert(key) {
                    kept.push(suggestion);
                } else {
                    counts.dropped_read_alongside += 1;
                }
            }
            _ => counts.dropped_read_alongside += 1,
        }
    }
    context.insert("read_alongside".to_owned(), Value::Array(kept));
    let note = context
        .get("reception_note")
        .and_then(Value::as_str)
        .map(compact)
        .filter(|note| !note.is_empty());
    context.insert(
        "reception_note".to_owned(),
        note.map_or(Value::Null, Value::String),
    );
    context.insert(
        "counts".to_owned(),
        serde_json::to_value(&counts).unwrap_or(Value::Null),
    );
    Some(counts)
}

/// One claim checked: its basis kept only when its support survived.
fn settle_claim(mut claim: Value, dossier: &Dossier, counts: &mut ContextCounts) -> Option<Value> {
    let fields = claim.as_object_mut()?;
    let text = compact(
        fields
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if text.is_empty() {
        return None;
    }
    fields.insert("text".to_owned(), Value::String(text));
    let requested = fields
        .get("basis")
        .and_then(Value::as_str)
        .unwrap_or("background")
        .to_owned();
    let quotes = fields
        .get("evidence")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let requested_ids = string_list(fields.get("work_ids"));
    let known_ids = requested_ids
        .iter()
        .filter(|id| dossier.get(id).is_some())
        .map(|id| id.trim().to_owned())
        .collect::<Vec<_>>();
    counts.unknown_work_ids += requested_ids.len() - known_ids.len();
    fields.insert("work_ids".to_owned(), json!(known_ids));
    let (basis, downgrade) = match requested.as_str() {
        "paper" if quotes > 0 => ("paper", None),
        "paper" => {
            counts.downgraded_quote_not_found += 1;
            ("background", Some("quote_not_found"))
        }
        "dossier" if !known_ids.is_empty() => ("dossier", None),
        "dossier" => {
            counts.downgraded_unknown_work += 1;
            ("background", Some("unknown_work"))
        }
        _ => ("background", None),
    };
    match basis {
        "paper" => counts.paper_claims += 1,
        "dossier" => counts.dossier_claims += 1,
        _ => counts.background_claims += 1,
    }
    fields.insert("basis".to_owned(), Value::String(basis.to_owned()));
    match downgrade {
        Some(reason) => {
            fields.insert("downgraded".to_owned(), Value::String(reason.to_owned()));
        }
        None => {
            fields.remove("downgraded");
        }
    }
    Some(claim)
}

/// One reading suggestion matched to the dossier; `None` without a title.
fn settle_suggestion(
    mut suggestion: Value,
    dossier: &Dossier,
    counts: &mut ContextCounts,
) -> Option<Value> {
    let fields = suggestion.as_object_mut()?;
    let requested = fields
        .get("work_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let record = requested.as_deref().and_then(|id| dossier.get(id));
    if requested.is_some() && record.is_none() {
        counts.unknown_work_ids += 1;
    }
    if let Some(record) = record {
        fields.insert("work_id".to_owned(), Value::String(record.id.clone()));
        if let Some(title) = &record.title {
            fields.insert("title".to_owned(), Value::String(compact(title)));
        }
        if let Some(year) = record.year {
            fields.insert("year".to_owned(), json!(year));
        }
        if fields.get("authors").is_none_or(Value::is_null)
            && let Some(first) = record.authors.first()
        {
            let authors = if record.authors.len() > 1 {
                format!("{first} et al.")
            } else {
                first.clone()
            };
            fields.insert("authors".to_owned(), Value::String(authors));
        }
        fields.insert(
            "local_paper_id".to_owned(),
            record
                .local_paper_id
                .clone()
                .map_or(Value::Null, Value::String),
        );
    } else {
        fields.insert("work_id".to_owned(), Value::Null);
        fields.insert("local_paper_id".to_owned(), Value::Null);
        if fields.get("basis").and_then(Value::as_str) == Some("dossier") {
            fields.insert("basis".to_owned(), Value::String("background".to_owned()));
        }
    }
    if fields.get("basis").and_then(Value::as_str) == Some("paper") && record.is_none() {
        // A reading suggestion has no quote to admit; the paper naming a
        // work is still only as good as the model's reading of it.
        fields.insert("basis".to_owned(), Value::String("background".to_owned()));
    }
    let title = compact(
        fields
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if title.is_empty() {
        return None;
    }
    fields.insert("title".to_owned(), Value::String(title));
    let why = compact(
        fields
            .get("why")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    fields.insert("why".to_owned(), Value::String(why));
    Some(suggestion)
}

/// What the context node is told about the core draft.
///
/// Identity, type, question, thesis and position, the relations, and the
/// bibliography, with the quotes left out because the paper itself is in the
/// same prompt.
#[must_use]
pub fn core_brief(draft: &Map<String, Value>) -> Option<String> {
    let mut brief = Map::new();
    if let Some(identity) = draft.get("identity") {
        let authors = identity
            .get("authors")
            .and_then(Value::as_array)
            .map(|authors| {
                authors
                    .iter()
                    .filter_map(|author| author.get("name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        brief.insert(
            "identity".to_owned(),
            json!({
                "title": identity.get("title"),
                "authors": authors,
                "year": identity.get("year"),
                "venue": identity.get("venue"),
            }),
        );
    }
    if let Some(kind) = draft.get("type").and_then(|kind| kind.get("value")) {
        brief.insert("type".to_owned(), kind.clone());
    }
    if let Some(question) = draft
        .get("question")
        .and_then(|question| question.get("text"))
    {
        brief.insert("question".to_owned(), question.clone());
    }
    if let Some(thesis) = draft
        .get("card")
        .and_then(|card| card.pointer("/thesis/text"))
    {
        brief.insert("thesis".to_owned(), thesis.clone());
    }
    if let Some(position) = draft.get("card").and_then(|card| card.get("position")) {
        let mut summary = Map::new();
        for side in ["predecessors", "targets", "replications"] {
            let items = position
                .get(side)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            json!({
                                "reference_key": item.get("reference_key"),
                                "cited_work": item.get("cited_work"),
                                "why": item.get("why"),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            summary.insert(side.to_owned(), Value::Array(items));
        }
        brief.insert("position".to_owned(), Value::Object(summary));
    }
    if let Some(relations) = draft.get("relations").and_then(Value::as_array) {
        let relations = relations
            .iter()
            .map(|relation| {
                json!({
                    "reference_key": relation.get("reference_key"),
                    "relation": relation.get("relation"),
                    "why": relation.get("why"),
                })
            })
            .collect::<Vec<_>>();
        brief.insert("relations".to_owned(), Value::Array(relations));
    }
    if let Some(references) = draft.get("references").and_then(Value::as_array) {
        let references = references
            .iter()
            .take(BRIEF_REFERENCE_LIMIT)
            .map(|reference| {
                json!({
                    "key": reference.get("key"),
                    "title": reference.get("title"),
                    "authors": reference.get("authors"),
                    "year": reference.get("year"),
                })
            })
            .collect::<Vec<_>>();
        brief.insert("references".to_owned(), Value::Array(references));
    }
    if brief.is_empty() {
        return None;
    }
    Some(format!(
        "The core draft of this paper, made by an earlier pass (its quotes were checked; read the paper for anything that matters):\n```json\n{}\n```",
        serde_json::to_string(&Value::Object(brief)).ok()?
    ))
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper_artifacts::dossier::DossierWork;

    fn dossier() -> Dossier {
        Dossier {
            subject: None,
            references: vec![DossierWork {
                id: "w1".to_owned(),
                title: Some("The Old Model".to_owned()),
                year: Some(1974),
                authors: vec!["A. Smith".to_owned(), "B. Jones".to_owned()],
                local_paper_id: Some("p1".to_owned()),
                ..DossierWork::default()
            }],
            citing: vec![DossierWork {
                id: "w2".to_owned(),
                title: Some("A Reanalysis".to_owned()),
                year: Some(2017),
                ..DossierWork::default()
            }],
        }
    }

    fn claim(basis: &str, quotes: usize, ids: &[&str]) -> Value {
        json!({
            "text": format!("A {basis} claim."),
            "basis": basis,
            "evidence": (0..quotes).map(|_| json!({"quote": "q", "page": 1})).collect::<Vec<_>>(),
            "work_ids": ids,
        })
    }

    #[test]
    fn should_keep_supported_claims_and_downgrade_the_rest_to_background() {
        let mut draft = json!({"context": {
            "lead": [claim("paper", 1, &[]), claim("paper", 0, &[])],
            "before": [claim("dossier", 0, &["w1", "w404"]), claim("dossier", 0, &["w404"])],
            "contribution": [claim("background", 0, &["w2"]), {"text": "  ", "basis": "paper"}],
            "after": [claim("mystery", 0, &[])],
            "read_alongside": [],
            "reception_note": null
        }});
        let counts = settle(&mut draft, &dossier()).expect("context");
        let context = &draft["context"];
        assert_eq!(context["lead"][0]["basis"], "paper");
        assert_eq!(context["lead"][1]["basis"], "background");
        assert_eq!(context["lead"][1]["downgraded"], "quote_not_found");
        assert_eq!(context["before"][0]["basis"], "dossier");
        assert_eq!(context["before"][0]["work_ids"], json!(["w1"]));
        assert_eq!(context["before"][1]["basis"], "background");
        assert_eq!(context["before"][1]["downgraded"], "unknown_work");
        assert_eq!(context["contribution"].as_array().map(Vec::len), Some(1));
        assert_eq!(context["contribution"][0]["work_ids"], json!(["w2"]));
        assert_eq!(context["after"][0]["basis"], "background");
        assert!(context["after"][0].get("downgraded").is_none());
        assert_eq!(counts.paper_claims, 1);
        assert_eq!(counts.dossier_claims, 1);
        assert_eq!(counts.background_claims, 4);
        assert_eq!(counts.downgraded_quote_not_found, 1);
        assert_eq!(counts.downgraded_unknown_work, 1);
        assert_eq!(counts.unknown_work_ids, 2);
        assert_eq!(counts.dossier_works, 2);
        assert_eq!(context["counts"]["unknown_work_ids"], 2);
    }

    #[test]
    fn should_match_reading_suggestions_to_the_dossier_and_cap_them() {
        let suggestion = |id: Option<&str>, title: &str, basis: &str| {
            json!({"work_id": id, "title": title, "authors": null, "year": null,
                   "relation": "predecessor", "why": " Read it  first. ", "basis": basis})
        };
        let mut suggestions = vec![
            suggestion(Some("w1"), "old model, misremembered", "dossier"),
            suggestion(Some("w404"), "An Invented Work", "dossier"),
            suggestion(None, "A Known Classic", "background"),
            suggestion(Some("w1"), "Duplicate", "dossier"),
            suggestion(None, "", "background"),
        ];
        suggestions
            .extend((0..6).map(|index| suggestion(None, &format!("Extra {index}"), "background")));
        let mut draft = json!({"context": {
            "lead": [], "before": [], "contribution": [], "after": [],
            "read_alongside": suggestions, "reception_note": "  Too recent to say. "
        }});
        let counts = settle(&mut draft, &dossier()).expect("context");
        let kept = draft["context"]["read_alongside"].as_array().expect("list");
        assert_eq!(kept.len(), READ_ALONGSIDE_LIMIT);
        assert_eq!(kept[0]["title"], "The Old Model");
        assert_eq!(kept[0]["year"], 1974);
        assert_eq!(kept[0]["authors"], "A. Smith et al.");
        assert_eq!(kept[0]["local_paper_id"], "p1");
        assert_eq!(kept[0]["why"], "Read it first.");
        assert_eq!(kept[1]["title"], "An Invented Work");
        assert!(kept[1]["work_id"].is_null());
        assert_eq!(kept[1]["basis"], "background");
        assert_eq!(counts.unknown_work_ids, 1);
        assert_eq!(
            counts.dropped_read_alongside, 5,
            "duplicate, untitled, three past the cap"
        );
        assert_eq!(draft["context"]["reception_note"], "Too recent to say.");
    }

    #[test]
    fn should_leave_a_draft_without_context_alone() {
        let mut draft = json!({"card": {}});
        assert!(settle(&mut draft, &Dossier::default()).is_none());
        assert_eq!(draft, json!({"card": {}}));
    }

    #[test]
    fn should_brief_the_core_draft_without_its_quotes() {
        let draft = json!({
            "identity": {"title": "T", "authors": [{"name": "A", "affiliation": null}], "year": 1990, "venue": null},
            "card": {"thesis": {"text": "It works.", "evidence": [{"quote": "secret quote", "page": 1}]},
                     "position": {"predecessors": [{"reference_key": "1", "cited_work": "Old", "why": "w", "evidence": []}],
                                  "targets": [], "replications": []}},
            "relations": [{"reference_key": "1", "relation": "extends", "why": "w", "quote": "secret quote", "page": 1}],
            "references": [{"key": "1", "title": "Old", "authors": [], "year": 1980, "venue": null, "doi": null, "page": 3}]
        });
        let brief = core_brief(draft.as_object().expect("object")).expect("brief");
        assert!(brief.contains("\"thesis\":\"It works.\""), "{brief}");
        assert!(brief.contains("\"authors\":[\"A\"]"));
        assert!(brief.contains("\"cited_work\":\"Old\""));
        assert!(!brief.contains("secret quote"));
        assert!(core_brief(&Map::new()).is_none());
    }
}
