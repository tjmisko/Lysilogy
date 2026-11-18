//! Keeping the two segmentations consistent wherever the draft changes.
//!
//! The outline is admitted against the page text as soon as the outline call
//! answers, and the logical units are repaired against the admitted outline
//! as soon as the sections call answers, so plausibility checks, later calls
//! and the self-check all see settled structure. The self-check may flag the
//! outline and the grouping but not rewrite them: its patches to derived or
//! structural fields become flags, because a changed outline would silently
//! shift every unit's range.

use serde_json::{Map, Value};

use super::{
    corrections::{Action, Correction},
    grouping,
    outline::{self, Entry},
};
use crate::domain::{ExtractedPage, QualityWarning, QualityWarningKind};

/// The section fields a self-check may still replace on a grouped unit.
const EDITABLE_UNIT_FIELDS: &[&str] = &["title", "summary", "question", "kind", "family"];

/// The admitted outline, when it is rich enough to group logical units from.
#[must_use]
pub fn usable_outline(outline: Option<&Value>, pages: &[ExtractedPage]) -> Option<Vec<Entry>> {
    let entries = outline::entries(outline?, pages);
    outline::usable(&entries, pages.len()).then_some(entries)
}

/// Settle one call's answer: admit its outline, then repair its sections
/// against that outline or the one already in `draft`.
pub fn settle_fragment(
    fragment: &mut Value,
    draft: &Map<String, Value>,
    pages: &[ExtractedPage],
) -> Vec<QualityWarning> {
    let Some(fields) = fragment.as_object_mut() else {
        return Vec::new();
    };
    if let Some(value) = fields.get_mut("outline") {
        outline::admit(value, pages);
    }
    let Some(Value::Array(units)) = fields.get("sections") else {
        return Vec::new();
    };
    let mut units = units.clone();
    let source = fields.get("outline").or_else(|| draft.get("outline"));
    let entries = usable_outline(source, pages).unwrap_or_default();
    let notes = grouping::repair(&mut units, &entries, pages);
    fields.insert("sections".to_owned(), Value::Array(units));
    repair_warning(&notes).into_iter().collect()
}

/// Settle a whole draft, after the self-check.
pub fn settle_draft(draft: &mut Value, pages: &[ExtractedPage]) -> Vec<QualityWarning> {
    settle_fragment(draft, &Map::new(), pages)
}

fn repair_warning(notes: &[String]) -> Option<QualityWarning> {
    (!notes.is_empty()).then(|| QualityWarning {
        kind: QualityWarningKind::SectionsGroupingRepaired,
        field: "sections".to_owned(),
        message: format!(
            "The grouping of outline entries into reading units was repaired: {}",
            notes.join(" ")
        ),
    })
}

/// The self-check's corrections with every patch to the outline, and to the
/// range, pages or span of a grouped unit, turned into a flag.
#[must_use]
pub fn guard_corrections(corrections: &[Correction], draft: &Value) -> Vec<Correction> {
    let grouped = draft
        .get("sections")
        .and_then(Value::as_array)
        .is_some_and(|units| grouping::is_grouped(units));
    corrections
        .iter()
        .cloned()
        .map(|mut correction| {
            if correction.action != Action::Flag && structural(&correction, grouped) {
                correction.action = Action::Flag;
                correction.value_json = None;
            }
            correction
        })
        .collect()
}

fn structural(correction: &Correction, grouped: bool) -> bool {
    let path = correction.path.as_str();
    if path == "/outline" || path.starts_with("/outline/") {
        return true;
    }
    if !grouped || !(path == "/sections" || path.starts_with("/sections/")) {
        return false;
    }
    // Only a named field of one unit stays editable.
    let mut segments = path.trim_start_matches('/').split('/').skip(1);
    let unit = segments.next();
    let field = segments.next();
    !(unit.is_some_and(|unit| unit.parse::<usize>().is_ok())
        && field.is_some_and(|field| EDITABLE_UNIT_FIELDS.contains(&field)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pages() -> Vec<ExtractedPage> {
        [
            "1 Introduction\nWe study the effect of things on other things today.",
            "2 Model\nWe write down a model of things and their many effects.",
            "3 Results\nThe effect is large and robust across many samples.",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| ExtractedPage {
            number: u32::try_from(index + 1).expect("page"),
            text: (*text).to_owned(),
        })
        .collect()
    }

    fn correction(path: &str, action: Action) -> Correction {
        Correction {
            path: path.to_owned(),
            action,
            value_json: Some("\"x\"".to_owned()),
            reason: "r".to_owned(),
            evidence: Vec::new(),
        }
    }

    #[test]
    fn should_settle_a_sections_answer_against_the_draft_outline() {
        let pages = pages();
        let mut draft = Map::new();
        let mut outline_answer = json!({"outline": [
            {"number": "1", "title": "Introduction", "level": 1, "page": 1, "role": "main"},
            {"number": "2", "title": "Model", "level": 1, "page": 2, "role": "main"},
            {"number": "3", "title": "Results", "level": 1, "page": 3, "role": "main"}
        ]});
        assert!(settle_fragment(&mut outline_answer, &draft, &pages).is_empty());
        draft.insert("outline".to_owned(), outline_answer["outline"].clone());
        let mut sections = json!({"sections": [
            {"title": "Why and how", "kind": "theory", "family": "context", "outline_first": 0, "outline_last": 1, "summary": "s", "question": "q"}
        ]});
        let warnings = settle_fragment(&mut sections, &draft, &pages);
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].kind,
            QualityWarningKind::SectionsGroupingRepaired
        );
        let units = sections["sections"].as_array().expect("units");
        assert_eq!(
            units[0]["outline_last"], 2,
            "the skipped results join the unit"
        );
        assert_eq!(units[0]["pages"], json!({"start": 1, "end": 3}));
    }

    #[test]
    fn should_turn_structural_self_check_patches_into_flags() {
        let grouped = json!({"sections": [{"outline_first": 0, "outline_last": 1}]});
        let corrections = vec![
            correction("/outline/0/title", Action::Replace),
            correction("/sections/0/summary", Action::Replace),
            correction("/sections/0/pages", Action::Replace),
            correction("/sections/0", Action::Replace),
            correction("/sections", Action::Add),
            correction("/identity/title", Action::Replace),
        ];
        let guarded = guard_corrections(&corrections, &grouped)
            .into_iter()
            .map(|correction| correction.action)
            .collect::<Vec<_>>();
        assert_eq!(
            guarded,
            [
                Action::Flag,
                Action::Replace,
                Action::Flag,
                Action::Flag,
                Action::Flag,
                Action::Replace
            ]
        );
        let spans = json!({"sections": [{"outline_first": null, "outline_last": null}]});
        let guarded = guard_corrections(&corrections, &spans);
        assert_eq!(
            guarded[2].action,
            Action::Replace,
            "page units stay editable"
        );
        assert_eq!(guarded[0].action, Action::Flag);
    }
}
