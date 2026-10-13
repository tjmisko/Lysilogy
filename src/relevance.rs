//! Pure ranking and clustering of a Work's neighborhood (Ramp D, Decisions 8-11).
//!
//! The center Work is never ranked. Candidates are ordered pinned first, then
//! unmarked by score; downplayed Works are always collapsed and hidden Works
//! are excluded from the totals. Whatever is not visible lands in exactly one
//! cluster, so the overlay can offer to expand it.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
};

use lysilogy_core::{TagId, Work, WorkId, WorkMark, title_key};
use serde::Serialize;

/// Titles shown per cluster.
const SAMPLE_TITLES: usize = 3;
/// Placeholder for a Work without a usable title.
const UNTITLED: &str = "Untitled";
/// Years outside this range are treated as unknown when clustering by decade.
const VALID_YEARS: std::ops::RangeInclusive<i32> = 1..=2200;

/// A tag as the ranking sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagRef {
    pub id: TagId,
    pub name: String,
}

/// One neighbor with everything ranking needs.
#[derive(Clone, Copy, Debug)]
pub struct NeighborCandidate<'a> {
    pub work: &'a Work,
    pub score: f64,
    pub mark: Option<WorkMark>,
    pub tags: &'a [TagRef],
}

/// How much to show.
#[derive(Clone, Copy, Debug)]
pub struct RankingRequest<'a> {
    /// `None` shows every non-hidden, non-downplayed neighbor.
    pub limit: Option<usize>,
    /// Cluster keys whose members become visible. Unknown keys are ignored.
    pub expand: &'a HashSet<String>,
    /// A neighbor passes when it has any of these tags.
    pub tag_filter: Option<&'a HashSet<TagId>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterKind {
    Tag,
    Decade,
    Downplayed,
    Hidden,
}

/// A group of collapsed neighbors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Cluster {
    pub key: String,
    pub kind: ClusterKind,
    pub label: String,
    pub count: usize,
    /// Members in rank order.
    pub work_ids: Vec<WorkId>,
    pub sample_titles: Vec<String>,
}

/// The ranked view of a neighborhood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedNeighborhood {
    /// Visible Works in rank order: top-N first, then expanded members.
    pub visible: Vec<WorkId>,
    /// Collapsed neighbors, each in exactly one cluster.
    pub clusters: Vec<Cluster>,
    /// Neighbors that passed the filter, excluding hidden ones.
    pub total: usize,
    /// Hidden neighbors that passed the filter.
    pub hidden: usize,
    /// Pinned neighbors that passed the filter.
    pub pinned: usize,
    /// Neighbors removed by the tag filter.
    pub filtered_out: usize,
}

struct Entry<'a> {
    candidate: NeighborCandidate<'a>,
    id: WorkId,
    year: Option<i32>,
    title_key: String,
}

impl Entry<'_> {
    const fn class(&self) -> u8 {
        match self.candidate.mark {
            Some(WorkMark::Pinned) => 0,
            None => 1,
            Some(WorkMark::Downplayed) => 2,
            Some(WorkMark::Hidden) => 3,
        }
    }

    fn display_title(&self) -> String {
        self.candidate
            .work
            .display_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map_or_else(|| UNTITLED.to_owned(), str::to_owned)
    }

    /// Smallest `lower(name)` tag, ties by id.
    fn first_tag(&self) -> Option<&TagRef> {
        self.candidate
            .tags
            .iter()
            .min_by(|a, b| (a.name.to_lowercase(), a.id).cmp(&(b.name.to_lowercase(), b.id)))
    }

    fn decade(&self) -> Option<i32> {
        self.year
            .filter(|year| VALID_YEARS.contains(year))
            .map(|year| year / 10 * 10)
    }
}

fn rank_order(a: &Entry<'_>, b: &Entry<'_>) -> Ordering {
    a.class()
        .cmp(&b.class())
        .then_with(|| b.candidate.score.total_cmp(&a.candidate.score))
        .then_with(|| match (a.year, b.year) {
            (Some(left), Some(right)) => right.cmp(&left),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| a.title_key.cmp(&b.title_key))
        .then_with(|| a.id.cmp(&b.id))
}

/// Cluster order: group, name, negated decade, key.
type SlotOrder = (u8, String, i64, String);

/// Which cluster a collapsed entry belongs to.
struct ClusterSlot {
    order: SlotOrder,
    key: String,
    kind: ClusterKind,
    label: String,
}

fn cluster_slot(entry: &Entry<'_>) -> ClusterSlot {
    match entry.candidate.mark {
        Some(WorkMark::Hidden) => {
            return ClusterSlot {
                order: (4, String::new(), 0, "hidden".to_owned()),
                key: "hidden".to_owned(),
                kind: ClusterKind::Hidden,
                label: "Hidden".to_owned(),
            };
        }
        Some(WorkMark::Downplayed) => {
            return ClusterSlot {
                order: (3, String::new(), 0, "downplayed".to_owned()),
                key: "downplayed".to_owned(),
                kind: ClusterKind::Downplayed,
                label: "Downplayed".to_owned(),
            };
        }
        _ => {}
    }
    if let Some(tag) = entry.first_tag() {
        return ClusterSlot {
            order: (0, tag.name.to_lowercase(), 0, format!("tag:{}", tag.id)),
            key: format!("tag:{}", tag.id),
            kind: ClusterKind::Tag,
            label: tag.name.clone(),
        };
    }
    let Some(decade) = entry.decade() else {
        return ClusterSlot {
            order: (2, String::new(), 0, String::new()),
            key: "decade:unknown".to_owned(),
            kind: ClusterKind::Decade,
            label: "Year unknown".to_owned(),
        };
    };
    ClusterSlot {
        order: (1, String::new(), -i64::from(decade), String::new()),
        key: format!("decade:{decade:04}"),
        kind: ClusterKind::Decade,
        label: format!("{decade}s"),
    }
}

/// Rank the neighbors of `center`.
///
/// Duplicate candidates and the center itself are ignored.
#[must_use]
pub fn rank_neighborhood(
    center: WorkId,
    candidates: &[NeighborCandidate<'_>],
    request: &RankingRequest<'_>,
) -> RankedNeighborhood {
    let mut seen = HashSet::from([center]);
    let mut filtered_out = 0;
    let mut entries: Vec<Entry<'_>> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !seen.insert(candidate.work.id) {
            continue;
        }
        if let Some(filter) = request.tag_filter
            && !candidate.tags.iter().any(|tag| filter.contains(&tag.id))
        {
            filtered_out += 1;
            continue;
        }
        entries.push(Entry {
            candidate: *candidate,
            id: candidate.work.id,
            year: candidate.work.display_year,
            title_key: title_key(candidate.work.display_title.as_deref().unwrap_or("")),
        });
    }
    entries.sort_by(rank_order);

    let hidden = entries.iter().filter(|entry| entry.class() == 3).count();
    let pinned = entries.iter().filter(|entry| entry.class() == 0).count();
    let total = entries.len() - hidden;

    // Pinned and unmarked entries sort first; the top-N is a prefix of them.
    let shown_classes = entries.iter().filter(|entry| entry.class() <= 1).count();
    let top = request
        .limit
        .map_or(shown_classes, |limit| limit.max(pinned).min(shown_classes));

    let mut slots: BTreeMap<SlotOrder, (ClusterSlot, Vec<usize>)> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate().skip(top) {
        let slot = cluster_slot(entry);
        let sort_key = slot.order.clone();
        slots
            .entry(sort_key)
            .or_insert_with(|| (slot, Vec::new()))
            .1
            .push(index);
    }

    let mut expanded_indexes: Vec<usize> = Vec::new();
    let mut clusters = Vec::with_capacity(slots.len());
    for (slot, members) in slots.into_values() {
        if request.expand.contains(&slot.key) {
            expanded_indexes.extend(members);
            continue;
        }
        clusters.push(Cluster {
            key: slot.key,
            kind: slot.kind,
            label: slot.label,
            count: members.len(),
            sample_titles: members
                .iter()
                .take(SAMPLE_TITLES)
                .map(|&index| entries[index].display_title())
                .collect(),
            work_ids: members.iter().map(|&index| entries[index].id).collect(),
        });
    }
    expanded_indexes.sort_unstable();
    let visible_indexes = (0..top).chain(expanded_indexes);

    RankedNeighborhood {
        visible: visible_indexes.map(|index| entries[index].id).collect(),
        clusters,
        total,
        hidden,
        pinned,
        filtered_out,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use lysilogy_core::{FieldState, Visibility, WorkKind};

    use super::*;

    fn work(title: Option<&str>, year: Option<i32>) -> Work {
        Work {
            id: WorkId::new(),
            kind: WorkKind::Article,
            visibility: Visibility::Private,
            display_title: title.map(str::to_owned),
            title_state: FieldState::Known,
            display_year: year,
            year_state: FieldState::Known,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn tag(name: &str) -> TagRef {
        TagRef {
            id: TagId::new(),
            name: name.to_owned(),
        }
    }

    struct Row {
        work: Work,
        score: f64,
        mark: Option<WorkMark>,
        tags: Vec<TagRef>,
    }

    fn row(title: &str, year: Option<i32>, score: f64) -> Row {
        Row {
            work: work(Some(title), year),
            score,
            mark: None,
            tags: Vec::new(),
        }
    }

    fn marked(mut row: Row, mark: WorkMark) -> Row {
        row.mark = Some(mark);
        row
    }

    fn tagged(mut row: Row, tags: &[&TagRef]) -> Row {
        row.tags = tags.iter().map(|tag| (*tag).clone()).collect();
        row
    }

    fn candidates(rows: &[Row]) -> Vec<NeighborCandidate<'_>> {
        rows.iter()
            .map(|row| NeighborCandidate {
                work: &row.work,
                score: row.score,
                mark: row.mark,
                tags: &row.tags,
            })
            .collect()
    }

    fn rank(
        rows: &[Row],
        limit: Option<usize>,
        expand: &[&str],
        filter: Option<&HashSet<TagId>>,
    ) -> RankedNeighborhood {
        let expand: HashSet<String> = expand.iter().map(|key| (*key).to_owned()).collect();
        rank_neighborhood(
            WorkId::new(),
            &candidates(rows),
            &RankingRequest {
                limit,
                expand: &expand,
                tag_filter: filter,
            },
        )
    }

    fn ids(rows: &[&Row]) -> Vec<WorkId> {
        rows.iter().map(|row| row.work.id).collect()
    }

    fn cluster_keys(ranked: &RankedNeighborhood) -> Vec<&str> {
        ranked
            .clusters
            .iter()
            .map(|cluster| cluster.key.as_str())
            .collect()
    }

    #[test]
    fn should_order_by_score_then_year_then_title_then_id_when_ranking() {
        let high = row("Zeta", Some(1990), 0.9);
        let newer = row("Zeta", Some(2010), 0.5);
        let older = row("Zeta", Some(2000), 0.5);
        let undated = row("Zeta", None, 0.5);
        let alpha = row("Alpha", None, 0.5);
        let mut twins = [row("Same", None, 0.1), row("Same", None, 0.1)];
        twins.sort_by_key(|row| row.work.id);
        let [first_twin, second_twin] = twins;
        let rows = [second_twin, undated, older, first_twin, newer, alpha, high];
        let ranked = rank(&rows, None, &[], None);
        let by_title = |title: &str, year: Option<i32>| {
            rows.iter()
                .find(|row| {
                    row.work.display_title.as_deref() == Some(title)
                        && row.work.display_year == year
                })
                .map(|row| row.work.id)
                .expect("row")
        };
        assert_eq!(ranked.visible[0], by_title("Zeta", Some(1990)));
        assert_eq!(ranked.visible[1], by_title("Zeta", Some(2010)));
        assert_eq!(ranked.visible[2], by_title("Zeta", Some(2000)));
        // Same score and nulls last for the year, then title key.
        assert_eq!(ranked.visible[3], by_title("Alpha", None));
        assert_eq!(ranked.visible[4], by_title("Zeta", None));
        let mut twin_ids: Vec<WorkId> = rows
            .iter()
            .filter(|row| row.work.display_title.as_deref() == Some("Same"))
            .map(|row| row.work.id)
            .collect();
        twin_ids.sort();
        assert_eq!(ranked.visible[5..], twin_ids[..]);
    }

    #[test]
    fn should_return_every_unmarked_neighbor_when_limit_absent() {
        let rows: Vec<Row> = (0..25).map(|n| row(&format!("T{n}"), None, 0.5)).collect();
        let ranked = rank(&rows, None, &[], None);
        assert_eq!(ranked.visible.len(), 25);
        assert!(ranked.clusters.is_empty());
        assert_eq!(ranked.total, 25);
    }

    #[test]
    fn should_keep_all_pinned_visible_when_pinned_exceed_limit() {
        let mut rows: Vec<Row> = (0..5)
            .map(|n| marked(row(&format!("P{n}"), None, 0.0), WorkMark::Pinned))
            .collect();
        rows.extend((0..5).map(|n| row(&format!("U{n}"), None, 1.0)));
        let ranked = rank(&rows, Some(3), &[], None);
        assert_eq!(ranked.visible.len(), 5);
        assert!(
            ranked
                .visible
                .iter()
                .all(|id| rows.iter().any(|r| r.work.id == *id && r.mark.is_some()))
        );
        assert_eq!(ranked.pinned, 5);
        assert_eq!(ranked.clusters.iter().map(|c| c.count).sum::<usize>(), 5);
    }

    #[test]
    fn should_count_pinned_toward_limit_when_ranking() {
        let mut rows = vec![
            marked(row("Pinned one", None, 0.0), WorkMark::Pinned),
            marked(row("Pinned two", None, 0.0), WorkMark::Pinned),
        ];
        rows.extend((0..6).map(|n| row(&format!("U{n}"), None, 0.5 + f64::from(n) / 100.0)));
        let ranked = rank(&rows, Some(4), &[], None);
        assert_eq!(ranked.visible.len(), 4);
        assert_eq!(ranked.visible[..2], ids(&[&rows[0], &rows[1]])[..]);
        // The two best unmarked follow the pinned.
        assert_eq!(ranked.visible[2], rows[7].work.id);
        assert_eq!(ranked.visible[3], rows[6].work.id);
    }

    #[test]
    fn should_collapse_downplayed_even_when_score_is_highest() {
        let rows = [
            marked(row("Loud", None, 1.0), WorkMark::Downplayed),
            row("Quiet", None, 0.0),
        ];
        let ranked = rank(&rows, None, &[], None);
        assert_eq!(ranked.visible, ids(&[&rows[1]]));
        assert_eq!(cluster_keys(&ranked), ["downplayed"]);
        assert_eq!(ranked.clusters[0].work_ids, ids(&[&rows[0]]));
        assert_eq!(ranked.clusters[0].label, "Downplayed");
        assert_eq!(ranked.total, 2);
    }

    #[test]
    fn should_exclude_hidden_from_total_and_visible_when_ranking() {
        let rows = [
            marked(row("Gone", None, 1.0), WorkMark::Hidden),
            row("Here", None, 0.1),
        ];
        let ranked = rank(&rows, None, &[], None);
        assert_eq!(ranked.visible, ids(&[&rows[1]]));
        assert_eq!(ranked.total, 1);
        assert_eq!(ranked.hidden, 1);
        assert_eq!(cluster_keys(&ranked), ["hidden"]);
        assert_eq!(ranked.clusters[0].kind, ClusterKind::Hidden);
    }

    /// Small deterministic generator so the property test needs no extra crate.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
    }

    fn generated_rows(count: usize, seed: u64, tags: &[TagRef]) -> Vec<Row> {
        let mut rng = Lcg(seed);
        (0..count)
            .map(|n| {
                let year = match rng.next() % 6 {
                    0 => None,
                    1 => Some(-5),
                    2 => Some(99_999),
                    _ => Some(1950 + i32::try_from(rng.next() % 75).unwrap_or(0)),
                };
                let mut item = row(
                    &format!("Title {n}"),
                    year,
                    f64::from(u32::try_from(rng.next() % 100).unwrap_or(0)) / 100.0,
                );
                item.mark = match rng.next() % 8 {
                    0 => Some(WorkMark::Pinned),
                    1 => Some(WorkMark::Downplayed),
                    2 => Some(WorkMark::Hidden),
                    _ => None,
                };
                for tag in tags {
                    if rng.next().is_multiple_of(5) {
                        item.tags.push(tag.clone());
                    }
                }
                item
            })
            .collect()
    }

    #[test]
    fn should_assign_each_remaining_work_to_exactly_one_cluster() {
        let tags = [tag("Alpha"), tag("beta"), tag("Gamma")];
        for seed in 0..5 {
            let rows = generated_rows(300, seed, &tags);
            for limit in [Some(1), Some(40), Some(500), None] {
                let ranked = rank(&rows, limit, &[], None);
                let mut covered: Vec<WorkId> = ranked.visible.clone();
                for cluster in &ranked.clusters {
                    assert_eq!(cluster.count, cluster.work_ids.len());
                    assert!(cluster.count > 0);
                    covered.extend(&cluster.work_ids);
                }
                let unique: HashSet<WorkId> = covered.iter().copied().collect();
                assert_eq!(unique.len(), covered.len(), "overlap, seed {seed}");
                let expected: HashSet<WorkId> = rows.iter().map(|row| row.work.id).collect();
                assert_eq!(unique, expected, "gap, seed {seed}, limit {limit:?}");
                let hidden = rows
                    .iter()
                    .filter(|row| row.mark == Some(WorkMark::Hidden))
                    .count();
                assert_eq!(ranked.total, rows.len() - hidden);
                assert_eq!(ranked.hidden, hidden);
            }
        }
    }

    #[test]
    fn should_cluster_by_alphabetically_first_tag_before_decade() {
        let zebra = tag("Zebra");
        let apple = tag("apple");
        let rows = [
            row("Top", None, 1.0),
            tagged(row("Both", Some(1999), 0.1), &[&zebra, &apple]),
            tagged(row("Zebra only", Some(1999), 0.1), &[&zebra]),
            row("Plain", Some(1999), 0.1),
        ];
        let ranked = rank(&rows, Some(1), &[], None);
        assert_eq!(
            cluster_keys(&ranked),
            [
                format!("tag:{}", apple.id).as_str(),
                format!("tag:{}", zebra.id).as_str(),
                "decade:1990"
            ]
        );
        assert_eq!(ranked.clusters[0].label, "apple");
        assert_eq!(ranked.clusters[0].kind, ClusterKind::Tag);
        assert_eq!(ranked.clusters[0].work_ids, ids(&[&rows[1]]));
        assert_eq!(ranked.clusters[1].work_ids, ids(&[&rows[2]]));
    }

    #[test]
    fn should_label_decades_and_unknown_year_when_clustering() {
        let rows = [
            row("Top", None, 1.0),
            row("A", Some(1999), 0.5),
            row("B", Some(2000), 0.5),
            row("C", Some(-3), 0.5),
            row("D", Some(0), 0.5),
            row("E", Some(99_999), 0.5),
            row("F", None, 0.5),
        ];
        let ranked = rank(&rows, Some(1), &[], None);
        let labels: Vec<(&str, &str)> = ranked
            .clusters
            .iter()
            .map(|c| (c.key.as_str(), c.label.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("decade:2000", "2000s"),
                ("decade:1990", "1990s"),
                ("decade:unknown", "Year unknown"),
            ]
        );
        assert_eq!(ranked.clusters[2].count, 4);
        assert!(
            ranked
                .clusters
                .iter()
                .all(|c| c.kind == ClusterKind::Decade)
        );
    }

    #[test]
    fn should_order_clusters_tags_decades_unknown_downplayed_hidden() {
        let beta = tag("Beta");
        let alpha = tag("alpha");
        let rows = [
            row("Top", None, 1.0),
            marked(row("H", None, 0.5), WorkMark::Hidden),
            marked(row("D", None, 0.5), WorkMark::Downplayed),
            row("U", None, 0.5),
            row("Old", Some(1985), 0.5),
            row("New", Some(2015), 0.5),
            tagged(row("TB", None, 0.5), &[&beta]),
            tagged(row("TA", None, 0.5), &[&alpha]),
        ];
        let ranked = rank(&rows, Some(1), &[], None);
        assert_eq!(
            cluster_keys(&ranked),
            [
                format!("tag:{}", alpha.id).as_str(),
                format!("tag:{}", beta.id).as_str(),
                "decade:2010",
                "decade:1980",
                "decade:unknown",
                "downplayed",
                "hidden",
            ]
        );
    }

    #[test]
    fn should_append_expanded_cluster_members_after_top_n() {
        let rows: Vec<Row> = [
            ("Top", 1.0, 2015),
            ("A", 0.4, 2011),
            ("B", 0.3, 2012),
            ("C", 0.2, 1985),
        ]
        .iter()
        .map(|(title, score, year)| row(title, Some(*year), *score))
        .chain([marked(row("Down", None, 0.9), WorkMark::Downplayed)])
        .collect();
        let ranked = rank(&rows, Some(1), &["decade:2010", "downplayed"], None);
        assert_eq!(
            ranked.visible,
            ids(&[&rows[0], &rows[1], &rows[2], &rows[4]])
        );
        assert_eq!(cluster_keys(&ranked), ["decade:1980"]);
        assert_eq!(ranked.total, 5);
    }

    #[test]
    fn should_show_hidden_members_when_hidden_cluster_is_expanded() {
        let rows = [
            row("Shown", None, 0.5),
            marked(row("Gone", None, 1.0), WorkMark::Hidden),
        ];
        let ranked = rank(&rows, None, &["hidden"], None);
        assert_eq!(ranked.visible, ids(&[&rows[0], &rows[1]]));
        assert!(ranked.clusters.is_empty());
        assert_eq!(ranked.total, 1);
        assert_eq!(ranked.hidden, 1);
    }

    #[test]
    fn should_ignore_unknown_expand_keys() {
        let rows = [row("Top", None, 1.0), row("Next", Some(1999), 0.5)];
        let plain = rank(&rows, Some(1), &[], None);
        let stale = rank(
            &rows,
            Some(1),
            &["tag:tgone", "decade:3000", "", "nonsense"],
            None,
        );
        assert_eq!(plain, stale);
    }

    #[test]
    fn should_filter_by_any_selected_tag_before_ranking_and_drop_unmatched_pinned() {
        let methods = tag("methods");
        let theory = tag("theory");
        let rows = [
            marked(row("Pinned untagged", None, 1.0), WorkMark::Pinned),
            tagged(row("M", None, 0.2), &[&methods]),
            tagged(row("T", None, 0.9), &[&theory]),
            tagged(row("Other", None, 0.5), &[&tag("other")]),
            row("Plain", None, 0.7),
        ];
        let filter: HashSet<TagId> = [methods.id, theory.id].into_iter().collect();
        let ranked = rank(&rows, Some(1), &[], Some(&filter));
        assert_eq!(ranked.filtered_out, 3);
        assert_eq!(ranked.total, 2);
        assert_eq!(ranked.pinned, 0);
        assert_eq!(ranked.visible, ids(&[&rows[2]]));
        assert_eq!(ranked.clusters.iter().map(|c| c.count).sum::<usize>(), 1);
    }

    #[test]
    fn should_return_empty_visible_and_no_clusters_when_unknown_tag_filters_everything() {
        let rows = [row("A", None, 0.5), row("B", Some(2001), 0.5)];
        let filter: HashSet<TagId> = std::iter::once(TagId::new()).collect();
        let ranked = rank(&rows, Some(10), &[], Some(&filter));
        assert!(ranked.visible.is_empty());
        assert!(ranked.clusters.is_empty());
        assert_eq!(ranked.total, 0);
        assert_eq!(ranked.filtered_out, 2);
        let empty: HashSet<TagId> = HashSet::new();
        let ranked = rank(&rows, None, &[], Some(&empty));
        assert!(ranked.visible.is_empty());
    }

    #[test]
    fn should_never_rank_or_cluster_the_center_when_it_appears_as_neighbor() {
        let center = work(Some("Center"), Some(2000));
        let other = work(Some("Other"), Some(2000));
        let tags = [];
        let candidates = [
            NeighborCandidate {
                work: &center,
                score: 1.0,
                mark: Some(WorkMark::Hidden),
                tags: &tags,
            },
            NeighborCandidate {
                work: &other,
                score: 0.1,
                mark: None,
                tags: &tags,
            },
            NeighborCandidate {
                work: &other,
                score: 0.9,
                mark: None,
                tags: &tags,
            },
        ];
        let expand = HashSet::new();
        let ranked = rank_neighborhood(
            center.id,
            &candidates,
            &RankingRequest {
                limit: Some(5),
                expand: &expand,
                tag_filter: None,
            },
        );
        assert_eq!(ranked.visible, [other.id]);
        assert_eq!(ranked.total, 1);
        assert_eq!(ranked.hidden, 0);
        assert!(ranked.clusters.is_empty());
    }

    #[test]
    fn should_be_deterministic_when_input_order_is_shuffled() {
        let tags = [tag("Alpha"), tag("beta")];
        let mut rows = generated_rows(120, 7, &tags);
        // Force ties so the tie-breakers matter.
        for (n, row) in rows.iter_mut().enumerate() {
            row.score = f64::from(u32::try_from(n % 4).unwrap_or(0)) / 4.0;
            row.work.display_title = Some(format!("Same {}", n % 3));
        }
        let expected = rank(&rows, Some(30), &["downplayed"], None);
        let mut rng = Lcg(99);
        for _ in 0..10 {
            for index in (1..rows.len()).rev() {
                let other = usize::try_from(rng.next()).unwrap_or(0) % (index + 1);
                rows.swap(index, other);
            }
            assert_eq!(rank(&rows, Some(30), &["downplayed"], None), expected);
        }
    }

    #[test]
    fn should_limit_sample_titles_to_three_and_use_placeholder_for_untitled() {
        let rows = [
            row("Top", None, 1.0),
            row("First", None, 0.9),
            Row {
                work: work(None, None),
                score: 0.8,
                mark: None,
                tags: Vec::new(),
            },
            Row {
                work: work(Some("   "), None),
                score: 0.7,
                mark: None,
                tags: Vec::new(),
            },
            row("Fourth", None, 0.6),
            row("Fifth", None, 0.5),
        ];
        let ranked = rank(&rows, Some(1), &[], None);
        assert_eq!(ranked.clusters.len(), 1);
        assert_eq!(ranked.clusters[0].count, 5);
        assert_eq!(
            ranked.clusters[0].sample_titles,
            ["First", "Untitled", "Untitled"]
        );
    }

    #[test]
    fn should_serialize_cluster_kinds_in_snake_case() {
        let kinds = [
            ClusterKind::Tag,
            ClusterKind::Decade,
            ClusterKind::Downplayed,
            ClusterKind::Hidden,
        ];
        let text = serde_json::to_string(&kinds).expect("json");
        assert_eq!(text, r#"["tag","decade","downplayed","hidden"]"#);
    }
}
