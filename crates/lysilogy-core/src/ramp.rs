//! Reading ramps: validated titles, error codes, and the "foundational first" order.

use std::collections::{HashMap, HashSet};

use crate::WorkId;

/// Most Works on one item's path (seed or root through the item), so depth is at most 8.
pub const RAMP_PATH_MAX: usize = 9;
/// Most items one ramp holds.
pub const RAMP_ITEMS_MAX: usize = 500;
/// Longest ramp title in characters.
pub const RAMP_TITLE_MAX_CHARS: usize = 120;
/// Title used when the seed has none.
pub const RAMP_TITLE_FALLBACK: &str = "Reading ramp";

/// Why ramp input or a ramp change was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RampError {
    #[error("ramp title must not be empty")]
    EmptyTitle,
    #[error("ramp title must be at most {RAMP_TITLE_MAX_CHARS} characters")]
    TitleTooLong,
    #[error("ramp title must not contain control characters")]
    TitleControlCharacter,
    #[error("the Work is already in the ramp")]
    ItemExists,
    #[error("the Work it was reached from is not in the ramp")]
    ViaNotInRamp,
    #[error("the Work it was reached from does not cite it")]
    NoCitationEdge,
    #[error("the path would be longer than {RAMP_PATH_MAX} Works")]
    TooDeep,
    #[error("a ramp holds at most {RAMP_ITEMS_MAX} items")]
    Full,
    #[error("the seed item cannot be removed")]
    SeedItem,
    #[error("the order must list every item exactly once")]
    InvalidOrder,
}

impl RampError {
    /// The machine-readable API error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptyTitle | Self::TitleTooLong | Self::TitleControlCharacter => {
                "invalid_ramp_title"
            }
            Self::ItemExists => "item_exists",
            Self::ViaNotInRamp => "via_not_in_ramp",
            Self::NoCitationEdge => "no_citation_edge",
            Self::TooDeep => "ramp_too_deep",
            Self::Full => "ramp_full",
            Self::SeedItem => "seed_item",
            Self::InvalidOrder => "invalid_order",
        }
    }
}

/// A validated ramp title: trimmed, single-spaced, 1 to 120 characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RampTitle(String);

impl RampTitle {
    /// Trim, collapse whitespace runs to one space, and validate.
    pub fn parse(text: &str) -> Result<Self, RampError> {
        if text.chars().any(char::is_control) {
            return Err(RampError::TitleControlCharacter);
        }
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            return Err(RampError::EmptyTitle);
        }
        if collapsed.chars().count() > RAMP_TITLE_MAX_CHARS {
            return Err(RampError::TitleTooLong);
        }
        Ok(Self(collapsed))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The default title for a ramp: the seed's title cleaned and cut to 120 characters at a
/// character boundary, or [`RAMP_TITLE_FALLBACK`] when there is nothing usable.
#[must_use]
pub fn default_ramp_title(seed_title: Option<&str>) -> RampTitle {
    let cleaned: String = seed_title
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_whitespace() || !character.is_control())
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(RAMP_TITLE_MAX_CHARS).collect();
    RampTitle::parse(truncated.trim_end()).unwrap_or_else(|_| RampTitle(RAMP_TITLE_FALLBACK.into()))
}

/// What the ordering needs to know about one ramp item.
#[derive(Clone, Debug, PartialEq)]
pub struct RampOrderItem {
    pub work_id: WorkId,
    pub depth: usize,
    pub score: f64,
    pub year: Option<i32>,
    pub title_key: String,
    pub is_seed: bool,
}

/// Higher priority sorts first: deeper, then more prominent, then older (unknown last),
/// then by title key and ID.
fn priority(left: &RampOrderItem, right: &RampOrderItem) -> std::cmp::Ordering {
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
pub fn suggest_order(items: &[RampOrderItem], cites: &[(WorkId, WorkId)]) -> Vec<WorkId> {
    let mut seen = HashSet::new();
    let unique: Vec<&RampOrderItem> = items
        .iter()
        .filter(|item| seen.insert(item.work_id))
        .collect();
    let (seeds, body): (Vec<&RampOrderItem>, Vec<&RampOrderItem>) =
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

    fn item(number: u128, depth: usize, score: f64, year: Option<i32>) -> RampOrderItem {
        RampOrderItem {
            work_id: id(number),
            depth,
            score,
            year,
            title_key: format!("title {number}"),
            is_seed: false,
        }
    }

    fn seed(number: u128) -> RampOrderItem {
        RampOrderItem {
            is_seed: true,
            ..item(number, 0, 1.0, Some(2020))
        }
    }

    #[test]
    fn should_collapse_whitespace_when_parsing_ramp_title() {
        let title = RampTitle::parse("  Learning \u{2003} to   summarize  ").expect("parse");
        assert_eq!(title.as_str(), "Learning to summarize");
    }

    #[test]
    fn should_reject_empty_long_and_control_character_ramp_titles() {
        assert_eq!(RampTitle::parse(""), Err(RampError::EmptyTitle));
        assert_eq!(RampTitle::parse(" \u{2003} "), Err(RampError::EmptyTitle));
        assert_eq!(
            RampTitle::parse("a\u{0}b"),
            Err(RampError::TitleControlCharacter)
        );
        assert_eq!(
            RampTitle::parse("a\nb"),
            Err(RampError::TitleControlCharacter)
        );
        assert!(RampTitle::parse(&"\u{e9}".repeat(120)).is_ok());
        assert_eq!(
            RampTitle::parse(&"\u{e9}".repeat(121)),
            Err(RampError::TitleTooLong)
        );
        for error in [
            RampError::EmptyTitle,
            RampError::TitleTooLong,
            RampError::TitleControlCharacter,
        ] {
            assert_eq!(error.code(), "invalid_ramp_title");
        }
    }

    #[test]
    fn should_expose_stable_error_codes_when_ramp_changes_conflict() {
        let codes: Vec<&str> = [
            RampError::ItemExists,
            RampError::ViaNotInRamp,
            RampError::NoCitationEdge,
            RampError::TooDeep,
            RampError::Full,
            RampError::SeedItem,
            RampError::InvalidOrder,
        ]
        .iter()
        .map(|error| error.code())
        .collect();
        assert_eq!(
            codes,
            [
                "item_exists",
                "via_not_in_ramp",
                "no_citation_edge",
                "ramp_too_deep",
                "ramp_full",
                "seed_item",
                "invalid_order"
            ]
        );
    }

    #[test]
    fn should_truncate_at_char_boundary_when_defaulting_ramp_title() {
        let long = "\u{e9}\u{1f600}".repeat(100);
        let title = default_ramp_title(Some(&long));
        assert_eq!(title.as_str().chars().count(), 120);
        assert!(long.starts_with(title.as_str()));
        let spaced = format!("{} tail", "a".repeat(119));
        assert_eq!(default_ramp_title(Some(&spaced)).as_str(), "a".repeat(119));
        let messy = default_ramp_title(Some("A\nB\u{0}  C"));
        assert_eq!(messy.as_str(), "A B C");
    }

    #[test]
    fn should_fall_back_when_seed_title_is_missing_or_blank() {
        for input in [None, Some(""), Some("   "), Some("\u{0}\u{7}")] {
            assert_eq!(default_ramp_title(input).as_str(), RAMP_TITLE_FALLBACK);
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
            RampOrderItem {
                title_key: "b".into(),
                ..item(1, 1, 0.5, Some(2000))
            },
            RampOrderItem {
                title_key: "a".into(),
                ..item(2, 1, 0.5, Some(2000))
            },
            RampOrderItem {
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
        items: &mut Vec<RampOrderItem>,
        start: usize,
        out: &mut Vec<Vec<RampOrderItem>>,
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
