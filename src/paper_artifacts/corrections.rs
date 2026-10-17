//! Pass 2 may only correct or flag a pass-1 draft.
//!
//! Each correction is a JSON Pointer patch with verbatim evidence. A
//! correction is applied only when its evidence is admitted, its pointer
//! resolves, and its value keeps the JSON type of what it replaces, so a bad
//! patch is rejected and counted rather than corrupting the draft.
//!
//! Equations are the exception: the page image is their evidence, and the
//! text layer cannot reproduce math verbatim. A correction to an equation
//! object needs evidence on a page but is applied as image-evidenced, and
//! reported separately.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::admission::{LOCAL_REPAIR_SIMILARITY, SourceText, Verdict};

#[derive(Clone, Debug, Deserialize)]
pub struct CheckResponse {
    pub corrections: Vec<Correction>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Correction {
    pub path: String,
    pub action: Action,
    pub value_json: Option<String>,
    pub reason: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Replace,
    Remove,
    Add,
    Flag,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Evidence {
    pub quote: String,
    pub page: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    /// Applied to an equation on image evidence; the text cannot verify it.
    AppliedImageEvidence,
    Flagged,
    Rejected(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct CorrectionOutcome {
    pub index: usize,
    pub path: String,
    pub action: Action,
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// Apply `corrections` to `draft`: replacements, then additions, then
/// removals from the highest index down, so no patch moves another's target.
pub fn apply(
    draft: &mut Value,
    corrections: &[Correction],
    source: &SourceText<'_>,
) -> Vec<CorrectionOutcome> {
    let mut outcomes = Vec::with_capacity(corrections.len());
    let mut removals = Vec::<(usize, String)>::new();
    let order = [Action::Flag, Action::Replace, Action::Add, Action::Remove];
    for action in order {
        for (index, correction) in corrections.iter().enumerate() {
            if correction.action != action {
                continue;
            }
            let image_evidence = targets_equation(draft, correction);
            let evidence = if image_evidence {
                if correction.evidence.is_empty() {
                    Err("no evidence".to_owned())
                } else {
                    Ok(())
                }
            } else {
                admitted(correction, source)
            };
            let outcome = match evidence {
                Err(reason) => Outcome::Rejected(reason),
                Ok(()) => match action {
                    Action::Flag => Outcome::Flagged,
                    Action::Replace => replace(draft, correction),
                    Action::Add => add(draft, correction),
                    Action::Remove => {
                        let outcome = removable(draft, &correction.path);
                        if outcome == Outcome::Applied {
                            removals.push((index, correction.path.clone()));
                        }
                        outcome
                    }
                },
            };
            let outcome = match outcome {
                Outcome::Applied if image_evidence => Outcome::AppliedImageEvidence,
                other => other,
            };
            outcomes.push(CorrectionOutcome {
                index,
                path: correction.path.clone(),
                action,
                outcome,
            });
        }
    }
    let mut targets = removals
        .iter()
        .filter_map(|(_, path)| {
            let (parent, index) = path.rsplit_once('/')?;
            Some((parent.to_owned(), index.parse::<usize>().ok()?))
        })
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.cmp(&left.1)));
    targets.dedup();
    for (parent, index) in targets {
        if let Some(Value::Array(items)) = draft.pointer_mut(&parent)
            && index < items.len()
        {
            items.remove(index);
        }
    }
    outcomes.sort_by_key(|outcome| outcome.index);
    outcomes
}

/// Whether the correction changes an existing equation object or adds one.
fn targets_equation(draft: &Value, correction: &Correction) -> bool {
    let is_equation =
        |object: &Value| object.get("kind").and_then(Value::as_str) == Some("equation");
    if correction.action == Action::Add && correction.path == "/objects" {
        return correction
            .value_json
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .is_some_and(|value| is_equation(&value));
    }
    let Some(rest) = correction.path.strip_prefix("/objects/") else {
        return false;
    };
    let index = rest.split('/').next().unwrap_or_default();
    draft
        .pointer(&format!("/objects/{index}"))
        .is_some_and(is_equation)
}

fn admitted(correction: &Correction, source: &SourceText<'_>) -> Result<(), String> {
    if correction.evidence.is_empty() {
        return Err("no evidence".to_owned());
    }
    let supported = correction.evidence.iter().any(|evidence| {
        match source.verdict(&evidence.quote, Some(evidence.page)) {
            Verdict::Verbatim { .. } => true,
            Verdict::Near(found) => found.similarity() >= LOCAL_REPAIR_SIMILARITY,
            Verdict::Missing => false,
        }
    });
    if supported {
        Ok(())
    } else {
        Err("evidence is not in the source".to_owned())
    }
}

fn parsed_value(correction: &Correction) -> Result<Value, Outcome> {
    let Some(text) = correction.value_json.as_deref() else {
        return Err(Outcome::Rejected("missing value_json".to_owned()));
    };
    serde_json::from_str(text)
        .map_err(|error| Outcome::Rejected(format!("value_json is not JSON: {error}")))
}

fn replace(draft: &mut Value, correction: &Correction) -> Outcome {
    if correction.path.is_empty() || correction.path == "/" {
        return Outcome::Rejected("cannot replace the whole draft".to_owned());
    }
    let value = match parsed_value(correction) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let Some(slot) = draft.pointer_mut(&correction.path) else {
        return Outcome::Rejected("path does not exist".to_owned());
    };
    if !compatible(slot, &value) {
        return Outcome::Rejected("value changes the field's JSON type".to_owned());
    }
    *slot = value;
    Outcome::Applied
}

fn add(draft: &mut Value, correction: &Correction) -> Outcome {
    let value = match parsed_value(correction) {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };
    let Some(Value::Array(items)) = draft.pointer_mut(&correction.path) else {
        return Outcome::Rejected("add must point at an array".to_owned());
    };
    if items.first().is_some_and(|first| !same_kind(first, &value)) {
        return Outcome::Rejected("value does not match the array's elements".to_owned());
    }
    items.push(value);
    Outcome::Applied
}

fn removable(draft: &Value, path: &str) -> Outcome {
    let Some((parent, index)) = path.rsplit_once('/') else {
        return Outcome::Rejected("path does not exist".to_owned());
    };
    let Ok(index) = index.parse::<usize>() else {
        return Outcome::Rejected("remove only applies to array elements".to_owned());
    };
    match draft.pointer(parent) {
        Some(Value::Array(items)) if index < items.len() => Outcome::Applied,
        Some(Value::Array(_)) | None => Outcome::Rejected("path does not exist".to_owned()),
        Some(_) => Outcome::Rejected("remove only applies to array elements".to_owned()),
    }
}

/// Null may replace or be replaced by anything the schema allows; otherwise
/// the JSON kind must not change.
const fn compatible(old: &Value, new: &Value) -> bool {
    matches!(old, Value::Null) || matches!(new, Value::Null) || same_kind(old, new)
}

const fn same_kind(left: &Value, right: &Value) -> bool {
    matches!(
        (left, right),
        (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Array(_), Value::Array(_))
            | (Value::Object(_), Value::Object(_))
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::ExtractedPage;

    fn pages() -> Vec<ExtractedPage> {
        vec![ExtractedPage {
            number: 1,
            text: "Deep Residual Learning for Image Recognition\nKaiming He, Xiangyu Zhang\nTable 1: Error rates on ImageNet.".to_owned(),
        }]
    }

    fn draft() -> Value {
        json!({
            "identity": {"title": "Deep Residual Learning", "year": null},
            "objects": [{"label": "Table 1"}, {"label": "Table 2"}, {"label": "Table 3"}],
            "key_numbers": []
        })
    }

    fn correction(path: &str, action: Action, value: Option<&str>, quote: &str) -> Correction {
        Correction {
            path: path.to_owned(),
            action,
            value_json: value.map(str::to_owned),
            reason: "r".to_owned(),
            evidence: if quote.is_empty() {
                Vec::new()
            } else {
                vec![Evidence {
                    quote: quote.to_owned(),
                    page: 1,
                }]
            },
        }
    }

    const TITLE: &str = "Deep Residual Learning for Image Recognition";

    #[test]
    fn should_replace_a_field_when_the_evidence_is_verbatim() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[correction(
                "/identity/title",
                Action::Replace,
                Some(&format!("\"{TITLE}\"")),
                TITLE,
            )],
            &SourceText::new(&pages),
        );
        assert_eq!(outcomes[0].outcome, Outcome::Applied);
        assert_eq!(value["identity"]["title"], TITLE);
    }

    #[test]
    fn should_reject_a_correction_without_evidence_or_with_invented_evidence() {
        let pages = pages();
        let source = SourceText::new(&pages);
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[
                correction("/identity/title", Action::Replace, Some("\"X\""), ""),
                correction(
                    "/identity/title",
                    Action::Replace,
                    Some("\"X\""),
                    "a sentence that never appears anywhere",
                ),
            ],
            &source,
        );
        assert!(
            matches!(&outcomes[0].outcome, Outcome::Rejected(reason) if reason == "no evidence")
        );
        assert!(
            matches!(&outcomes[1].outcome, Outcome::Rejected(reason) if reason.contains("not in the source"))
        );
        assert_eq!(value, draft());
    }

    #[test]
    fn should_reject_a_replacement_that_changes_the_json_type() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[correction(
                "/identity/title",
                Action::Replace,
                Some("[1, 2]"),
                TITLE,
            )],
            &SourceText::new(&pages),
        );
        assert!(matches!(outcomes[0].outcome, Outcome::Rejected(_)));
        assert_eq!(value, draft());
    }

    #[test]
    fn should_allow_null_to_become_a_value() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[correction(
                "/identity/year",
                Action::Replace,
                Some("2015"),
                TITLE,
            )],
            &SourceText::new(&pages),
        );
        assert_eq!(outcomes[0].outcome, Outcome::Applied);
        assert_eq!(value["identity"]["year"], 2015);
    }

    #[test]
    fn should_reject_bad_json_missing_paths_and_whole_draft_replacement() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[
                correction("/identity/title", Action::Replace, Some("not json"), TITLE),
                correction("/identity/missing", Action::Replace, Some("\"x\""), TITLE),
                correction("", Action::Replace, Some("{}"), TITLE),
                correction("/identity/title", Action::Replace, None, TITLE),
            ],
            &SourceText::new(&pages),
        );
        assert!(
            outcomes
                .iter()
                .all(|outcome| matches!(outcome.outcome, Outcome::Rejected(_)))
        );
        assert_eq!(value, draft());
    }

    #[test]
    fn should_remove_several_elements_without_shifting_targets() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[
                correction(
                    "/objects/0",
                    Action::Remove,
                    None,
                    "Table 1: Error rates on ImageNet.",
                ),
                correction(
                    "/objects/2",
                    Action::Remove,
                    None,
                    "Table 1: Error rates on ImageNet.",
                ),
            ],
            &SourceText::new(&pages),
        );
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.outcome == Outcome::Applied)
        );
        assert_eq!(value["objects"], json!([{"label": "Table 2"}]));
    }

    #[test]
    fn should_reject_removing_an_object_field_or_an_out_of_range_element() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[
                correction("/identity/title", Action::Remove, None, TITLE),
                correction("/objects/7", Action::Remove, None, TITLE),
            ],
            &SourceText::new(&pages),
        );
        assert!(
            outcomes
                .iter()
                .all(|outcome| matches!(outcome.outcome, Outcome::Rejected(_)))
        );
        assert_eq!(value, draft());
    }

    #[test]
    fn should_add_to_an_array_only_when_the_element_matches() {
        let pages = pages();
        let mut value = draft();
        let quote = "Table 1: Error rates on ImageNet.";
        let outcomes = apply(
            &mut value,
            &[
                correction(
                    "/objects",
                    Action::Add,
                    Some(r#"{"label": "Table 4"}"#),
                    quote,
                ),
                correction("/objects", Action::Add, Some("\"Table 5\""), quote),
                correction("/identity", Action::Add, Some("{}"), quote),
                correction(
                    "/key_numbers",
                    Action::Add,
                    Some(r#"{"value": "3.57%"}"#),
                    quote,
                ),
            ],
            &SourceText::new(&pages),
        );
        let statuses = outcomes
            .iter()
            .map(|outcome| &outcome.outcome)
            .collect::<Vec<_>>();
        assert_eq!(statuses[0], &Outcome::Applied);
        assert!(matches!(statuses[1], Outcome::Rejected(_)));
        assert!(matches!(statuses[2], Outcome::Rejected(_)));
        assert_eq!(statuses[3], &Outcome::Applied);
        assert_eq!(value["objects"].as_array().map(Vec::len), Some(4));
    }

    #[test]
    fn should_apply_an_equation_correction_on_image_evidence_and_label_it() {
        let pages = pages();
        let mut value = json!({"objects": [
            {"kind": "equation", "label": "(3)", "latex": "a=b", "page": 1},
            {"kind": "table", "label": "Table 1", "latex": null, "page": 1}
        ]});
        let math = "− ln |K ε l | ≤ 1 2 ln(ε −1)";
        let outcomes = apply(
            &mut value,
            &[
                correction(
                    "/objects/0/latex",
                    Action::Replace,
                    Some(r#""a=b+c""#),
                    math,
                ),
                correction("/objects/0/page", Action::Replace, Some("2"), ""),
                correction("/objects/1/page", Action::Replace, Some("2"), math),
                correction(
                    "/objects",
                    Action::Add,
                    Some(r#"{"kind": "equation", "label": "(4)", "latex": "x", "page": 1}"#),
                    math,
                ),
                correction(
                    "/objects",
                    Action::Add,
                    Some(r#"{"kind": "figure", "label": "Figure 9", "latex": null, "page": 1}"#),
                    math,
                ),
            ],
            &SourceText::new(&pages),
        );
        let statuses = outcomes
            .iter()
            .map(|outcome| outcome.outcome.clone())
            .collect::<Vec<_>>();
        assert_eq!(statuses[0], Outcome::AppliedImageEvidence);
        assert!(matches!(&statuses[1], Outcome::Rejected(reason) if reason == "no evidence"));
        assert!(
            matches!(&statuses[2], Outcome::Rejected(reason) if reason.contains("not in the source"))
        );
        assert_eq!(statuses[3], Outcome::AppliedImageEvidence);
        assert!(matches!(statuses[4], Outcome::Rejected(_)));
        assert_eq!(value["objects"][0]["latex"], "a=b+c");
        assert_eq!(value["objects"][1]["page"], 1);
        assert_eq!(value["objects"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn should_record_a_flag_without_changing_the_draft() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[correction("/identity/title", Action::Flag, None, TITLE)],
            &SourceText::new(&pages),
        );
        assert_eq!(outcomes[0].outcome, Outcome::Flagged);
        assert_eq!(value, draft());
    }

    #[test]
    fn should_report_outcomes_in_the_order_the_model_gave() {
        let pages = pages();
        let mut value = draft();
        let outcomes = apply(
            &mut value,
            &[
                correction("/objects/1", Action::Remove, None, TITLE),
                correction("/identity/title", Action::Flag, None, TITLE),
                correction(
                    "/identity/title",
                    Action::Replace,
                    Some(&format!("\"{TITLE}\"")),
                    TITLE,
                ),
            ],
            &SourceText::new(&pages),
        );
        assert_eq!(
            outcomes
                .iter()
                .map(|outcome| outcome.index)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }
}
