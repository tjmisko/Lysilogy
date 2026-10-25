//! Reading lists: validated titles, error codes, limits, and the "foundational first" order.

use std::collections::{HashMap, HashSet};

use crate::WorkId;

/// Most Works on one item's path (seed or root through the item), so depth is at most 8.
pub const RAMP_PATH_MAX: usize = 9;
/// Most Work and claim nodes one list holds.
pub const LIST_WORKS_MAX: usize = 500;
/// Most topic nodes one list holds.
pub const LIST_TOPICS_MAX: usize = 200;
/// Most stored edges one list holds.
pub const LIST_EDGES_MAX: usize = 4_000;
/// Longest list description in characters.
pub const LIST_DESCRIPTION_MAX_CHARS: usize = 4_000;
/// Longest topic label in characters.
pub const LIST_TOPIC_LABEL_MAX_CHARS: usize = 80;
/// Longest list title in characters.
pub const LIST_TITLE_MAX_CHARS: usize = 120;
/// Title used when the seed has none.
pub const LIST_TITLE_FALLBACK: &str = "Reading list";

/// Why list input or a list change was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ListError {
    #[error("list title must not be empty")]
    EmptyTitle,
    #[error("list title must be at most {LIST_TITLE_MAX_CHARS} characters")]
    TitleTooLong,
    #[error("list title must not contain control characters")]
    TitleControlCharacter,
    #[error("description must be at most {LIST_DESCRIPTION_MAX_CHARS} characters")]
    DescriptionTooLong,
    #[error(
        "topic label must be 1 to {LIST_TOPIC_LABEL_MAX_CHARS} characters without control characters"
    )]
    InvalidTopicLabel,
    #[error("the Work is already in the list")]
    ItemExists,
    #[error("the Work it was reached from is not in the list")]
    ViaNotInList,
    #[error("the Work it was reached from does not cite it")]
    NoCitationEdge,
    /// Walk-era limit on `path_work_ids`; kept for the ramp migration.
    #[error("the path would be longer than {RAMP_PATH_MAX} Works")]
    TooDeep,
    #[error("a list holds at most {LIST_WORKS_MAX} Works and claims")]
    Full,
    #[error("a list holds at most {LIST_TOPICS_MAX} topics")]
    TopicsFull,
    #[error("a list holds at most {LIST_EDGES_MAX} edges")]
    EdgesFull,
    /// Ramp-era rule (the seed could not be removed); D14 drops it, kept until callers do.
    #[error("the seed item cannot be removed")]
    SeedItem,
    #[error("the paper is not linked to a Work")]
    PaperNotLinked,
    #[error("the edge would close a cycle")]
    EdgeCycle,
    #[error("the edge kind does not fit its endpoints")]
    InvalidEdgeKind,
    #[error("the order must list every item exactly once")]
    InvalidOrder,
}

impl ListError {
    /// The machine-readable API error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptyTitle | Self::TitleTooLong | Self::TitleControlCharacter => {
                "invalid_list_title"
            }
            Self::DescriptionTooLong => "invalid_description",
            Self::InvalidTopicLabel => "invalid_topic_label",
            Self::ItemExists => "item_exists",
            Self::ViaNotInList => "via_not_in_list",
            Self::NoCitationEdge => "no_citation_edge",
            Self::TooDeep => "ramp_too_deep",
            Self::Full => "list_full",
            Self::TopicsFull => "topics_full",
            Self::EdgesFull => "edges_full",
            Self::SeedItem => "seed_item",
            Self::PaperNotLinked => "paper_not_linked",
            Self::EdgeCycle => "edge_cycle",
            Self::InvalidEdgeKind => "invalid_edge_kind",
            Self::InvalidOrder => "invalid_order",
        }
    }
}

/// A validated list title: trimmed, single-spaced, 1 to 120 characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListTitle(String);

impl ListTitle {
    /// Trim, collapse whitespace runs to one space, and validate.
    pub fn parse(text: &str) -> Result<Self, ListError> {
        if text.chars().any(char::is_control) {
            return Err(ListError::TitleControlCharacter);
        }
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            return Err(ListError::EmptyTitle);
        }
        if collapsed.chars().count() > LIST_TITLE_MAX_CHARS {
            return Err(ListError::TitleTooLong);
        }
        Ok(Self(collapsed))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The default title for a list: the seed's title cleaned and cut to 120 characters at a
/// character boundary, or [`LIST_TITLE_FALLBACK`] when there is nothing usable.
#[must_use]
pub fn default_list_title(seed_title: Option<&str>) -> ListTitle {
    let cleaned: String = seed_title
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_whitespace() || !character.is_control())
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(LIST_TITLE_MAX_CHARS).collect();
    ListTitle::parse(truncated.trim_end()).unwrap_or_else(|_| ListTitle(LIST_TITLE_FALLBACK.into()))
}

/// A validated topic label: trimmed, single-spaced, 1 to 80 characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicLabel(String);

impl TopicLabel {
    /// Trim, collapse whitespace runs to one space, and validate.
    pub fn parse(text: &str) -> Result<Self, ListError> {
        if text.chars().any(char::is_control) {
            return Err(ListError::InvalidTopicLabel);
        }
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let length = collapsed.chars().count();
        if length == 0 || length > LIST_TOPIC_LABEL_MAX_CHARS {
            return Err(ListError::InvalidTopicLabel);
        }
        Ok(Self(collapsed))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A list description: trimmed, blank means none, at most 4,000 characters
/// (`parse_mark_note`'s rules with the description limit).
pub fn parse_list_description(text: Option<&str>) -> Result<Option<String>, ListError> {
    let Some(trimmed) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > LIST_DESCRIPTION_MAX_CHARS {
        return Err(ListError::DescriptionTooLong);
    }
    Ok(Some(trimmed.to_owned()))
}

/// What the ordering needs to know about one list item.
#[derive(Clone, Debug, PartialEq)]
pub struct ListOrderItem {
    pub work_id: WorkId,
    pub depth: usize,
    pub score: f64,
    pub year: Option<i32>,
    pub title_key: String,
    pub is_seed: bool,
}

/// Higher priority sorts first: deeper, then more prominent, then older (unknown last),
/// then by title key and ID.
fn priority(left: &ListOrderItem, right: &ListOrderItem) -> std::cmp::Ordering {
    right
        .depth
        .cmp(&left.depth)
        .then_with(|| right.score.total_cmp(&left.score))
        .then_with(|| match (left.year, right.year) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .then_with(|| left.title_key.cmp(&right.title_key))
        .then_with(|| left.work_id.cmp(&right.work_id))
}

/// The "foundational first" order.
///
/// A cited item comes before the item citing it, ties go to the priority above, and the
/// seed is always last. `cites` holds `(citing, cited)` pairs;
/// pairs mentioning Works outside `items`, self pairs, and pairs involving the seed are
/// ignored. When only cycles remain, the best remaining item is taken.
#[must_use]
pub fn suggest_order(items: &[ListOrderItem], cites: &[(WorkId, WorkId)]) -> Vec<WorkId> {
    let mut seen = HashSet::new();
    let unique: Vec<&ListOrderItem> = items
        .iter()
        .filter(|item| seen.insert(item.work_id))
        .collect();
    let (seeds, body): (Vec<&ListOrderItem>, Vec<&ListOrderItem>) =
        unique.into_iter().partition(|item| item.is_seed);

    let index_of: HashMap<WorkId, usize> = body
        .iter()
        .enumerate()
        .map(|(index, item)| (item.work_id, index))
        .collect();
    let mut unmet = vec![0usize; body.len()];
    let mut dependants: Vec<Vec<usize>> = vec![Vec::new(); body.len()];
    let mut pairs = HashSet::new();
    for &(citing, cited) in cites {
        let (Some(&citing_index), Some(&cited_index)) =
            (index_of.get(&citing), index_of.get(&cited))
        else {
            continue;
        };
        if citing_index == cited_index || !pairs.insert((citing_index, cited_index)) {
            continue;
        }
        unmet[citing_index] += 1;
        dependants[cited_index].push(citing_index);
    }

    let mut remaining: Vec<usize> = (0..body.len()).collect();
    let mut ordered = Vec::with_capacity(items.len());
    while !remaining.is_empty() {
        let best = |candidates: &mut dyn Iterator<Item = usize>| {
            candidates.min_by(|&a, &b| priority(body[a], body[b]))
        };
        let pick = best(&mut remaining.iter().copied().filter(|&index| unmet[index] == 0))
            .or_else(|| best(&mut remaining.iter().copied()))
            .expect("remaining is not empty");
        remaining.retain(|&index| index != pick);
        ordered.push(body[pick].work_id);
        for &dependant in &dependants[pick] {
            unmet[dependant] = unmet[dependant].saturating_sub(1);
        }
    }

    let mut seeds = seeds;
    seeds.sort_by(|left, right| priority(left, right));
    ordered.extend(seeds.into_iter().map(|item| item.work_id));
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn id(number: u128) -> WorkId {
        WorkId::from_uuid(Uuid::from_u128(number))
    }

    fn item(number: u128, depth: usize, score: f64, year: Option<i32>) -> ListOrderItem {
        ListOrderItem {
            work_id: id(number),
            depth,
            score,
            year,
            title_key: format!("title {number}"),
            is_seed: false,
        }
    }

    fn seed(number: u128) -> ListOrderItem {
        ListOrderItem {
            is_seed: true,
            ..item(number, 0, 1.0, Some(2020))
        }
    }

    #[test]
    fn should_collapse_whitespace_when_parsing_list_title() {
        let title = ListTitle::parse("  Learning \u{2003} to   summarize  ").expect("parse");
        assert_eq!(title.as_str(), "Learning to summarize");
    }

    #[test]
    fn should_reject_empty_long_and_control_character_list_titles() {
        assert_eq!(ListTitle::parse(""), Err(ListError::EmptyTitle));
        assert_eq!(ListTitle::parse(" \u{2003} "), Err(ListError::EmptyTitle));
        assert_eq!(
            ListTitle::parse("a\u{0}b"),
            Err(ListError::TitleControlCharacter)
        );
        assert_eq!(
            ListTitle::parse("a\nb"),
            Err(ListError::TitleControlCharacter)
        );
        assert!(ListTitle::parse(&"\u{e9}".repeat(120)).is_ok());
        assert_eq!(
            ListTitle::parse(&"\u{e9}".repeat(121)),
            Err(ListError::TitleTooLong)
        );
        for error in [
            ListError::EmptyTitle,
            ListError::TitleTooLong,
            ListError::TitleControlCharacter,
        ] {
            assert_eq!(error.code(), "invalid_list_title");
        }
    }

    #[test]
    fn should_expose_stable_error_codes_when_list_changes_conflict() {
        let codes: Vec<&str> = [
            ListError::ItemExists,
            ListError::ViaNotInList,
            ListError::NoCitationEdge,
            ListError::TooDeep,
            ListError::Full,
            ListError::SeedItem,
            ListError::InvalidOrder,
            ListError::TopicsFull,
            ListError::EdgesFull,
            ListError::PaperNotLinked,
            ListError::EdgeCycle,
            ListError::InvalidEdgeKind,
            ListError::DescriptionTooLong,
            ListError::InvalidTopicLabel,
        ]
        .iter()
        .map(|error| error.code())
        .collect();
        assert_eq!(
            codes,
            [
                "item_exists",
                "via_not_in_list",
                "no_citation_edge",
                "ramp_too_deep",
                "list_full",
                "seed_item",
                "invalid_order",
                "topics_full",
                "edges_full",
                "paper_not_linked",
                "edge_cycle",
                "invalid_edge_kind",
                "invalid_description",
                "invalid_topic_label"
            ]
        );
    }

    #[test]
    fn should_enforce_limits_when_parsing_topic_label_and_description() {
        assert_eq!(
            TopicLabel::parse("  reward \u{2003} models ")
                .expect("label")
                .as_str(),
            "reward models"
        );
        assert!(TopicLabel::parse(&"\u{e9}".repeat(80)).is_ok());
        for bad in ["", "   ", "a\nb", &"x".repeat(81)] {
            assert_eq!(TopicLabel::parse(bad), Err(ListError::InvalidTopicLabel));
        }
        assert_eq!(parse_list_description(None), Ok(None));
        assert_eq!(parse_list_description(Some("  \n ")), Ok(None));
        assert_eq!(
            parse_list_description(Some("  why  ")),
            Ok(Some("why".to_owned()))
        );
        assert!(parse_list_description(Some(&"\u{e9}".repeat(4_000))).is_ok());
        assert_eq!(
            parse_list_description(Some(&"\u{e9}".repeat(4_001))),
            Err(ListError::DescriptionTooLong)
        );
    }

    #[test]
    fn should_truncate_at_char_boundary_when_defaulting_list_title() {
        let long = "\u{e9}\u{1f600}".repeat(100);
        let title = default_list_title(Some(&long));
        assert_eq!(title.as_str().chars().count(), 120);
        assert!(long.starts_with(title.as_str()));
        let spaced = format!("{} tail", "a".repeat(119));
        assert_eq!(default_list_title(Some(&spaced)).as_str(), "a".repeat(119));
        let messy = default_list_title(Some("A\nB\u{0}  C"));
        assert_eq!(messy.as_str(), "A B C");
    }

    #[test]
    fn should_fall_back_when_seed_title_is_missing_or_blank() {
        for input in [None, Some(""), Some("   "), Some("\u{0}\u{7}")] {
            assert_eq!(default_list_title(input).as_str(), LIST_TITLE_FALLBACK);
        }
    }

    #[test]
    fn should_put_cited_item_before_citing_item_when_suggesting_order() {
        // `citing` is deeper and more prominent, but cites `cited`.
        let items = [item(1, 3, 0.9, Some(2020)), item(2, 1, 0.1, Some(2000))];
        assert_eq!(suggest_order(&items, &[(id(1), id(2))]), vec![id(2), id(1)]);
        assert_eq!(suggest_order(&items, &[]), vec![id(1), id(2)]);
    }

    #[test]
    fn should_order_deeper_then_more_prominent_then_older_when_unconstrained() {
        let items = [
            item(1, 1, 0.9, Some(2000)),
            item(2, 2, 0.1, Some(2020)),
            item(3, 2, 0.5, Some(2020)),
            item(4, 2, 0.5, Some(2010)),
            item(5, 2, 0.5, None),
        ];
        assert_eq!(
            suggest_order(&items, &[]),
            vec![id(4), id(3), id(5), id(2), id(1)]
        );
        let tied = [
            ListOrderItem {
                title_key: "b".into(),
                ..item(1, 1, 0.5, Some(2000))
            },
            ListOrderItem {
                title_key: "a".into(),
                ..item(2, 1, 0.5, Some(2000))
            },
            ListOrderItem {
                title_key: "a".into(),
                ..item(0, 1, 0.5, Some(2000))
            },
        ];
        assert_eq!(suggest_order(&tied, &[]), vec![id(0), id(2), id(1)]);
    }

    #[test]
    fn should_place_seed_last_when_suggesting_order() {
        let items = [item(1, 1, 0.1, None), seed(9), item(2, 2, 0.1, None)];
        assert_eq!(suggest_order(&items, &[]), vec![id(2), id(1), id(9)]);
        // Even when an item cites the seed or the seed cites an item.
        assert_eq!(
            suggest_order(&items, &[(id(1), id(9)), (id(9), id(2))]),
            vec![id(2), id(1), id(9)]
        );
    }

    #[test]
    fn should_break_cycles_deterministically_when_items_cite_each_other() {
        let items = [
            item(1, 1, 0.2, Some(2000)),
            item(2, 1, 0.9, Some(2000)),
            item(3, 1, 0.5, Some(2000)),
        ];
        let cites = [(id(1), id(2)), (id(2), id(1))];
        // 3 is free of the cycle and goes first; then 2 is the better of the cycle.
        let order = suggest_order(&items, &cites);
        assert_eq!(order, vec![id(3), id(2), id(1)]);
        let mut reversed = items.to_vec();
        reversed.reverse();
        assert_eq!(suggest_order(&reversed, &cites), order);
    }

    #[test]
    fn should_ignore_unknown_and_self_citations_when_suggesting_order() {
        let items = [item(1, 2, 0.9, None), item(2, 1, 0.1, None)];
        let cites = [
            (id(1), id(1)),
            (id(1), id(99)),
            (id(99), id(2)),
            (id(1), id(2)),
            (id(1), id(2)),
        ];
        assert_eq!(suggest_order(&items, &cites), vec![id(2), id(1)]);
        let without = [(id(1), id(1)), (id(1), id(99)), (id(99), id(2))];
        assert_eq!(suggest_order(&items, &without), vec![id(1), id(2)]);
    }

    #[test]
    fn should_keep_first_when_item_listed_twice() {
        let items = [
            item(1, 1, 0.1, None),
            item(1, 5, 0.9, None),
            item(2, 1, 0.0, None),
        ];
        assert_eq!(suggest_order(&items, &[]), vec![id(1), id(2)]);
    }

    fn permutations(
        items: &mut Vec<ListOrderItem>,
        start: usize,
        out: &mut Vec<Vec<ListOrderItem>>,
    ) {
        if start == items.len() {
            out.push(items.clone());
            return;
        }
        for swap in start..items.len() {
            items.swap(start, swap);
            permutations(items, start + 1, out);
            items.swap(start, swap);
        }
    }

    #[test]
    fn should_be_deterministic_when_input_order_is_shuffled() {
        let base = vec![
            item(1, 2, 0.4, Some(2001)),
            item(2, 2, 0.4, Some(2001)),
            item(3, 1, 0.8, Some(1999)),
            item(4, 3, 0.1, None),
            item(5, 1, 0.8, Some(1999)),
            seed(6),
        ];
        let cites = [
            (id(1), id(3)),
            (id(3), id(4)),
            (id(4), id(1)),
            (id(2), id(5)),
        ];
        let mut base = base;
        let expected = suggest_order(&base, &cites);
        assert_eq!(expected.len(), 6);
        assert_eq!(expected.last(), Some(&id(6)));
        let mut all = Vec::new();
        permutations(&mut base, 0, &mut all);
        assert_eq!(all.len(), 720);
        for permutation in all {
            assert_eq!(suggest_order(&permutation, &cites), expected);
        }
    }

    #[test]
    fn should_return_empty_when_no_items_given() {
        assert!(suggest_order(&[], &[]).is_empty());
        assert!(suggest_order(&[], &[(id(1), id(2))]).is_empty());
    }
}
