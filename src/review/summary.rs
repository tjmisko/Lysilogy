//! Aggregates over stored reviews, so two prompt or pipeline versions can be
//! compared dimension by dimension, and the offline export.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{GENERAL_COMPONENT, GradeValue, Review, Rubric, rubric::DimensionKind};

/// The group for reviews of analyses that recorded no pipeline version.
pub const UNVERSIONED: &str = "unversioned";

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub rubric_id: String,
    pub reviews: usize,
    /// One entry per pipeline version, oldest first by earliest review.
    pub groups: Vec<GroupSummary>,
    /// Issue counts across every version, by component then severity.
    pub issues_by_component: BTreeMap<String, BTreeMap<String, usize>>,
    /// Issue counts across every version, by tag.
    pub issues_by_tag: BTreeMap<String, usize>,
    /// Citation-label grades grouped by the labeler and prompt version that
    /// wrote the pinned labels, which differ from the analysis pipeline.
    pub label_versions: Vec<LabelVersionSummary>,
    /// Accept, reject and edit actions on labels in the graph, as grading
    /// data per path, model and prompt version. Empty without a graph.
    pub label_actions: Vec<LabelActionSummary>,
}

/// Citation-label verdicts for one label version: a core model and
/// pipeline version (`core:…`), an edge labeler and prompt version
/// (`…@v3+digest`), or `user`.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LabelVersionSummary {
    pub label_version: String,
    /// Reviews that graded at least one label of this version.
    pub reviews: usize,
    /// Labels given a verdict.
    pub labels: usize,
    /// Verdict counts by option id (`ok`, `wrong_relation`, …).
    pub verdicts: BTreeMap<String, usize>,
    /// Mean of the component's score dimensions over the reviews whose
    /// run was written by this edge-labeler version (a review-level score,
    /// so it is filed under the run's labeler, not under core versions).
    pub score_mean: Option<f64>,
}

/// Review actions on AI labels in the graph for one path, model and prompt version.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LabelActionSummary {
    pub path: String,
    pub model: Option<String>,
    pub prompt_version: Option<i32>,
    pub accepted: i64,
    pub rejected: i64,
    pub edited: i64,
    pub unreviewed: i64,
    pub replaced: i64,
    /// Accepted over accepted, rejected and edited: how often a reviewed
    /// label was right as written.
    pub accept_rate: Option<f64>,
}

impl From<lysilogy_db::relationship::LabelReviewCounts> for LabelActionSummary {
    fn from(row: lysilogy_db::relationship::LabelReviewCounts) -> Self {
        let reviewed = row.accepted + row.rejected + row.edited;
        let to_f64 = |value: i64| f64::from(i32::try_from(value).unwrap_or(i32::MAX));
        Self {
            accept_rate: (reviewed > 0).then(|| to_f64(row.accepted) / to_f64(reviewed)),
            path: row.path,
            model: row.model,
            prompt_version: row.prompt_version,
            accepted: row.accepted,
            rejected: row.rejected,
            edited: row.edited,
            unreviewed: row.unreviewed,
            replaced: row.replaced,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GroupSummary {
    pub pipeline_version: String,
    pub reviews: usize,
    pub papers: usize,
    pub overall: Stat,
    pub components: BTreeMap<String, ComponentSummary>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Stat {
    pub n: usize,
    pub mean: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ComponentSummary {
    /// Reviews that recorded anything for this component.
    pub graded: usize,
    pub not_produced: usize,
    pub hard_rejects: usize,
    /// Mean of every score dimension in this component, pooled.
    pub score_mean: Option<f64>,
    pub dimensions: BTreeMap<String, DimensionSummary>,
    /// Issue counts by severity.
    pub issues: BTreeMap<String, usize>,
    /// Items given a per-item verdict (e.g. citation labels checked).
    pub items: usize,
    /// Per-item verdict counts by option id.
    pub verdicts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DimensionSummary {
    Score {
        n: usize,
        mean: Option<f64>,
    },
    PassFail {
        n: usize,
        passed: usize,
        pass_rate: Option<f64>,
    },
    Choice {
        n: usize,
        counts: BTreeMap<String, usize>,
    },
    Count {
        n: usize,
        total: u64,
        mean: Option<f64>,
    },
}

/// Summarises reviews per pipeline version and per component.
#[must_use]
pub fn summarize(rubric: &Rubric, reviews: &[Review]) -> Summary {
    let mut versions: Vec<(&str, Vec<&Review>)> = Vec::new();
    let mut ordered = reviews.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|review| review.created_at);
    for review in ordered {
        let version = review.pipeline_version();
        match versions.iter_mut().find(|(name, _)| *name == version) {
            Some((_, group)) => group.push(review),
            None => versions.push((version, vec![review])),
        }
    }
    let mut issues_by_component: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut issues_by_tag = BTreeMap::new();
    for issue in reviews.iter().flat_map(|review| &review.issues) {
        *issues_by_component
            .entry(issue.component.clone())
            .or_default()
            .entry(issue.severity.clone())
            .or_default() += 1;
        if let Some(tag) = &issue.tag {
            *issues_by_tag.entry(tag.clone()).or_default() += 1;
        }
    }
    Summary {
        rubric_id: rubric.id.clone(),
        reviews: reviews.len(),
        groups: versions
            .into_iter()
            .map(|(version, group)| summarize_group(rubric, version, &group))
            .collect(),
        issues_by_component,
        issues_by_tag,
        label_versions: summarize_label_versions(rubric, reviews),
        label_actions: Vec::new(),
    }
}

/// The edge-label verdicts grouped by the version of each graded label, read
/// from the sample its review pinned, and the review-level score grouped by
/// the run's edge-labeler version.
fn summarize_label_versions(rubric: &Rubric, reviews: &[Review]) -> Vec<LabelVersionSummary> {
    let Some(component) = rubric.component(EDGE_LABELS_COMPONENT) else {
        return Vec::new();
    };
    let mut groups: BTreeMap<String, LabelVersionSummary> = BTreeMap::new();
    let mut scores: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for review in reviews {
        let Some(grade) = review
            .components
            .get(EDGE_LABELS_COMPONENT)
            .filter(|grade| grade.is_graded())
        else {
            continue;
        };
        let pinned = review.snapshot.field(super::EDGE_LABELS_FIELD);
        let run_version = pinned
            .and_then(|pinned| pinned.get("label_version"))
            .and_then(Value::as_str)
            .unwrap_or(UNVERSIONED)
            .to_owned();
        let item_version = |claim_id: &str| -> String {
            pinned
                .and_then(|pinned| pinned.get("sample"))
                .and_then(Value::as_array)
                .and_then(|sample| {
                    sample.iter().find(|edge| {
                        edge.pointer("/label/claim_id").and_then(Value::as_str) == Some(claim_id)
                    })
                })
                .and_then(|edge| edge.get("label_version"))
                .and_then(Value::as_str)
                .map_or_else(|| run_version.clone(), str::to_owned)
        };
        let mut seen = BTreeSet::new();
        for (key, value) in &grade.values {
            let (Some(claim_id), GradeValue::Choice(verdict)) = (component.item_id(key), value)
            else {
                continue;
            };
            let version = item_version(claim_id);
            let group = groups.entry(version.clone()).or_insert_with(|| LabelVersionSummary {
                label_version: version.clone(),
                ..LabelVersionSummary::default()
            });
            group.labels += 1;
            *group.verdicts.entry(verdict.clone()).or_default() += 1;
            if seen.insert(version) {
                group.reviews += 1;
            }
        }
        let review_scores = component
            .dimensions
            .iter()
            .filter(|dimension| dimension.kind == DimensionKind::Score)
            .filter_map(|dimension| match grade.values.get(&dimension.id) {
                Some(GradeValue::Number(score)) => Some(*score),
                _ => None,
            });
        scores.entry(run_version).or_default().extend(review_scores);
    }
    for (version, values) in scores {
        if values.is_empty() {
            continue;
        }
        let group = groups.entry(version.clone()).or_insert_with(|| LabelVersionSummary {
            label_version: version,
            ..LabelVersionSummary::default()
        });
        group.score_mean = stat(values.into_iter()).mean;
    }
    groups.into_values().collect()
}

/// The rubric component that grades citation labels.
pub const EDGE_LABELS_COMPONENT: &str = "edge_labels";

fn summarize_group(rubric: &Rubric, version: &str, reviews: &[&Review]) -> GroupSummary {
    let papers = reviews
        .iter()
        .map(|review| review.paper_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let overall = stat(reviews.iter().filter_map(|review| review.overall));
    let components = rubric
        .components
        .iter()
        .map(|component| (component.id.clone(), summarize_component(component, reviews)))
        .collect();
    GroupSummary {
        pipeline_version: version.to_owned(),
        reviews: reviews.len(),
        papers,
        overall,
        components,
    }
}

fn summarize_component(component: &super::RubricComponent, reviews: &[&Review]) -> ComponentSummary {
    let grades = reviews
        .iter()
        .filter_map(|review| review.components.get(&component.id))
        .filter(|grade| grade.is_graded())
        .collect::<Vec<_>>();
    let dimensions = component
        .dimensions
        .iter()
        .map(|dimension| {
            let values = grades
                .iter()
                .filter_map(|grade| grade.values.get(&dimension.id))
                .collect::<Vec<_>>();
            (
                dimension.id.clone(),
                summarize_dimension(dimension.kind, &values),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let pooled = stat(
        component
            .dimensions
            .iter()
            .filter(|dimension| dimension.kind == DimensionKind::Score)
            .flat_map(|dimension| {
                grades
                    .iter()
                    .filter_map(|grade| match grade.values.get(&dimension.id) {
                        Some(GradeValue::Number(score)) => Some(*score),
                        _ => None,
                    })
            }),
    );
    let mut issues = BTreeMap::new();
    for issue in reviews
        .iter()
        .flat_map(|review| &review.issues)
        .filter(|issue| issue.component == component.id)
    {
        *issues.entry(issue.severity.clone()).or_default() += 1;
    }
    let mut items = 0;
    let mut verdicts = BTreeMap::new();
    for grade in &grades {
        for (key, value) in &grade.values {
            if let (Some(_), GradeValue::Choice(verdict)) = (component.item_id(key), value) {
                items += 1;
                *verdicts.entry(verdict.clone()).or_default() += 1;
            }
        }
    }
    ComponentSummary {
        items,
        verdicts,
        graded: grades.len(),
        not_produced: grades.iter().filter(|grade| grade.not_produced).count(),
        hard_rejects: grades.iter().filter(|grade| grade.hard_reject).count(),
        score_mean: pooled.mean,
        dimensions,
        issues,
    }
}

fn summarize_dimension(kind: DimensionKind, values: &[&GradeValue]) -> DimensionSummary {
    let numbers = || {
        values.iter().filter_map(|value| match value {
            GradeValue::Number(number) => Some(*number),
            _ => None,
        })
    };
    match kind {
        DimensionKind::Score => {
            let stat = stat(numbers());
            DimensionSummary::Score {
                n: stat.n,
                mean: stat.mean,
            }
        }
        DimensionKind::Count => {
            let stat = stat(numbers());
            DimensionSummary::Count {
                n: stat.n,
                total: numbers().map(u64::from).sum(),
                mean: stat.mean,
            }
        }
        DimensionKind::PassFail => {
            let flags = values
                .iter()
                .filter_map(|value| match value {
                    GradeValue::Flag(flag) => Some(*flag),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let passed = flags.iter().filter(|flag| **flag).count();
            DimensionSummary::PassFail {
                n: flags.len(),
                passed,
                pass_rate: ratio(passed, flags.len()),
            }
        }
        DimensionKind::Choice => {
            let mut counts = BTreeMap::new();
            for value in values {
                if let GradeValue::Choice(choice) = value {
                    *counts.entry(choice.clone()).or_default() += 1;
                }
            }
            DimensionSummary::Choice {
                n: counts.values().sum(),
                counts,
            }
        }
    }
}

fn stat(values: impl Iterator<Item = u32>) -> Stat {
    let (n, sum) = values.fold((0_usize, 0_u64), |(n, sum), value| {
        (n + 1, sum + u64::from(value))
    });
    Stat {
        n,
        mean: (n > 0).then(|| to_f64(sum) / to_f64(n as u64)),
    }
}

fn ratio(part: usize, whole: usize) -> Option<f64> {
    (whole > 0).then(|| to_f64(part as u64) / to_f64(whole as u64))
}

fn to_f64(value: u64) -> f64 {
    // Review counts stay far below 2^32; saturate rather than lose precision.
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

/// A plain-text table of the summary for the terminal.
#[must_use]
pub fn render_summary(rubric: &Rubric, summary: &Summary) -> String {
    let mut text = format!(
        "{} reviews against rubric {}\n",
        summary.reviews, summary.rubric_id
    );
    let format_mean =
        |mean: Option<f64>| mean.map_or_else(|| "-".to_owned(), |m| format!("{m:.2}"));
    for group in &summary.groups {
        let _ = write!(
            text,
            "\n== pipeline {} ({} reviews, {} papers) overall {} (n={})\n",
            group.pipeline_version,
            group.reviews,
            group.papers,
            format_mean(group.overall.mean),
            group.overall.n
        );
        for component in &rubric.components {
            let Some(found) = group.components.get(&component.id) else {
                continue;
            };
            let _ = writeln!(
                text,
                "  {:<16} graded {:>2}  not produced {:>2}  hard rejects {:>2}  mean {}",
                component.label,
                found.graded,
                found.not_produced,
                found.hard_rejects,
                format_mean(found.score_mean)
            );
            for dimension in &component.dimensions {
                let Some(dimension_summary) = found.dimensions.get(&dimension.id) else {
                    continue;
                };
                let value = match dimension_summary {
                    DimensionSummary::Score { n, mean } => {
                        format!("{} (n={n})", format_mean(*mean))
                    }
                    DimensionSummary::Count { n, total, mean } => {
                        format!("total {total}, mean {} (n={n})", format_mean(*mean))
                    }
                    DimensionSummary::PassFail {
                        n,
                        passed,
                        pass_rate,
                    } => format!(
                        "{passed}/{n} pass ({})",
                        pass_rate
                            .map_or_else(|| "-".to_owned(), |rate| format!("{:.0}%", rate * 100.0))
                    ),
                    DimensionSummary::Choice { counts, .. } => counts
                        .iter()
                        .map(|(choice, count)| format!("{choice} {count}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                let _ = writeln!(text, "      {:<28} {value}", dimension.label);
            }
        }
    }
    render_labels(rubric, summary, &mut text);
    if !summary.issues_by_component.is_empty() {
        text.push_str("\nFlagged issues\n");
        for (component, severities) in &summary.issues_by_component {
            let counts = severities
                .iter()
                .map(|(severity, count)| format!("{severity} {count}"))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(text, "  {component:<16} {counts}");
        }
    }
    text
}

/// The citation-label sections of the summary: grades by label version and
/// the review actions recorded in the graph.
fn render_labels(rubric: &Rubric, summary: &Summary, text: &mut String) {
    let format_mean =
        |mean: Option<f64>| mean.map_or_else(|| "-".to_owned(), |m| format!("{m:.2}"));
    let verdict_label = |id: &str| {
        rubric
            .component(EDGE_LABELS_COMPONENT)
            .and_then(|component| component.item_verdict.as_ref())
            .and_then(|verdict| verdict.options.iter().find(|option| option.id == id))
            .map_or_else(|| id.to_owned(), |option| option.label.clone())
    };
    for group in &summary.label_versions {
        let _ = writeln!(
            text,
            "\n== citation labels {} ({} reviews, {} labels) mean {}",
            group.label_version,
            group.reviews,
            group.labels,
            format_mean(group.score_mean)
        );
        for (verdict, count) in &group.verdicts {
            let _ = writeln!(text, "      {:<32} {count}", verdict_label(verdict));
        }
    }
    if !summary.label_actions.is_empty() {
        text.push_str("\nLabel review actions in the graph\n");
        for row in &summary.label_actions {
            let _ = writeln!(
                text,
                "  {:<14} {} v{}: accepted {}, rejected {}, edited {}, unreviewed {}, replaced {} (accept rate {})",
                row.path,
                row.model.as_deref().unwrap_or("-"),
                row.prompt_version.map_or_else(|| "-".to_owned(), |version| version.to_string()),
                row.accepted,
                row.rejected,
                row.edited,
                row.unreviewed,
                row.replaced,
                row.accept_rate
                    .map_or_else(|| "-".to_owned(), |rate| format!("{:.0}%", rate * 100.0))
            );
        }
    }
}

/// Everything needed to analyse reviews offline in one file.
#[derive(Clone, Debug, Serialize)]
pub struct ReviewExport {
    pub exported_at: DateTime<Utc>,
    pub rubric: Value,
    pub summary: Summary,
    pub reviews: Vec<Review>,
    /// Every flagged issue with the reviewed output it refers to, ready to be
    /// turned into a regression case.
    pub issues: Vec<ExportedIssue>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportedIssue {
    pub paper_id: String,
    pub paper_title: String,
    pub manifest_id: Option<String>,
    pub snapshot_id: String,
    pub pipeline_version: String,
    pub component: String,
    pub severity: String,
    pub tag: Option<String>,
    pub note: String,
    pub quote: Option<String>,
    /// The snapshot fields the component reads, as they were when flagged.
    pub reviewed_output: Map<String, Value>,
}

#[must_use]
pub fn export(
    rubric: &Rubric,
    rubric_json: Value,
    reviews: Vec<Review>,
    now: DateTime<Utc>,
) -> ReviewExport {
    let issues = reviews
        .iter()
        .flat_map(|review| {
            review.issues.iter().map(move |issue| {
                let reads = rubric
                    .component(&issue.component)
                    .map(|component| component.reads.clone())
                    .unwrap_or_default();
                let reviewed_output = review
                    .snapshot
                    .fields
                    .iter()
                    .filter(|(key, _)| issue.component == GENERAL_COMPONENT || reads.contains(key))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                ExportedIssue {
                    paper_id: review.paper_id.to_string(),
                    paper_title: review.paper_title.clone(),
                    manifest_id: review.manifest_id.clone(),
                    snapshot_id: review.snapshot.snapshot_id.clone(),
                    pipeline_version: review.pipeline_version().to_owned(),
                    component: issue.component.clone(),
                    severity: issue.severity.clone(),
                    tag: issue.tag.clone(),
                    note: issue.note.clone(),
                    quote: issue.quote.clone(),
                    reviewed_output,
                }
            })
        })
        .collect();
    ReviewExport {
        exported_at: now,
        rubric: rubric_json,
        summary: summarize(rubric, &reviews),
        reviews,
        issues,
    }
}
