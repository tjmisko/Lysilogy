//! Choosing which Works a bounded walk expands next. Pure and deterministic.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::BuildHasher;

use lysilogy_core::{WorkId, WorkMark};

/// One child of an expanded parent, as the planner sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrontierCandidate {
    pub parent: WorkId,
    pub work_id: WorkId,
    pub score: f64,
    pub year: Option<i32>,
    pub mark: Option<WorkMark>,
}

/// Newer first, unknown years last.
fn year_order(left: Option<i32>, right: Option<i32>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// The `(parent, child)` pairs to expand next.
///
/// Parents are visited in `parents_in_order`. Each takes its first `fanout`
/// children by score (highest first), then year (newest first, unknown last),
/// then Work ID, skipping hidden and downplayed children, Works in `visited`,
/// and Works an earlier parent already claimed. Planning stops once `budget`
/// pairs are chosen.
#[must_use]
pub fn plan_frontier<S: BuildHasher>(
    candidates: &[FrontierCandidate],
    parents_in_order: &[WorkId],
    fanout: usize,
    visited: &HashSet<WorkId, S>,
    budget: usize,
) -> Vec<(WorkId, WorkId)> {
    let mut chosen: HashSet<WorkId> = HashSet::new();
    let mut plan: Vec<(WorkId, WorkId)> = Vec::new();
    for &parent in parents_in_order {
        let mut children: Vec<&FrontierCandidate> = candidates
            .iter()
            .filter(|candidate| candidate.parent == parent)
            .filter(|candidate| {
                !matches!(
                    candidate.mark,
                    Some(WorkMark::Hidden | WorkMark::Downplayed)
                )
            })
            .collect();
        children.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| year_order(left.year, right.year))
                .then_with(|| left.work_id.cmp(&right.work_id))
        });
        let mut taken = 0;
        for child in children {
            if taken >= fanout || plan.len() >= budget {
                break;
            }
            if visited.contains(&child.work_id) || !chosen.insert(child.work_id) {
                continue;
            }
            plan.push((parent, child.work_id));
            taken += 1;
        }
        if plan.len() >= budget {
            break;
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(parent: WorkId, work_id: WorkId, score: f64) -> FrontierCandidate {
        FrontierCandidate {
            parent,
            work_id,
            score,
            year: None,
            mark: None,
        }
    }

    fn ids(count: usize) -> Vec<WorkId> {
        let mut ids: Vec<WorkId> = (0..count).map(|_| WorkId::new()).collect();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn should_take_top_fanout_children_per_parent_by_score_when_planning() {
        let works = ids(8);
        let (first, second) = (works[0], works[1]);
        let candidates = [
            candidate(first, works[2], 0.1),
            candidate(first, works[3], 0.9),
            candidate(first, works[4], 0.5),
            candidate(second, works[5], 0.2),
            candidate(second, works[6], 0.8),
            candidate(second, works[7], 0.4),
        ];
        let plan = plan_frontier(&candidates, &[first, second], 2, &HashSet::new(), 10);
        assert_eq!(
            plan,
            [
                (first, works[3]),
                (first, works[4]),
                (second, works[6]),
                (second, works[7]),
            ]
        );
        let parents_reversed = plan_frontier(&candidates, &[second, first], 1, &HashSet::new(), 10);
        assert_eq!(parents_reversed, [(second, works[6]), (first, works[3])]);
    }

    #[test]
    fn should_skip_hidden_downplayed_visited_and_seed_when_planning() {
        let works = ids(7);
        let parent = works[0];
        let seed = works[1];
        let mut hidden = candidate(parent, works[2], 0.99);
        hidden.mark = Some(WorkMark::Hidden);
        let mut downplayed = candidate(parent, works[3], 0.98);
        downplayed.mark = Some(WorkMark::Downplayed);
        let mut pinned = candidate(parent, works[4], 0.1);
        pinned.mark = Some(WorkMark::Pinned);
        let candidates = [
            hidden,
            downplayed,
            candidate(parent, seed, 0.97),
            candidate(parent, works[5], 0.96),
            pinned,
            candidate(parent, works[6], 0.5),
        ];
        let visited = HashSet::from([seed, works[5]]);
        let plan = plan_frontier(&candidates, &[parent], 5, &visited, 10);
        assert_eq!(plan, [(parent, works[6]), (parent, works[4])]);
    }

    #[test]
    fn should_stop_at_budget_when_planning_frontier() {
        let works = ids(6);
        let (first, second) = (works[0], works[1]);
        let candidates = [
            candidate(first, works[2], 0.9),
            candidate(first, works[3], 0.8),
            candidate(second, works[4], 0.7),
            candidate(second, works[5], 0.6),
        ];
        let visited = HashSet::new();
        assert_eq!(
            plan_frontier(&candidates, &[first, second], 2, &visited, 3),
            [(first, works[2]), (first, works[3]), (second, works[4])]
        );
        assert_eq!(
            plan_frontier(&candidates, &[first, second], 2, &visited, 1),
            [(first, works[2])]
        );
        assert!(plan_frontier(&candidates, &[first, second], 2, &visited, 0).is_empty());
        assert!(plan_frontier(&candidates, &[first, second], 0, &visited, 5).is_empty());
        assert!(plan_frontier(&[], &[first], 3, &visited, 5).is_empty());
    }

    #[test]
    fn should_break_ties_by_year_then_id_when_scores_equal() {
        let works = ids(6);
        let parent = works[0];
        let mut older = candidate(parent, works[1], 0.5);
        older.year = Some(2001);
        let mut newer = candidate(parent, works[2], 0.5);
        newer.year = Some(2020);
        let undated = candidate(parent, works[3], 0.5);
        let mut tied_first = candidate(parent, works[4], 0.5);
        tied_first.year = Some(2010);
        let mut tied_second = candidate(parent, works[5], 0.5);
        tied_second.year = Some(2010);
        let candidates = [tied_second, undated, older, tied_first, newer];
        let plan = plan_frontier(&candidates, &[parent], 5, &HashSet::new(), 10);
        let ranked: Vec<WorkId> = plan.iter().map(|(_, child)| *child).collect();
        assert_eq!(ranked, [works[2], works[4], works[5], works[1], works[3]]);
    }

    #[test]
    fn should_be_deterministic_when_candidates_are_shuffled() {
        let works = ids(9);
        let (first, second) = (works[0], works[1]);
        let mut candidates = vec![
            candidate(first, works[2], 0.5),
            candidate(first, works[3], 0.5),
            candidate(first, works[4], 0.7),
            candidate(second, works[3], 0.9),
            candidate(second, works[5], 0.5),
            candidate(second, works[6], 0.5),
            candidate(second, works[7], f64::NAN),
            candidate(second, works[8], 0.1),
        ];
        let expected = plan_frontier(&candidates, &[first, second], 3, &HashSet::new(), 20);
        assert_eq!(expected.len(), 6);
        for rotation in 0..candidates.len() {
            candidates.rotate_left(1);
            assert_eq!(
                plan_frontier(&candidates, &[first, second], 3, &HashSet::new(), 20),
                expected,
                "rotation {rotation}"
            );
        }
        candidates.reverse();
        assert_eq!(
            plan_frontier(&candidates, &[first, second], 3, &HashSet::new(), 20),
            expected
        );
    }
}
