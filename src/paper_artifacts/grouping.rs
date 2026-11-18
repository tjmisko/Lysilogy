//! Logical reading units as contiguous ranges of the printed outline.
//!
//! When a paper has a usable outline, the model is asked only which runs of
//! outline entries belong together, plus a title, a synthesis and the
//! question each unit answers. Everything structural is checked and derived
//! here: ranges are put in order, overlaps are clipped, main-text entries the
//! model skipped are absorbed into a neighbour, references and appendices
//! end up in at most one unit each, and pages and source spans come from the
//! outline rather than from the model. Each repair is reported in plain
//! language. A unit can only start at a printed heading, so a long section
//! is split only at one of its sub-headings.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::outline::{Entry, furniture, furniture_key};
use crate::domain::ExtractedPage;

/// Words taken for a derived end phrase.
const END_PHRASE_WORDS: usize = 8;

/// How many pages back a derived end phrase may be looked for.
const END_PHRASE_PAGES: u32 = 2;

/// One unit while its range is being repaired.
struct Unit {
    fields: Map<String, Value>,
    first: usize,
    last: usize,
}

impl Unit {
    fn title(&self) -> String {
        self.fields
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled")
            .to_owned()
    }
}

fn index(fields: &Map<String, Value>, key: &str) -> Option<usize> {
    fields
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

/// Whether any unit names an outline range: the grouped shape.
#[must_use]
pub fn is_grouped(units: &[Value]) -> bool {
    units.iter().any(|unit| {
        unit.get("outline_first").is_some_and(Value::is_u64)
            || unit.get("outline_last").is_some_and(Value::is_u64)
    })
}

/// Mark page-span units as made without an outline, so every stored unit has
/// the same shape.
fn clear_ranges(units: &mut [Value]) {
    for unit in units {
        if let Some(fields) = unit.as_object_mut() {
            fields.insert("outline_first".to_owned(), Value::Null);
            fields.insert("outline_last".to_owned(), Value::Null);
        }
    }
}

/// Repair grouped units against `outline` in place and derive their pages
/// and source spans. Returns what was repaired, in plain language.
///
/// With an empty outline (none, or too sparse to group from) the units are
/// page-span units and keep the model's own pages; their outline indices
/// are cleared. Running this twice changes nothing.
pub fn repair(units: &mut Vec<Value>, outline: &[Entry], pages: &[ExtractedPage]) -> Vec<String> {
    if outline.is_empty() || !is_grouped(units) {
        clear_ranges(units);
        return Vec::new();
    }
    let mut notes = Vec::new();
    let mut grouped = parse(units, outline.len(), &mut notes);
    grouped.sort_by_key(|unit| (unit.first, unit.last));
    clip_overlaps(&mut grouped, &mut notes);
    separate_special(&mut grouped, outline, &mut notes);
    merge_special(&mut grouped, outline, &mut notes);
    absorb_gaps(&mut grouped, outline, &mut notes);
    grouped.sort_by_key(|unit| unit.first);
    let furniture = furniture(pages);
    *units = grouped
        .into_iter()
        .map(|unit| derive(unit, outline, pages, &furniture))
        .collect();
    notes
}

fn parse(units: &[Value], len: usize, notes: &mut Vec<String>) -> Vec<Unit> {
    let mut parsed = Vec::new();
    for unit in units {
        let Some(fields) = unit.as_object() else {
            continue;
        };
        let range = match (
            index(fields, "outline_first"),
            index(fields, "outline_last"),
        ) {
            (Some(first), Some(last)) => Some((first.min(last), first.max(last))),
            (Some(only), None) | (None, Some(only)) => Some((only, only)),
            (None, None) => None,
        };
        let title = fields
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("untitled");
        match range {
            Some((first, last)) if last < len => parsed.push(Unit {
                fields: fields.clone(),
                first,
                last,
            }),
            _ => notes.push(format!(
                "Dropped the unit \"{title}\": its outline range was missing or outside the {len} outline entries."
            )),
        }
    }
    parsed
}

fn clip_overlaps(units: &mut Vec<Unit>, notes: &mut Vec<String>) {
    let mut kept: Vec<Unit> = Vec::with_capacity(units.len());
    for mut unit in units.drain(..) {
        if let Some(previous) = kept.last()
            && unit.first <= previous.last
        {
            let previous_title = previous.title();
            unit.first = previous.last + 1;
            if unit.first > unit.last {
                notes.push(format!(
                    "Dropped the unit \"{}\": its outline entries were all inside \"{previous_title}\".",
                    unit.title()
                ));
                continue;
            }
            notes.push(format!(
                "Clipped \"{}\" to start at outline entry {}, after \"{previous_title}\", which it overlapped.",
                unit.title(),
                unit.first
            ));
        }
        kept.push(unit);
    }
    *units = kept;
}

/// The role of the entry at `index`, special roles only.
fn special_role(outline: &[Entry], index: usize) -> Option<&str> {
    outline
        .get(index)
        .filter(|entry| entry.is_special())
        .map(|entry| entry.role.as_str())
}

/// A unit may not straddle the main text and the references or appendices:
/// it keeps the run its first entry belongs to.
fn separate_special(units: &mut [Unit], outline: &[Entry], notes: &mut Vec<String>) {
    for unit in units.iter_mut() {
        let role = special_role(outline, unit.first);
        let end = (unit.first..=unit.last)
            .find(|&index| special_role(outline, index) != role)
            .map(|index| index - 1);
        if let Some(end) = end {
            notes.push(format!(
                "Ended \"{}\" at outline entry {end}: it ran from {} into {}.",
                unit.title(),
                role.map_or("the main text", |role| role),
                special_role(outline, end + 1).map_or("the main text", |role| role)
            ));
            unit.last = end;
        }
    }
}

/// References and appendices form at most one unit each.
fn merge_special(units: &mut Vec<Unit>, outline: &[Entry], notes: &mut Vec<String>) {
    for role in ["references", "appendix"] {
        let mut owner: Option<usize> = None;
        let mut position = 0;
        while position < units.len() {
            if special_role(outline, units[position].first) != Some(role) {
                position += 1;
                continue;
            }
            let Some(owner_position) = owner else {
                owner = Some(position);
                position += 1;
                continue;
            };
            let from = units[owner_position].last + 1;
            let to = units[position].first;
            let joinable = (from..to).all(|index| special_role(outline, index) == Some(role));
            let merged = units.remove(position);
            if joinable {
                units[owner_position].last = merged.last;
                notes.push(format!(
                    "Merged \"{}\" into \"{}\": the {role} are one unit.",
                    merged.title(),
                    units[owner_position].title()
                ));
            } else {
                notes.push(format!(
                    "Dropped the unit \"{}\": the {role} are one unit, already \"{}\".",
                    merged.title(),
                    units[owner_position].title()
                ));
            }
        }
    }
}

/// Every main-text entry belongs to a unit: a run the model skipped joins
/// the unit before it, else the unit after it, else becomes its own unit.
fn absorb_gaps(units: &mut Vec<Unit>, outline: &[Entry], notes: &mut Vec<String>) {
    loop {
        units.sort_by_key(|unit| unit.first);
        let covered = units
            .iter()
            .flat_map(|unit| unit.first..=unit.last)
            .collect::<BTreeSet<_>>();
        let Some(start) =
            (0..outline.len()).find(|index| !covered.contains(index) && outline[*index].is_main())
        else {
            return;
        };
        let mut gap_start = start;
        while gap_start > 0 && !covered.contains(&(gap_start - 1)) {
            gap_start -= 1;
        }
        let mut gap_end = start;
        while gap_end + 1 < outline.len() && !covered.contains(&(gap_end + 1)) {
            gap_end += 1;
        }
        let first_main = (gap_start..=gap_end)
            .find(|index| outline[*index].is_main())
            .unwrap_or(start);
        let last_main = (gap_start..=gap_end)
            .rev()
            .find(|index| outline[*index].is_main())
            .unwrap_or(start);
        let names = (first_main..=last_main)
            .map(|index| format!("\"{}\"", outline[index].anchor))
            .collect::<Vec<_>>()
            .join(", ");
        let before = units.iter().position(|unit| {
            gap_start > 0
                && unit.last == gap_start - 1
                && special_role(outline, unit.first).is_none()
        });
        let after = units.iter().position(|unit| {
            unit.first == gap_end + 1 && special_role(outline, unit.first).is_none()
        });
        if let Some(position) = before {
            units[position].last = last_main;
            notes.push(format!(
                "Added skipped heading(s) {names} to \"{}\".",
                units[position].title()
            ));
        } else if let Some(position) = after {
            units[position].first = first_main;
            notes.push(format!(
                "Added skipped heading(s) {names} to \"{}\".",
                units[position].title()
            ));
        } else {
            let entry = &outline[first_main];
            let mut fields = Map::new();
            fields.insert("title".to_owned(), Value::from(entry.title.clone()));
            fields.insert("kind".to_owned(), Value::from("other"));
            fields.insert("family".to_owned(), Value::from("evidence"));
            fields.insert("summary".to_owned(), Value::from(""));
            fields.insert("question".to_owned(), Value::from(""));
            notes.push(format!(
                "Made a unit from skipped heading(s) {names}; it has no synthesis."
            ));
            units.push(Unit {
                fields,
                first: first_main,
                last: last_main,
            });
        }
    }
}

/// A unit with its outline range written back and its pages and source span
/// derived from the outline.
fn derive(
    unit: Unit,
    outline: &[Entry],
    pages: &[ExtractedPage],
    furniture: &BTreeSet<String>,
) -> Value {
    let Unit {
        mut fields,
        first,
        last,
    } = unit;
    let start = &outline[first];
    let last_page = pages.last().map_or(start.page, |page| page.number);
    // The unit ends just before the next heading, or at the end of the paper.
    let (boundary_page, boundary_line) = outline
        .get(last + 1)
        .map_or((last_page, usize::MAX), |next| (next.page, next.line));
    let (end_page, end_text) = text_before(pages, boundary_page, boundary_line, furniture)
        .unwrap_or_else(|| (start.page, start.anchor.clone()));
    let end_page = end_page.max(start.page);
    fields.insert("outline_first".to_owned(), Value::from(first));
    fields.insert("outline_last".to_owned(), Value::from(last));
    fields.insert(
        "pages".to_owned(),
        serde_json::json!({"start": start.page, "end": end_page}),
    );
    fields.insert(
        "source_span".to_owned(),
        serde_json::json!({
            "start_text": start.anchor,
            "start_page": start.page,
            "end_text": end_text,
            "end_page": end_page,
        }),
    );
    match special_role(outline, first) {
        Some("references") => {
            fields.insert("kind".to_owned(), Value::from("references"));
            fields.insert("family".to_owned(), Value::from("reference"));
        }
        Some("appendix") => {
            fields.insert("kind".to_owned(), Value::from("appendix"));
        }
        _ => {}
    }
    for key in ["summary", "question"] {
        fields.entry(key).or_insert_with(|| Value::from(""));
    }
    Value::Object(fields)
}

/// The last words of prose printed before line `line` of page `page`, looking
/// back up to two pages, and the page they are on. Running heads, page
/// numbers and other short lines are skipped.
fn text_before(
    pages: &[ExtractedPage],
    page: u32,
    line: usize,
    furniture: &BTreeSet<String>,
) -> Option<(u32, String)> {
    let floor = page.saturating_sub(END_PHRASE_PAGES);
    for number in (floor..=page).rev() {
        let Some(current) = pages.iter().find(|candidate| candidate.number == number) else {
            continue;
        };
        let lines = current.text.lines().collect::<Vec<_>>();
        let end = if number == page {
            line.min(lines.len())
        } else {
            lines.len()
        };
        let prose = lines[..end].iter().rev().find(|text| {
            let words = text.split_whitespace().count();
            let letters = text.chars().filter(|c| c.is_alphabetic()).count();
            words >= 3 && letters >= 8 && !furniture.contains(&furniture_key(text))
        });
        if let Some(prose) = prose {
            let words = prose.split_whitespace().collect::<Vec<_>>();
            let tail = words[words.len().saturating_sub(END_PHRASE_WORDS)..].join(" ");
            return Some((number, tail));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pages() -> Vec<ExtractedPage> {
        let texts = [
            "Abstract\nWe study the effect of things on other things.\n1 Introduction\nThings matter a great deal to many people.",
            "2 Model\nWe write down a model of things and their effects.\n2.1 Assumptions\nThe assumptions are standard in this literature.",
            "3 Data\nThe data cover forty years of things in many places.\n4 Results\nThe effect is large and robust across samples.",
            "5 Conclusion\nThings have large effects on other things overall.\nReferences\nAuthor, A. A paper about things. Journal of Things.",
            "Appendix A Proofs\nThe proof of the main result follows directly here.\nAppendix B Data\nThe data appendix describes every source in detail.",
        ];
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| ExtractedPage {
                number: u32::try_from(index + 1).expect("page"),
                text: (*text).to_owned(),
            })
            .collect()
    }

    fn outline(pages: &[ExtractedPage]) -> Vec<Entry> {
        let mut value = json!([
            {"number": null, "title": "Abstract", "level": 1, "page": 1, "role": "front_matter"},
            {"number": "1", "title": "Introduction", "level": 1, "page": 1, "role": "main"},
            {"number": "2", "title": "Model", "level": 1, "page": 2, "role": "main"},
            {"number": "2.1", "title": "Assumptions", "level": 2, "page": 2, "role": "main"},
            {"number": "3", "title": "Data", "level": 1, "page": 3, "role": "main"},
            {"number": "4", "title": "Results", "level": 1, "page": 3, "role": "main"},
            {"number": "5", "title": "Conclusion", "level": 1, "page": 4, "role": "main"},
            {"number": null, "title": "References", "level": 1, "page": 4, "role": "references"},
            {"number": null, "title": "Appendix A Proofs", "level": 1, "page": 5, "role": "appendix"},
            {"number": null, "title": "Appendix B Data", "level": 1, "page": 5, "role": "appendix"}
        ]);
        super::super::outline::admit(&mut value, pages).entries
    }

    fn unit(title: &str, first: Option<usize>, last: Option<usize>) -> Value {
        json!({
            "title": title, "kind": "other", "family": "evidence",
            "outline_first": first, "outline_last": last,
            "summary": format!("{title} summary."), "question": "Why?"
        })
    }

    fn ranges(units: &[Value]) -> Vec<(String, u64, u64)> {
        units
            .iter()
            .map(|unit| {
                (
                    unit["title"].as_str().unwrap_or_default().to_owned(),
                    unit["outline_first"].as_u64().unwrap_or(99),
                    unit["outline_last"].as_u64().unwrap_or(99),
                )
            })
            .collect()
    }

    #[test]
    fn should_derive_pages_and_spans_from_a_valid_grouping() {
        let pages = pages();
        let outline = outline(&pages);
        assert_eq!(outline.len(), 10);
        let mut units = vec![
            unit("Why things matter", Some(1), Some(1)),
            unit("A model of things", Some(2), Some(3)),
            unit("Things have large effects", Some(4), Some(6)),
            unit("References", Some(7), Some(7)),
        ];
        let notes = repair(&mut units, &outline, &pages);
        assert!(notes.is_empty(), "{notes:#?}");
        assert_eq!(units[0]["pages"], json!({"start": 1, "end": 1}));
        assert_eq!(units[0]["source_span"]["start_text"], "1 Introduction");
        assert_eq!(
            units[0]["source_span"]["end_text"],
            "Things matter a great deal to many people."
        );
        assert_eq!(units[1]["pages"], json!({"start": 2, "end": 2}));
        assert_eq!(units[2]["pages"], json!({"start": 3, "end": 4}));
        assert_eq!(units[3]["kind"], "references");
        assert_eq!(units[3]["family"], "reference");
        // Idempotent.
        let again = units.clone();
        assert!(repair(&mut units, &outline, &pages).is_empty());
        assert_eq!(units, again);
    }

    #[test]
    fn should_repair_gaps_overlaps_and_split_appendices_deterministically() {
        let pages = pages();
        let outline = outline(&pages);
        let mut units = vec![
            // Out of order, and overlapping the model unit.
            unit("Evidence", Some(3), Some(5)),
            unit("Setup", Some(1), Some(3)),
            // Entry 6 (Conclusion) is skipped; this one runs into the references.
            unit("Appendix one", Some(8), Some(8)),
            unit("Appendix two", Some(9), Some(9)),
            unit("Nonsense", Some(40), Some(41)),
        ];
        let notes = repair(&mut units, &outline, &pages);
        assert_eq!(
            ranges(&units),
            [
                ("Setup".to_owned(), 1, 3),
                ("Evidence".to_owned(), 4, 6),
                ("Appendix one".to_owned(), 8, 9),
            ]
        );
        assert_eq!(notes.len(), 4, "{notes:#?}");
        assert!(notes.iter().any(|note| note.contains("Nonsense")));
        assert!(
            notes
                .iter()
                .any(|note| note.contains("Clipped \"Evidence\""))
        );
        assert!(notes.iter().any(|note| note.contains("\"5 Conclusion\"")));
        assert!(
            notes
                .iter()
                .any(|note| note.contains("Merged \"Appendix two\""))
        );
        assert_eq!(units[2]["kind"], "appendix");
    }

    #[test]
    fn should_end_a_unit_where_the_references_begin() {
        let pages = pages();
        let outline = outline(&pages);
        let mut units = vec![
            unit("Everything", Some(1), Some(6)),
            unit("Tail", Some(6), Some(8)),
        ];
        let notes = repair(&mut units, &outline, &pages);
        assert_eq!(
            ranges(&units),
            [("Everything".to_owned(), 1, 6), ("Tail".to_owned(), 7, 7)]
        );
        assert_eq!(units[1]["kind"], "references");
        assert!(
            notes
                .iter()
                .any(|note| note.starts_with("Clipped \"Tail\""))
        );
        assert!(notes.iter().any(|note| note.starts_with("Ended \"Tail\"")));
    }

    #[test]
    fn should_keep_page_span_units_when_there_is_no_outline() {
        let pages = pages();
        let mut units = vec![json!({
            "title": "Everything", "kind": "other", "family": "evidence",
            "pages": {"start": 1, "end": 5}, "summary": "All.", "question": "What?",
            "outline_first": 2, "outline_last": 3
        })];
        assert!(repair(&mut units, &[], &pages).is_empty());
        assert_eq!(units[0]["pages"], json!({"start": 1, "end": 5}));
        assert!(units[0]["outline_first"].is_null());
    }

    #[test]
    fn should_make_a_unit_when_the_model_grouped_nothing_valid() {
        let pages = pages();
        let outline = outline(&pages);
        let mut units = vec![unit("Lost", Some(70), Some(80))];
        let notes = repair(&mut units, &outline, &pages);
        assert_eq!(ranges(&units), [("Introduction".to_owned(), 1, 6)]);
        assert!(notes.iter().any(|note| note.contains("no synthesis")));
    }
}
