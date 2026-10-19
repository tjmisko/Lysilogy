//! The chunked object pass: a few pages at a time, only the objects printed
//! on them.
//!
//! A whole-paper answer runs out of room on equation-dense papers, so the
//! object inventory is read in page chunks and merged. Every page belongs to
//! exactly one chunk, and an object is kept only when the page it names is in
//! the chunk that returned it, so one printed label cannot arrive twice from
//! two chunks. Ranges such as `24-28` are rejected, never expanded.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

/// The most checklist equations one chunk carries; 77 dense equations in one
/// chunk overran a 32k-token answer.
pub const CHUNK_EQUATION_LOAD: usize = 40;

/// Consecutive page groups, each page exactly once.
///
/// A page joins the current chunk unless that would pass `max_pages` pages or
/// `max_load` checklist equations; a page heavier than `max_load` on its own
/// gets its own chunk.
#[must_use]
pub fn page_chunks(pages: &[(u32, usize)], max_pages: usize, max_load: usize) -> Vec<Vec<u32>> {
    let mut chunks = Vec::<Vec<u32>>::new();
    let mut load = 0;
    for &(page, page_load) in pages {
        let full = chunks
            .last()
            .is_none_or(|chunk| chunk.len() >= max_pages.max(1) || load + page_load > max_load);
        if full {
            chunks.push(Vec::new());
            load = 0;
        }
        if let Some(chunk) = chunks.last_mut() {
            chunk.push(page);
        }
        load += page_load;
    }
    chunks
}

/// Equation numbers the text layer shows at the end of a line, in order,
/// without repeats: `(12)`, `(2.7)`, `(A.3)`, `(B1)`, `(4b)`.
///
/// A checklist for the model, not truth: a reference to an equation that ends
/// a line also matches, and years are skipped.
#[must_use]
pub fn equation_candidates(page_text: &str) -> Vec<String> {
    let mut found = Vec::<String>::new();
    for line in page_text.lines() {
        let line = line.trim_end();
        let Some(body) = line.strip_suffix(')') else {
            continue;
        };
        let Some(open) = body.rfind('(') else {
            continue;
        };
        // `f(x)` is not a label: the number must stand apart from the text.
        if open > 0 && !body[..open].ends_with(char::is_whitespace) {
            continue;
        }
        let label = &body[open + 1..];
        if is_equation_label(label) && !found.iter().any(|seen| seen == label) {
            found.push(label.to_owned());
        }
    }
    found
}

fn is_equation_label(label: &str) -> bool {
    let rest = label
        .strip_prefix(|c: char| c.is_ascii_uppercase())
        .map_or(label, |rest| rest.strip_prefix('.').unwrap_or(rest));
    let rest = rest
        .strip_suffix(|c: char| c.is_ascii_lowercase())
        .unwrap_or(rest);
    let mut parts = rest.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    let digits = |part: &str| {
        !part.is_empty() && part.len() <= 3 && part.bytes().all(|b| b.is_ascii_digit())
    };
    // Four-digit numbers are years, not equation numbers.
    digits(first) && parts.all(digits)
}

/// Whether a chunk answer is worth one retry: the call failed, or it listed
/// fewer than half the equation numbers its checklist found (when the
/// checklist found at least `RETRY_MINIMUM_CANDIDATES`).
#[must_use]
pub fn needs_retry(objects: Option<&[Value]>, candidates: usize) -> bool {
    let Some(objects) = objects else {
        return true;
    };
    candidates >= RETRY_MINIMUM_CANDIDATES && numbered_equations(objects) * 2 < candidates
}

/// The better of two chunk answers: the one with more numbered objects; the
/// first on a tie.
#[must_use]
pub fn better(first: Option<Vec<Value>>, second: Option<Vec<Value>>) -> Option<Vec<Value>> {
    match (first, second) {
        (Some(first), Some(second)) => {
            if numbered(&second) > numbered(&first) {
                Some(second)
            } else {
                Some(first)
            }
        }
        (first, second) => first.or(second),
    }
}

/// Fewer checklist candidates than this never trigger a retry on their own.
pub const RETRY_MINIMUM_CANDIDATES: usize = 4;

fn numbered(objects: &[Value]) -> usize {
    objects
        .iter()
        .filter(|object| !normalized_number(object).is_empty())
        .count()
}

fn numbered_equations(objects: &[Value]) -> usize {
    objects
        .iter()
        .filter(|object| {
            text(object, "kind") == "equation" && !normalized_number(object).is_empty()
        })
        .count()
}

/// What merging kept and discarded, for the run record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MergeStats {
    pub chunks: usize,
    pub failed_chunks: usize,
    pub kept: usize,
    /// The object named a page outside the chunk that returned it.
    pub out_of_range: usize,
    /// The label or number was a range or list, such as `24-28`.
    pub ranges_rejected: usize,
    /// The same kind, number and page twice.
    pub duplicates: usize,
    /// Not an object with a kind and a page.
    pub malformed: usize,
}

/// One chunk's pages and its objects, or `None` when the call failed.
pub struct ChunkObjects {
    pub pages: Vec<u32>,
    pub objects: Option<Vec<Value>>,
}

/// Merge chunk results into one inventory in page order.
#[must_use]
pub fn merge(chunks: Vec<ChunkObjects>) -> (Vec<Value>, MergeStats) {
    let mut stats = MergeStats {
        chunks: chunks.len(),
        ..MergeStats::default()
    };
    let mut seen = HashSet::new();
    let mut kept = Vec::<(u32, Value)>::new();
    for chunk in chunks {
        let Some(objects) = chunk.objects else {
            stats.failed_chunks += 1;
            continue;
        };
        for object in objects {
            let page = object
                .get("page")
                .and_then(Value::as_u64)
                .and_then(|page| u32::try_from(page).ok());
            let kind = object
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let (Some(page), Some(kind)) = (page, kind) else {
                stats.malformed += 1;
                continue;
            };
            if !chunk.pages.contains(&page) {
                stats.out_of_range += 1;
                continue;
            }
            if is_range(&object) {
                stats.ranges_rejected += 1;
                continue;
            }
            let number = normalized_number(&object);
            if !number.is_empty() && !seen.insert((kind, number, page)) {
                stats.duplicates += 1;
                continue;
            }
            kept.push((page, object));
        }
    }
    // Stable: objects on one page keep the order the model gave.
    kept.sort_by_key(|(page, _)| *page);
    stats.kept = kept.len();
    (kept.into_iter().map(|(_, object)| object).collect(), stats)
}

fn text<'v>(object: &'v Value, key: &str) -> &'v str {
    object.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A printed label is one number; `24-28`, `(31)–(34)` or `5, 6` are several.
fn is_range(object: &Value) -> bool {
    [text(object, "number"), text(object, "label")]
        .into_iter()
        .any(|value| {
            value
                .char_indices()
                .filter(|(_, character)| matches!(character, '-' | '–' | '—' | ','))
                .any(|(index, character)| {
                    let before = value[..index].trim_end_matches([' ', ')']);
                    let after =
                        value[index + character.len_utf8()..].trim_start_matches([' ', '(']);
                    before.ends_with(|c: char| c.is_ascii_digit())
                        && after.starts_with(|c: char| c.is_ascii_alphanumeric())
                })
        })
}

fn normalized_number(object: &Value) -> String {
    let number = text(object, "number");
    let raw = if number.is_empty() {
        text(object, "label")
    } else {
        number
    };
    raw.chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '(' | ')'))
        .collect::<String>()
        .trim_end_matches('.')
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(kind: &str, number: &str, page: u32) -> Value {
        json!({"kind": kind, "label": format!("({number})"), "number": number, "page": page})
    }

    fn chunk(pages: &[u32], objects: Vec<Value>) -> ChunkObjects {
        ChunkObjects {
            pages: pages.to_vec(),
            objects: Some(objects),
        }
    }

    #[test]
    fn should_find_equation_numbers_that_end_a_line() {
        let text = "V(φ) = 1/2 Tr[φ, φ†].   (1.1)\nin the vacuum\nφ = a σ3, (1.3)\nx = y (A.3)\n z (B1)\nw (4b)\n";
        assert_eq!(equation_candidates(text), ["1.1", "1.3", "A.3", "B1", "4b"]);
    }

    #[test]
    fn should_skip_years_function_calls_and_repeats() {
        let text =
            "Phys. Rev. Lett. 117 (2016)\nwhere f(x)\nsee (12)\nas in (12)\n(Smith)\n(iii)\n()\n";
        assert_eq!(equation_candidates(text), ["12"]);
    }

    #[test]
    fn should_accept_a_number_alone_on_its_line() {
        assert_eq!(equation_candidates("(7)\n  (8)  \n"), ["7", "8"]);
    }

    #[test]
    fn should_retry_a_failed_chunk_or_one_that_lists_under_half_its_checklist() {
        let two = vec![object("equation", "1", 1), object("equation", "2", 1)];
        assert!(needs_retry(None, 0));
        assert!(needs_retry(Some(&two), 5));
        assert!(!needs_retry(Some(&two), 4));
        assert!(
            !needs_retry(Some(&[]), 3),
            "a short checklist alone never retries"
        );
        let figures = vec![
            object("figure", "1", 1),
            object("figure", "2", 1),
            object("figure", "3", 1),
        ];
        assert!(
            needs_retry(Some(&figures), 4),
            "figures do not count toward equations"
        );
    }

    #[test]
    fn should_keep_the_answer_with_more_numbered_objects() {
        let one = Some(vec![object("equation", "1", 1)]);
        let two = Some(vec![object("equation", "1", 1), object("equation", "2", 1)]);
        assert_eq!(better(one.clone(), two.clone()), two);
        assert_eq!(better(two.clone(), one.clone()), two);
        assert_eq!(better(one.clone(), one.clone()), one);
        assert_eq!(better(None, one.clone()), one);
        assert_eq!(better(one.clone(), None), one);
        assert_eq!(better(None, None), None);
    }

    fn unloaded(pages: std::ops::RangeInclusive<u32>) -> Vec<(u32, usize)> {
        pages.map(|page| (page, 0)).collect()
    }

    #[test]
    fn should_cover_every_page_exactly_once_including_a_partial_last_chunk() {
        let chunks = page_chunks(&unloaded(1..=25), 6, 40);
        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[4], vec![25]);
        assert_eq!(chunks.concat(), (1..=25).collect::<Vec<u32>>());
    }

    #[test]
    fn should_make_one_chunk_when_the_paper_is_shorter_than_a_chunk() {
        assert_eq!(page_chunks(&unloaded(1..=3), 6, 40), vec![vec![1, 2, 3]]);
        assert!(page_chunks(&[], 6, 40).is_empty());
        assert_eq!(page_chunks(&unloaded(1..=2), 0, 40), vec![vec![1], vec![2]]);
    }

    #[test]
    fn should_split_a_chunk_before_it_carries_too_many_equations() {
        // 1909.06187 pages 7-12 carried 77 equations in one chunk.
        let pages = [(7, 14), (8, 9), (9, 13), (10, 10), (11, 13), (12, 18)];
        let chunks = page_chunks(&pages, 6, 40);
        assert_eq!(chunks, vec![vec![7, 8, 9], vec![10, 11], vec![12]]);
        for chunk in &chunks {
            let load: usize = pages
                .iter()
                .filter(|(page, _)| chunk.contains(page))
                .map(|(_, load)| load)
                .sum();
            assert!(load <= 40, "{chunk:?} carries {load}");
        }
    }

    #[test]
    fn should_give_a_page_heavier_than_the_limit_its_own_chunk() {
        let chunks = page_chunks(&[(1, 2), (2, 55), (3, 1)], 6, 40);
        assert_eq!(chunks, vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn should_keep_objects_in_page_order_across_chunks() {
        let (objects, stats) = merge(vec![
            chunk(
                &[1, 2],
                vec![object("equation", "2", 2), object("equation", "1", 1)],
            ),
            chunk(&[3, 4], vec![object("figure", "1", 3)]),
        ]);
        let numbers = objects
            .iter()
            .map(|o| text(o, "number"))
            .collect::<Vec<_>>();
        assert_eq!(numbers, ["1", "2", "1"]);
        assert_eq!(stats.kept, 3);
    }

    #[test]
    fn should_drop_an_object_whose_page_is_outside_its_chunk() {
        // A mention of Figure 7 on page 2 must not become Figure 7 on page 9.
        let (objects, stats) = merge(vec![chunk(&[1, 2], vec![object("figure", "7", 9)])]);
        assert!(objects.is_empty());
        assert_eq!(stats.out_of_range, 1);
    }

    #[test]
    fn should_reject_ranges_and_lists_rather_than_expand_them() {
        let (objects, stats) = merge(vec![chunk(
            &[1],
            vec![
                object("equation", "24-28", 1),
                json!({"kind": "equation", "label": "(31)–(34)", "number": null, "page": 1}),
                object("equation", "5, 6", 1),
                object("equation", "A.3", 1),
                object("equation", "2.7", 1),
                json!({"kind": "table", "label": "Table A-1", "number": "A-1", "page": 1}),
            ],
        )]);
        assert_eq!(stats.ranges_rejected, 3);
        let numbers = objects
            .iter()
            .map(|o| text(o, "number"))
            .collect::<Vec<_>>();
        assert_eq!(numbers, ["A.3", "2.7", "A-1"]);
    }

    #[test]
    fn should_merge_a_label_listed_twice_on_the_same_page() {
        let (objects, stats) = merge(vec![chunk(
            &[4],
            vec![
                object("equation", "12", 4),
                json!({"kind": "equation", "label": "(12)", "number": " 12 ", "page": 4}),
            ],
        )]);
        assert_eq!(objects.len(), 1);
        assert_eq!(stats.duplicates, 1);
    }

    #[test]
    fn should_keep_two_printed_objects_that_share_a_label_on_different_pages() {
        // An appendix that restarts numbering prints "Table 1" twice.
        let (objects, stats) = merge(vec![
            chunk(&[3], vec![object("table", "1", 3)]),
            chunk(&[14], vec![object("table", "1", 14)]),
        ]);
        assert_eq!(objects.len(), 2);
        assert_eq!(stats.duplicates, 0);
    }

    #[test]
    fn should_keep_the_other_chunks_when_one_chunk_fails() {
        let (objects, stats) = merge(vec![
            chunk(&[1], vec![object("equation", "1", 1)]),
            ChunkObjects {
                pages: vec![2],
                objects: None,
            },
            chunk(&[3], vec![object("equation", "3", 3)]),
        ]);
        assert_eq!(objects.len(), 2);
        assert_eq!(stats.failed_chunks, 1);
        assert_eq!(stats.chunks, 3);
    }

    #[test]
    fn should_count_malformed_objects_without_a_kind_or_page() {
        let (objects, stats) = merge(vec![chunk(
            &[1],
            vec![
                json!({"kind": "figure", "number": "1"}),
                json!({"page": 1}),
                json!("text"),
            ],
        )]);
        assert!(objects.is_empty());
        assert_eq!(stats.malformed, 3);
    }

    #[test]
    fn should_keep_unnumbered_objects_without_deduplicating_them() {
        let unnumbered = json!({"kind": "figure", "label": null, "number": null, "page": 1});
        let (objects, _) = merge(vec![chunk(&[1], vec![unnumbered.clone(), unnumbered])]);
        assert_eq!(objects.len(), 2);
    }
}
