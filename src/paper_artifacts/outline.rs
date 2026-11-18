//! The paper's own headings as printed: the outline view.
//!
//! Most of the outline is found deterministically. Heading candidates come
//! from the page text (numbering such as `2`, `2.1`, `IV.` or `A.`, the usual
//! unnumbered headings such as "Introduction", short all-caps lines) and from
//! the layout (lines set in larger type than the body). The model only
//! confirms, rejects or corrects candidates and fills levels and roles, and
//! every heading it returns is admitted here by matching its printed line on
//! its page. A heading that cannot be found in the page text is dropped. A
//! paper that prints no headings gets an empty outline, which is a valid
//! result.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::{
    domain::{DocumentLayout, ExtractedPage},
    layout::canonical_chars,
};

/// Lines longer than this are prose, not headings.
const MAX_HEADING_CHARS: usize = 100;

/// Heading text, without its number, has at most this many words.
const MAX_HEADING_WORDS: usize = 12;

/// A layout line this much taller than the body type is set larger.
const LARGER_TYPE: f32 = 1.18;

/// A line repeated on at least this many pages is a running head or footer.
const FURNITURE_PAGES: usize = 3;

/// The most candidates offered to the model.
const MAX_CANDIDATES: usize = 160;

/// Unnumbered headings common enough to trust on sight.
const KNOWN_HEADINGS: &[&str] = &[
    "abstract",
    "introduction",
    "background",
    "relatedwork",
    "priorwork",
    "literaturereview",
    "preliminaries",
    "model",
    "themodel",
    "theory",
    "method",
    "methods",
    "methodology",
    "data",
    "dataandmethods",
    "empiricalstrategy",
    "identification",
    "experiments",
    "experimentalsetup",
    "results",
    "evaluation",
    "analysis",
    "discussion",
    "conclusion",
    "conclusions",
    "concludingremarks",
    "summary",
    "limitations",
    "futurework",
    "acknowledgments",
    "acknowledgements",
    "acknowledgment",
    "acknowledgement",
    "references",
    "bibliography",
    "literaturecited",
    "workscited",
    "appendix",
    "appendices",
    "supplementarymaterial",
    "notes",
];

const REFERENCE_HEADINGS: &[&str] = &[
    "references",
    "reference",
    "bibliography",
    "literaturecited",
    "workscited",
    "referencescited",
    "citedliterature",
    "referencesandnotes",
];

const BACK_MATTER_HEADINGS: &[&str] = &[
    "acknowledgments",
    "acknowledgements",
    "acknowledgment",
    "acknowledgement",
    "funding",
    "authorcontributions",
    "competinginterests",
    "declarationofcompetinginterest",
    "conflictsofinterest",
    "dataavailability",
    "ethicsstatement",
];

/// Caption and label lines that look like headings but are not.
const LABEL_PREFIXES: &[&str] = &[
    "figure",
    "fig",
    "table",
    "algorithm",
    "theorem",
    "lemma",
    "proof",
    "definition",
    "proposition",
    "corollary",
    "assumption",
    "remark",
    "example",
    "equation",
    "note",
    "source",
    "panel",
];

/// The roles an outline entry may have; anything else is read as `main`.
pub const ROLES: &[&str] = &[
    "front_matter",
    "main",
    "back_matter",
    "references",
    "appendix",
];

/// A line the text or layout suggests is a printed heading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingCandidate {
    pub page: u32,
    /// The line as printed, number included.
    pub text: String,
    pub number: Option<String>,
    /// The level the numbering implies, when it implies one.
    pub level: Option<u8>,
    /// Set in larger type than the body, according to the layout.
    pub larger: bool,
    /// Numbered or a well-known heading word, rather than only short,
    /// capitalised or large.
    pub strong: bool,
}

/// One admitted outline entry and where its printed line is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub number: Option<String>,
    pub title: String,
    pub level: u8,
    pub page: u32,
    pub role: String,
    /// The heading line as printed, number included.
    pub anchor: String,
    /// The index of the heading's first line in the page text's lines.
    pub line: usize,
}

impl Entry {
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.role == "main"
    }

    /// References and appendices, which are grouped as at most one unit each.
    #[must_use]
    pub fn is_special(&self) -> bool {
        matches!(self.role.as_str(), "references" | "appendix")
    }

    fn to_value(&self) -> Value {
        serde_json::json!({
            "number": self.number,
            "title": self.title,
            "level": self.level,
            "page": self.page,
            "role": self.role,
            "anchor": self.anchor,
        })
    }
}

/// What admitting a model outline kept and dropped.
#[derive(Clone, Debug, Default)]
pub struct Admitted {
    pub entries: Vec<Entry>,
    /// Titles that could not be found on their page or a neighbour.
    pub dropped: Vec<String>,
    /// Entries that named a heading line another entry already took.
    pub duplicates: usize,
}

fn canonical(text: &str) -> String {
    canonical_chars(text).into_iter().collect()
}

fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `R E F E R E N C E S` as `REFERENCES`; other lines unchanged.
fn despaced(line: &str) -> String {
    let words = line.split_whitespace().collect::<Vec<_>>();
    if words.len() >= 4 && words.iter().all(|word| word.chars().count() == 1) {
        words.concat()
    } else {
        compact(line)
    }
}

/// Whether a token is a printed section number: `2`, `2.`, `2.1`, `IV.`,
/// `A.`, `A.1`. A bare roman numeral or capital letter needs its period, so
/// the pronoun "I" and an initial are not numbers.
#[must_use]
pub fn is_section_number(token: &str) -> bool {
    let dotted = token.ends_with('.') || token.ends_with(')');
    let core = token.trim_end_matches(['.', ')']);
    if core.is_empty() || core.starts_with('.') {
        return false;
    }
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.len() > 4 || parts.iter().any(|part| part.is_empty()) {
        return false;
    }
    let numeric = |part: &str| part.len() <= 2 && part.chars().all(|c| c.is_ascii_digit());
    if parts.iter().all(|part| numeric(part)) {
        // A year or a page count is not a section number.
        return parts[0].parse::<u32>().is_ok_and(|first| first <= 40);
    }
    let letter = |part: &str| part.len() == 1 && part.chars().all(|c| c.is_ascii_uppercase());
    if letter(parts[0]) && parts.len() > 1 && parts[1..].iter().all(|part| numeric(part)) {
        return true;
    }
    if parts.len() == 1 && dotted {
        let roman = core.len() <= 6 && core.chars().all(|c| "IVXL".contains(c));
        return roman || letter(core);
    }
    false
}

/// The level a printed number implies: `2.1` is 2, `IV.` is 1, `A.1` is 2.
/// A bare letter implies nothing: it is an appendix or a subsection.
fn number_level(number: &str) -> Option<u8> {
    let core = number.trim_end_matches(['.', ')']);
    if core.chars().all(|c| "IVXL".contains(c)) {
        return Some(1);
    }
    let depth = core.split('.').count();
    if depth == 1 && core.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    u8::try_from(depth.min(4)).ok()
}

/// A heading line split into its printed number and the rest.
fn split_number(line: &str) -> (Option<&str>, &str) {
    let line = line.trim();
    let Some((first, rest)) = line.split_once(char::is_whitespace) else {
        return if is_section_number(line) {
            (Some(line), "")
        } else {
            (None, line)
        };
    };
    if is_section_number(first) {
        (Some(first), rest.trim())
    } else {
        (None, line)
    }
}

/// Whether text reads like heading words: starts with a capital, mostly
/// letters, short, and not a sentence.
fn heading_words(text: &str) -> bool {
    let words = text.split_whitespace().count();
    let Some(first) = text.chars().find(|c| c.is_alphanumeric()) else {
        return false;
    };
    let visible = text.chars().filter(|c| !c.is_whitespace()).count();
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let sentence = text.ends_with('.') && words > 6;
    (1..=MAX_HEADING_WORDS).contains(&words)
        && first.is_uppercase()
        && letters * 10 >= visible * 6
        && !sentence
        && !text.ends_with([',', ';'])
}

fn is_label(text: &str) -> bool {
    let first = text
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_lowercase();
    LABEL_PREFIXES.contains(&first.as_str())
}

fn all_caps(text: &str) -> bool {
    let letters = text
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect::<Vec<_>>();
    letters.len() >= 4
        && letters.iter().all(|c| c.is_uppercase())
        && text.split_whitespace().count() <= 8
        && !super::title::looks_like_identifier(text)
}

/// A line reduced to the key running heads are compared by: canonical
/// characters without digits, so "Page 3" and "Page 4" agree.
#[must_use]
pub fn furniture_key(line: &str) -> String {
    canonical(line)
        .chars()
        .filter(|c| !c.is_ascii_digit())
        .collect()
}

/// Lines repeated on several pages, with digits removed: running heads,
/// footers, journal banners.
#[must_use]
pub fn furniture(pages: &[ExtractedPage]) -> BTreeSet<String> {
    let mut seen = BTreeMap::<String, BTreeSet<u32>>::new();
    for page in pages {
        for line in page.text.lines() {
            let key = furniture_key(line);
            if key.len() >= 4 {
                seen.entry(key).or_default().insert(page.number);
            }
        }
    }
    let threshold = FURNITURE_PAGES.max(pages.len() / 4);
    seen.into_iter()
        .filter(|(_, on)| on.len() >= threshold)
        .map(|(key, _)| key)
        .collect()
}

fn is_furniture(line: &str, furniture: &BTreeSet<String>) -> bool {
    furniture.contains(&furniture_key(line))
}

/// For each page, the canonical text of each layout line set in larger type
/// than the body, mapped to how much larger.
fn larger_lines(layout: &DocumentLayout) -> BTreeMap<u32, BTreeMap<String, f32>> {
    let mut lines = Vec::<(u32, String, f32, usize)>::new();
    for page in &layout.pages {
        let mut grouped = BTreeMap::<u32, Vec<(&str, f32)>>::new();
        for token in &page.tokens {
            if let Some(rect) = token.rects.first() {
                let height = rect.y_max - rect.y_min;
                let width = rect.x_max - rect.x_min;
                // Rotated text is margin furniture.
                if height > 0.0 && width >= height * 0.5 {
                    grouped
                        .entry(token.line)
                        .or_default()
                        .push((token.text.as_str(), height));
                }
            }
        }
        for words in grouped.into_values() {
            let text = words
                .iter()
                .map(|(text, _)| *text)
                .collect::<Vec<_>>()
                .join(" ");
            let mut heights = words.iter().map(|(_, height)| *height).collect::<Vec<_>>();
            heights.sort_by(f32::total_cmp);
            let median = heights[heights.len() / 2];
            lines.push((page.number, text, median, words.len()));
        }
    }
    // The body size: the median line height, weighted by words.
    let mut weighted = lines
        .iter()
        .flat_map(|(_, _, height, count)| std::iter::repeat_n(*height, *count))
        .collect::<Vec<_>>();
    if weighted.is_empty() {
        return BTreeMap::new();
    }
    weighted.sort_by(f32::total_cmp);
    let body = weighted[weighted.len() / 2].max(f32::EPSILON);
    let mut larger = BTreeMap::<u32, BTreeMap<String, f32>>::new();
    for (page, text, height, _) in lines {
        let ratio = height / body;
        if ratio >= LARGER_TYPE {
            larger
                .entry(page)
                .or_default()
                .insert(canonical(&text), ratio);
        }
    }
    larger
}

/// Heading candidates in reading order, from the page text and, when there
/// is one, the layout. Noisy on purpose: the model rejects what is not a
/// heading, and admission rejects what is not printed.
#[must_use]
pub fn candidates(pages: &[ExtractedPage], layout: &DocumentLayout) -> Vec<HeadingCandidate> {
    let furniture = furniture(pages);
    let larger = larger_lines(layout);
    let mut found = Vec::new();
    for page in pages {
        let sizes = larger.get(&page.number);
        let lines = page.text.lines().map(despaced).collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if line.is_empty()
                || line.chars().count() > MAX_HEADING_CHARS
                || is_furniture(line, &furniture)
            {
                continue;
            }
            let is_larger = sizes.is_some_and(|sizes| sizes.contains_key(&canonical(line)));
            let (number, rest) = split_number(line);
            let candidate = match number {
                Some(number) if rest.is_empty() => {
                    // A number printed on its own line before the heading text.
                    // A page number followed by a running head is not one.
                    lines[index + 1..]
                        .iter()
                        .find(|next| !next.is_empty())
                        .filter(|next| {
                            heading_words(next)
                                && !next.ends_with('.')
                                && !is_furniture(next, &furniture)
                        })
                        .map(|next| (format!("{number} {next}"), Some(number), true))
                }
                Some(number) if heading_words(rest) => Some((line.clone(), Some(number), true)),
                Some(_) => None,
                None => {
                    let key = canonical(line.trim_end_matches([':', '.']));
                    let known =
                        KNOWN_HEADINGS.contains(&key.as_str()) || key.starts_with("appendix");
                    let shaped = !is_label(line)
                        && heading_words(line)
                        && !line.ends_with('.')
                        && (all_caps(line) || is_larger);
                    (known || shaped).then(|| (line.clone(), None, known))
                }
            };
            if let Some((text, number, strong)) = candidate {
                found.push(HeadingCandidate {
                    page: page.number,
                    text,
                    number: number.map(str::to_owned),
                    level: number.and_then(number_level),
                    larger: is_larger,
                    strong,
                });
            }
        }
    }
    found.truncate(MAX_CANDIDATES);
    found
}

/// The candidates as the outline call reads them.
#[must_use]
pub fn render_candidates(candidates: &[HeadingCandidate]) -> String {
    use std::fmt::Write as _;
    if candidates.is_empty() {
        return "Heading candidates from the text layer: none were found. That can mean the paper prints no headings, or that its headings are styled in a way the heuristics miss; read the pages to decide.".to_owned();
    }
    let mut text = String::from(
        "Heading candidates from the text layer, found by deterministic heuristics (numbering, familiar heading words, capitals, larger type). They are a starting point, not an answer: some are captions, table rows, running heads or the title, and real headings may be missing.\n",
    );
    for candidate in candidates {
        let mut signals = Vec::new();
        if candidate.number.is_some() {
            signals.push("numbered");
        }
        if candidate.larger {
            signals.push("larger type");
        }
        if candidate.number.is_none() && candidate.strong {
            signals.push("familiar heading");
        }
        let signals = if signals.is_empty() {
            String::new()
        } else {
            format!(" [{}]", signals.join(", "))
        };
        // Writing to a String cannot fail.
        let _ = writeln!(
            text,
            "- page {}: \"{}\"{signals}",
            candidate.page, candidate.text
        );
    }
    text
}

/// Two lines joined as printed: a line-break hyphen closes up.
fn join_lines(first: &str, second: &str) -> String {
    let first = first.trim_end();
    if let Some(stem) = first.strip_suffix('-')
        && second.starts_with(|c: char| c.is_lowercase())
    {
        format!("{stem}{}", second.trim())
    } else {
        format!("{first} {}", second.trim())
    }
}

/// Where a heading's text is printed on a page.
struct Found {
    line: usize,
    number: Option<String>,
    /// The heading text as printed, without the number.
    printed: String,
}

/// Whether `rest`, the printed line without its number, carries `title`:
/// the whole line, a line that is mostly the title, or a run-in heading
/// such as "Data. We collect ...".
fn carries(rest: &str, title: &str) -> Option<String> {
    let wanted = canonical(title);
    let have = canonical(rest);
    if wanted.chars().count() < 2 || !have.starts_with(&wanted) {
        return None;
    }
    let clean = |text: &str| compact(text.trim_end_matches([':', '.', ' ']));
    if have == wanted || wanted.len() * 10 >= have.len() * 6 {
        return Some(clean(rest));
    }
    // A run-in heading ends at its first period, colon or dash.
    let head = [". ", ": ", "\u{2014}", " - "]
        .iter()
        .filter_map(|separator| rest.find(separator))
        .min()
        .map(|end| &rest[..end])?;
    (canonical(head) == wanted).then(|| clean(head))
}

/// The first line on `text` at or after `after` that prints `title`, with
/// its number. A heading may wrap onto a second line, and its number may sit
/// on the line above.
fn find_heading(text: &str, title: &str, after: Option<usize>) -> Option<Found> {
    let lines = text.lines().map(despaced).collect::<Vec<_>>();
    let mut first_match = None;
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() || line.chars().count() > MAX_HEADING_CHARS * 2 {
            continue;
        }
        let (mut number, rest) = split_number(line);
        let previous = lines[..index].iter().rev().find(|line| !line.is_empty());
        if number.is_none()
            && let Some(previous) = previous
            && is_section_number(previous)
        {
            number = Some(previous.as_str());
        }
        let next = lines[index + 1..].iter().find(|line| !line.is_empty());
        let printed = carries(rest, title).or_else(|| {
            let next = next?;
            let joined = join_lines(rest, next);
            canonical(title)
                .starts_with(&canonical(rest))
                .then(|| carries(&joined, title))
                .flatten()
        });
        let Some(printed) = printed else {
            continue;
        };
        let found = Found {
            line: index,
            number: number.map(|number| number.trim_end_matches(')').to_owned()),
            printed,
        };
        if after.is_none_or(|after| index > after) {
            return Some(found);
        }
        first_match.get_or_insert(found);
    }
    first_match
}

/// The role a heading's words decide, whatever the model said.
fn decided_role(title: &str) -> Option<&'static str> {
    let key = canonical(title);
    if REFERENCE_HEADINGS.contains(&key.as_str()) {
        Some("references")
    } else if key.starts_with("appendix")
        || key.starts_with("onlineappendix")
        || key.starts_with("supplementary")
        || key == "appendices"
    {
        Some("appendix")
    } else if BACK_MATTER_HEADINGS.contains(&key.as_str()) {
        Some("back_matter")
    } else if key == "abstract" {
        Some("front_matter")
    } else {
        None
    }
}

/// Admit a model outline against the page text, in place.
///
/// Each entry's title must be printed on its page, or on the page before or
/// after (its page is then corrected). The printed number and title replace
/// the model's, numbered levels follow the numbering, and entries come back
/// in reading order without duplicates. Anything else is dropped. Running
/// this twice changes nothing.
pub fn admit(outline: &mut Value, pages: &[ExtractedPage]) -> Admitted {
    let Some(items) = outline.as_array() else {
        *outline = Value::Array(Vec::new());
        return Admitted::default();
    };
    let by_number = pages
        .iter()
        .map(|page| (page.number, page))
        .collect::<BTreeMap<_, _>>();
    let mut admitted = Admitted::default();
    let mut previous: Option<(u32, usize)> = None;
    for item in items {
        let raw_title = item
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        // A number the model left inside the title is read from the line.
        let (_, title) = split_number(raw_title);
        let cited = item
            .get("page")
            .and_then(Value::as_u64)
            .and_then(|page| u32::try_from(page).ok())
            .unwrap_or(1);
        let order = [Some(cited), cited.checked_add(1), cited.checked_sub(1)];
        let found = order.into_iter().flatten().find_map(|number| {
            let page = by_number.get(&number)?;
            let after = previous
                .filter(|(page, _)| *page == number)
                .map(|(_, line)| line);
            find_heading(&page.text, title, after).map(|found| (number, found))
        });
        let Some((page, found)) = found else {
            admitted.dropped.push(compact(raw_title));
            continue;
        };
        // The printed number wins over the model's.
        let number = found.number;
        let model_level = item
            .get("level")
            .and_then(Value::as_u64)
            .map_or(1, |level| u8::try_from(level.clamp(1, 4)).unwrap_or(1));
        let level = number
            .as_deref()
            .and_then(number_level)
            .unwrap_or(model_level);
        let model_role = item
            .get("role")
            .and_then(Value::as_str)
            .filter(|role| ROLES.contains(role))
            .unwrap_or("main");
        let role = decided_role(&found.printed)
            .unwrap_or(model_role)
            .to_owned();
        let anchor = number.as_ref().map_or_else(
            || found.printed.clone(),
            |number| format!("{number} {}", found.printed),
        );
        previous = Some((page, found.line));
        admitted.entries.push(Entry {
            number,
            title: found.printed,
            level,
            page,
            role,
            anchor,
            line: found.line,
        });
    }
    admitted
        .entries
        .sort_by_key(|entry| (entry.page, entry.line));
    let before = admitted.entries.len();
    admitted
        .entries
        .dedup_by_key(|entry| (entry.page, entry.line));
    admitted.duplicates = before - admitted.entries.len();
    let mut parent = 0;
    for entry in &mut admitted.entries {
        entry.level = entry.level.min(parent + 1).max(1);
        parent = entry.level;
    }
    *outline = Value::Array(admitted.entries.iter().map(Entry::to_value).collect());
    admitted
}

/// The admitted entries of an outline already in a draft, with where each
/// is printed.
#[must_use]
pub fn entries(outline: &Value, pages: &[ExtractedPage]) -> Vec<Entry> {
    let mut copy = outline.clone();
    admit(&mut copy, pages).entries
}

/// The main-text pages: up to the first references or appendix heading.
#[must_use]
pub fn body_pages(entries: &[Entry], page_count: usize) -> usize {
    entries
        .iter()
        .find(|entry| entry.is_special())
        .and_then(|entry| usize::try_from(entry.page).ok())
        .unwrap_or(page_count)
        .clamp(1, page_count.max(1))
}

/// Whether an outline is rich enough to group logical units from: at least
/// two main-text headings, and about one per six pages of main text, so the
/// ranges can reach the unit budget.
#[must_use]
pub fn usable(entries: &[Entry], page_count: usize) -> bool {
    let main = entries.iter().filter(|entry| entry.is_main()).count();
    main >= 2 && main >= body_pages(entries, page_count).div_ceil(6)
}

/// The outline as the logical-units call reads it.
#[must_use]
pub fn render_outline(entries: &[Entry]) -> String {
    use std::fmt::Write as _;
    let mut text = String::from(
        "The paper's printed outline, already checked against the page text. `outline_first` and `outline_last` refer to the indices in brackets.\n",
    );
    for (index, entry) in entries.iter().enumerate() {
        let indent = "  ".repeat(usize::from(entry.level.saturating_sub(1)));
        let role = if entry.is_main() {
            String::new()
        } else {
            format!(" ({})", entry.role.replace('_', " "))
        };
        // Writing to a String cannot fail.
        let _ = writeln!(
            text,
            "[{index}] page {}{role}: {indent}{}",
            entry.page, entry.anchor
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pages(texts: &[&str]) -> Vec<ExtractedPage> {
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| ExtractedPage {
                number: u32::try_from(index + 1).expect("page"),
                text: (*text).to_owned(),
            })
            .collect()
    }

    #[test]
    fn should_recognize_printed_section_numbers_but_not_words_or_years() {
        for number in ["2", "2.", "2.1", "3.4.1", "IV.", "A.", "A.1", "B.2.3", "1)"] {
            assert!(is_section_number(number), "{number}");
        }
        for word in ["I", "A", "2019", "1990.", "Table", "2.1.3.4.5", "x.1", ".5"] {
            assert!(!is_section_number(word), "{word}");
        }
        assert_eq!(number_level("2.1"), Some(2));
        assert_eq!(number_level("IV."), Some(1));
        assert_eq!(number_level("A."), None);
        assert_eq!(number_level("A.1"), Some(2));
    }

    #[test]
    fn should_find_numbered_and_familiar_headings_but_not_prose_or_captions() {
        let pages = pages(&[
            "Sparse Models\nAbstract\nWe study pruning.\n1 Introduction\nPruning is old.",
            "2 Related Work\n2.1 Magnitude pruning\nTable 1: Accuracy\n3 the model is described below in full.\n10 Epochs of training were run on eight GPUs for the main experiment.",
            "REFERENCES\n[1] A. Author. A paper. 2019.",
        ]);
        let found = candidates(&pages, &DocumentLayout::default());
        let texts = found
            .iter()
            .map(|candidate| candidate.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            [
                "Abstract",
                "1 Introduction",
                "2 Related Work",
                "2.1 Magnitude pruning",
                "REFERENCES"
            ]
        );
        assert_eq!(found[3].level, Some(2));
        assert!(found.iter().all(|candidate| candidate.strong));
    }

    #[test]
    fn should_offer_a_line_set_in_larger_type_as_a_candidate() {
        use crate::domain::{LayoutPage, LayoutToken, TextRect};
        let pages = pages(&[
            "How Markets Clear\nPrices move until supply meets demand in the end.\nShort Note\nMore text.",
        ]);
        let line = |line: u32, words: &[&str], top: f32, height: f32| {
            words
                .iter()
                .enumerate()
                .map(|(index, word)| {
                    let left = 60.0f32.mul_add(f32::from(u8::try_from(index).expect("index")), 72.0);
                    LayoutToken {
                        index: line * 100 + u32::try_from(index).expect("index"),
                        text: (*word).to_owned(),
                        line,
                        rects: vec![TextRect {
                            x_min: left,
                            y_min: top,
                            x_max: left + 50.0,
                            y_max: top + height,
                        }],
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut tokens = line(0, &["How", "Markets", "Clear"], 100.0, 14.0);
        tokens.extend(line(
            1,
            &["Prices", "move", "until", "supply", "meets", "demand", "in", "the", "end."],
            130.0,
            10.0,
        ));
        tokens.extend(line(2, &["Short", "Note"], 150.0, 10.0));
        tokens.extend(line(3, &["More", "text."], 170.0, 10.0));
        let layout = DocumentLayout {
            schema_version: 1,
            pages: vec![LayoutPage {
                number: 1,
                width: 612.0,
                height: 792.0,
                tokens,
                sentences: Vec::new(),
            }],
        };
        let found = candidates(&pages, &layout);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].text, "How Markets Clear");
        assert!(found[0].larger && !found[0].strong);
        assert!(render_candidates(&found).contains("[larger type]"));
    }

    #[test]
    fn should_skip_running_heads_repeated_across_pages() {
        let pages = pages(&[
            "JOURNAL OF THINGS\n1 Introduction",
            "JOURNAL OF THINGS\nText.",
            "JOURNAL OF THINGS\n2 Data",
            "JOURNAL OF THINGS\nText.",
        ]);
        let found = candidates(&pages, &DocumentLayout::default());
        assert!(
            found
                .iter()
                .all(|candidate| candidate.text != "JOURNAL OF THINGS")
        );
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn should_not_join_a_page_number_with_the_running_head_after_it() {
        let pages = pages(&[
            "1 Introduction\nWe study markets.",
            "2\nAMERICAN ECONOMIC REVIEW\nText goes on here.",
            "3\nAMERICAN ECONOMIC REVIEW\nMore text here.",
            "4\nAMERICAN ECONOMIC REVIEW\n2 Data\nWe use data.",
            "5\nAMERICAN ECONOMIC REVIEW\nEven more text.",
        ]);
        let numbered = candidates(&pages, &DocumentLayout::default())
            .into_iter()
            .filter(|candidate| candidate.number.is_some())
            .map(|candidate| candidate.text)
            .collect::<Vec<_>>();
        assert_eq!(numbered, ["1 Introduction", "2 Data"]);
    }

    #[test]
    fn should_count_entries_that_name_a_heading_already_taken() {
        let pages = pages(&["1 Introduction\nText.", "2 Data\nText."]);
        let mut outline = json!([
            {"number": "1", "title": "Introduction", "level": 1, "page": 1, "role": "main"},
            {"number": "1", "title": "Introduction", "level": 1, "page": 1, "role": "main"},
            {"number": "2", "title": "Data", "level": 1, "page": 2, "role": "main"}
        ]);
        let admitted = admit(&mut outline, &pages);
        assert_eq!(admitted.entries.len(), 2);
        assert_eq!(admitted.duplicates, 1);
    }

    #[test]
    fn should_admit_printed_headings_and_drop_invented_ones() {
        let pages = pages(&[
            "Title\n1 Introduction\nText.",
            "2 Re-\nlated Work\nText.\n2.1 Data. We collect many things here.",
            "References\n[1] Someone.",
        ]);
        let mut outline = json!([
            {"number": "2", "title": "Related Work", "level": 1, "page": 2, "role": "main"},
            {"number": "1", "title": "Introduction", "level": 1, "page": 1, "role": "main"},
            {"number": null, "title": "Data", "level": 3, "page": 2, "role": "main"},
            {"number": "3", "title": "Results", "level": 1, "page": 2, "role": "main"},
            {"number": null, "title": "References", "level": 1, "page": 2, "role": "main"}
        ]);
        let admitted = admit(&mut outline, &pages);
        assert_eq!(admitted.dropped, ["Results"]);
        let shown = admitted
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.anchor.as_str(),
                    entry.level,
                    entry.page,
                    entry.role.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            [
                ("1 Introduction", 1, 1, "main"),
                ("2 Related Work", 1, 2, "main"),
                ("2.1 Data", 2, 2, "main"),
                ("References", 1, 3, "references"),
            ]
        );
        let mut run_in = json!([{"number": null, "title": "Abstract", "level": 1, "page": 1, "role": "front_matter"}]);
        let ieee = self::pages(&["Abstract\u{2014}We study pruning in deep networks."]);
        assert_eq!(admit(&mut run_in, &ieee).entries.len(), 1);
        // Admission is idempotent.
        let again = outline.clone();
        admit(&mut outline, &pages);
        assert_eq!(outline, again);
        assert_eq!(outline[2]["number"], "2.1");
        assert_eq!(outline[2]["title"], "Data");
    }

    #[test]
    fn should_admit_an_empty_outline_for_a_paper_without_headings() {
        let pages = pages(&["A short letter.\nIt has no headings at all."]);
        let mut outline = json!([]);
        let admitted = admit(&mut outline, &pages);
        assert!(admitted.entries.is_empty() && admitted.dropped.is_empty());
        assert!(!usable(&admitted.entries, 1));
        let mut missing = Value::Null;
        admit(&mut missing, &pages);
        assert_eq!(missing, json!([]));
    }

    #[test]
    fn should_need_about_one_main_heading_per_six_pages_to_be_usable() {
        let entry = |page, role: &str| Entry {
            number: None,
            title: "T".to_owned(),
            level: 1,
            page,
            role: role.to_owned(),
            anchor: "T".to_owned(),
            line: 0,
        };
        let two = vec![entry(1, "main"), entry(5, "main")];
        assert!(usable(&two, 8));
        assert!(!usable(&two, 30));
        let with_references = vec![entry(1, "main"), entry(3, "main"), entry(9, "references")];
        assert!(usable(&with_references, 30));
        assert_eq!(body_pages(&with_references, 30), 9);
    }
}
