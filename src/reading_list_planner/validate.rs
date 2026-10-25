//! Validation of the model's drafts: pure functions that keep only what the application
//! supplied or can check, and count every rejection in the report.

use std::collections::{HashMap, HashSet};

use lysilogy_core::{Claim, ContextRelation, EdgeKind, TopicLabel, WorkId};
use serde_json::{Value, json};

use super::report::{AssumedKnownReport, Report};
use super::{
    CLAIMS_PER_TOPIC_MAX, CONTEXT_LINK_NOTE_MAX_CHARS, CONTEXT_LINKS_PER_ITEM_MAX,
    ContextLinkDraft, FillDraft, GRAPH_WORKS_PER_TOPIC_MAX, PlanDraft, RATIONALE_MAX_CHARS, Role,
    TOPICS_MAX,
};

/// The Works the model was shown; an ID outside them is rejected.
#[derive(Clone, Debug, Default)]
pub struct Supplied {
    /// Anchors and graph context.
    pub context: HashSet<WorkId>,
    /// The knowledge profile that was put in the prompt.
    pub profile: HashSet<WorkId>,
}

/// A link to a Work the reader has read, checked against the profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedLink {
    pub work_id: WorkId,
    pub relation: ContextRelation,
    pub note: String,
}

/// The JSON stored in a node's `context_links`.
#[must_use]
pub fn links_json(links: &[CheckedLink]) -> Value {
    Value::Array(
        links
            .iter()
            .map(|link| {
                json!({
                    "work_id": link.work_id,
                    "relation": link.relation.as_str(),
                    "note": link.note,
                })
            })
            .collect(),
    )
}

/// A topic that survived validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedTopic {
    pub label: TopicLabel,
    pub summary: Option<String>,
}

/// A paper the model remembers, ready to verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedClaim {
    /// Index into [`CheckedPlan::topics`].
    pub topic: usize,
    pub claim: Claim,
    pub role: Role,
    pub rationale: String,
    pub links: Vec<CheckedLink>,
}

/// A Work of the supplied context the model picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedGraphWork {
    pub topic: usize,
    pub work_id: WorkId,
    pub role: Role,
    pub rationale: String,
    pub links: Vec<CheckedLink>,
}

/// An edge between two kept topics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckedTopicEdge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
}

/// A topic the reader is taken to know, with the profile Works that say so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedAssumedKnown {
    pub topic: String,
    pub reason: String,
    pub evidence: Vec<WorkId>,
}

/// The plan after validation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckedPlan {
    pub topics: Vec<CheckedTopic>,
    pub claims: Vec<CheckedClaim>,
    pub graph_works: Vec<CheckedGraphWork>,
    pub edges: Vec<CheckedTopicEdge>,
    pub assumed_known: Vec<CheckedAssumedKnown>,
}

fn truncated(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}

fn non_empty(text: Option<&String>) -> Option<String> {
    text.map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Keep the links whose target is in the supplied profile, with a known relation, once
/// each, up to the limit; count the rest as rejected.
fn check_links(
    drafts: &[ContextLinkDraft],
    profile: &HashSet<WorkId>,
    report: &mut Report,
) -> Vec<CheckedLink> {
    let mut kept: Vec<CheckedLink> = Vec::new();
    for draft in drafts {
        let work = draft.work_id.trim().parse::<WorkId>().ok();
        let relation = draft.relation.trim().parse::<ContextRelation>().ok();
        let usable = match (work, relation) {
            (Some(work), Some(relation))
                if profile.contains(&work)
                    && kept.len() < CONTEXT_LINKS_PER_ITEM_MAX
                    && kept.iter().all(|link| link.work_id != work) =>
            {
                Some((work, relation))
            }
            _ => None,
        };
        let Some((work_id, relation)) = usable else {
            report.context_links.rejected += 1;
            continue;
        };
        report.context_links.accepted += 1;
        kept.push(CheckedLink {
            work_id,
            relation,
            note: truncated(&draft.note, CONTEXT_LINK_NOTE_MAX_CHARS),
        });
    }
    kept
}

fn lower(label: &TopicLabel) -> String {
    label.as_str().to_lowercase()
}

/// Validate the plan pass's answer. Every claim and graph Work the plan names is counted
/// in `report.candidates.seen` and ends in exactly one bucket.
#[allow(clippy::too_many_lines)] // one pass over the draft, read top to bottom
pub fn check_plan(draft: PlanDraft, supplied: &Supplied, report: &mut Report) -> CheckedPlan {
    let mut plan = CheckedPlan::default();
    let mut topic_index: HashMap<String, usize> = HashMap::new();
    for (position, topic) in draft.topics.into_iter().enumerate() {
        report.topics.planned += 1;
        let named = topic.claims.len() + topic.graph_works.len();
        report.candidates.seen += named;
        let label = TopicLabel::parse(&topic.label).ok();
        let kept = if position >= TOPICS_MAX {
            report.topics.dropped_over_limit += 1;
            None
        } else if let Some(label) = label {
            if topic_index.contains_key(&lower(&label)) {
                report.topics.dropped_duplicate += 1;
                None
            } else {
                Some(label)
            }
        } else {
            report.topics.dropped_invalid += 1;
            None
        };
        let Some(label) = kept else {
            report.candidates.topic_dropped += named;
            continue;
        };
        let index = plan.topics.len();
        topic_index.insert(lower(&label), index);
        plan.topics.push(CheckedTopic {
            label,
            summary: non_empty(Some(&topic.summary)),
        });
        for (position, claim) in topic.claims.into_iter().enumerate() {
            let title = claim.title.trim();
            let role = claim.role.trim().parse::<Role>().ok();
            if position >= CLAIMS_PER_TOPIC_MAX {
                report.candidates.over_limit += 1;
                continue;
            }
            let Some(role) = role.filter(|_| !title.is_empty()) else {
                report.candidates.invalid += 1;
                continue;
            };
            let links = check_links(&claim.context_links, &supplied.profile, report);
            plan.claims.push(CheckedClaim {
                topic: index,
                claim: Claim {
                    title: title.to_owned(),
                    authors: claim
                        .authors
                        .iter()
                        .map(|author| author.trim().to_owned())
                        .filter(|author| !author.is_empty())
                        .collect(),
                    year: claim.year.filter(|year| (1000..=2200).contains(year)),
                    venue: non_empty(claim.venue.as_ref()),
                    doi: non_empty(claim.doi.as_ref()),
                    arxiv: non_empty(claim.arxiv.as_ref()),
                    raw: None,
                },
                role,
                rationale: truncated(&claim.rationale, RATIONALE_MAX_CHARS),
                links,
            });
        }
        for (position, picked) in topic.graph_works.into_iter().enumerate() {
            if position >= GRAPH_WORKS_PER_TOPIC_MAX {
                report.candidates.over_limit += 1;
                continue;
            }
            let work = picked.work_id.trim().parse::<WorkId>().ok();
            let known = work
                .filter(|work| supplied.context.contains(work) || supplied.profile.contains(work));
            let Some(work_id) = known else {
                report.candidates.rejected_ids += 1;
                continue;
            };
            let Ok(role) = picked.role.trim().parse::<Role>() else {
                report.candidates.invalid += 1;
                continue;
            };
            let links = check_links(&picked.context_links, &supplied.profile, report);
            plan.graph_works.push(CheckedGraphWork {
                topic: index,
                work_id,
                role,
                rationale: truncated(&picked.rationale, RATIONALE_MAX_CHARS),
                links,
            });
        }
    }
    for edge in draft.topic_edges {
        report.edges.planned += 1;
        let kind = match edge.kind.trim() {
            "subtopic" => Some(EdgeKind::Subtopic),
            "prerequisite" => Some(EdgeKind::Prerequisite),
            _ => None,
        };
        let from = topic_index.get(&edge.from.trim().to_lowercase()).copied();
        let to = topic_index.get(&edge.to.trim().to_lowercase()).copied();
        let (Some(from), Some(to)) = (from, to) else {
            report.edges.dropped_unknown += 1;
            continue;
        };
        let Some(kind) = kind.filter(|_| from != to) else {
            report.edges.dropped_invalid += 1;
            continue;
        };
        plan.edges.push(CheckedTopicEdge { from, to, kind });
    }
    for known in draft.assumed_known {
        let topic = known.topic.trim().to_owned();
        let mut evidence: Vec<WorkId> = Vec::new();
        for text in &known.evidence {
            match text.trim().parse::<WorkId>() {
                Ok(work) if supplied.profile.contains(&work) => {
                    if !evidence.contains(&work) {
                        evidence.push(work);
                    }
                }
                _ => report.assumed_known_evidence_rejected += 1,
            }
        }
        if topic.is_empty() || evidence.is_empty() {
            report.assumed_known_dropped += 1;
            continue;
        }
        plan.assumed_known.push(CheckedAssumedKnown {
            topic,
            reason: truncated(&known.reason, RATIONALE_MAX_CHARS),
            evidence,
        });
    }
    plan
}

impl CheckedAssumedKnown {
    /// The report entry, with titles looked up in `titles`.
    #[must_use]
    pub fn to_report(&self, titles: &HashMap<WorkId, String>) -> AssumedKnownReport {
        AssumedKnownReport {
            topic: self.topic.clone(),
            reason: self.reason.clone(),
            evidence: self
                .evidence
                .iter()
                .map(|work| super::report::EvidenceWork {
                    work_id: *work,
                    title: titles.get(work).cloned(),
                })
                .collect(),
        }
    }
}

/// A Work the fill pass picked from the offered ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedPick {
    pub work_id: WorkId,
    /// Index into the topics the pass was told about; `None` when the label matched none.
    pub topic: Option<usize>,
    pub rationale: String,
    pub links: Vec<CheckedLink>,
}

/// The fill pass after validation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckedFill {
    pub picks: Vec<CheckedPick>,
    pub prerequisites: Vec<(WorkId, WorkId)>,
}

/// What the fill pass's answer is checked against.
pub struct FillBounds<'a> {
    /// The only Works it may pick.
    pub offered: &'a HashSet<WorkId>,
    /// Works already on the list; prerequisite edges may name them.
    pub members: &'a HashSet<WorkId>,
    /// Topic labels, in the order the pass was told them.
    pub topics: &'a [String],
    pub profile: &'a HashSet<WorkId>,
    /// Most picks to keep.
    pub remaining: usize,
}

/// Validate the fill pass's answer against the closed world it was offered.
pub fn check_fill(draft: FillDraft, bounds: &FillBounds<'_>, report: &mut Report) -> CheckedFill {
    let mut fill = CheckedFill::default();
    let labels: HashMap<String, usize> = bounds
        .topics
        .iter()
        .enumerate()
        .map(|(index, label)| (label.to_lowercase(), index))
        .collect();
    let mut picked: HashSet<WorkId> = HashSet::new();
    for pick in draft.picks {
        report.fill.picks_seen += 1;
        let work = pick.work_id.trim().parse::<WorkId>().ok();
        let Some(work_id) = work.filter(|work| bounds.offered.contains(work)) else {
            report.fill.rejected_ids += 1;
            continue;
        };
        if picked.contains(&work_id) {
            report.fill.duplicate += 1;
            continue;
        }
        if picked.len() >= bounds.remaining {
            report.fill.over_size += 1;
            continue;
        }
        picked.insert(work_id);
        let topic = labels.get(&pick.topic.trim().to_lowercase()).copied();
        if topic.is_none() {
            report.fill.unassigned_topic += 1;
        }
        let links = check_links(&pick.context_links, bounds.profile, report);
        report.fill.admitted += 1;
        fill.picks.push(CheckedPick {
            work_id,
            topic,
            rationale: truncated(&pick.rationale, RATIONALE_MAX_CHARS),
            links,
        });
    }
    for edge in draft.prerequisites {
        report.edges.planned += 1;
        let from = edge.from.trim().parse::<WorkId>().ok();
        let to = edge.to.trim().parse::<WorkId>().ok();
        let known = |work: Option<WorkId>| {
            work.filter(|work| picked.contains(work) || bounds.members.contains(work))
        };
        let (Some(from), Some(to)) = (known(from), known(to)) else {
            report.edges.dropped_unknown += 1;
            continue;
        };
        if from == to || fill.prerequisites.contains(&(from, to)) {
            report.edges.dropped_invalid += 1;
            continue;
        }
        fill.prerequisites.push((from, to));
    }
    fill
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading_list_planner::{
        AssumedKnownDraft, ClaimDraft, FillPickDraft, GraphWorkDraft, PrerequisiteDraft,
        TopicDraft, TopicEdgeDraft,
    };

    fn claim(title: &str) -> ClaimDraft {
        ClaimDraft {
            title: title.to_owned(),
            role: "core".to_owned(),
            ..ClaimDraft::default()
        }
    }

    fn topic(label: &str, claims: Vec<ClaimDraft>) -> TopicDraft {
        TopicDraft {
            label: label.to_owned(),
            summary: "About it.".to_owned(),
            claims,
            graph_works: Vec::new(),
        }
    }

    fn link(work: WorkId, relation: &str) -> ContextLinkDraft {
        ContextLinkDraft {
            work_id: work.to_string(),
            relation: relation.to_owned(),
            note: "Because.".to_owned(),
        }
    }

    fn supplied(context: &[WorkId], profile: &[WorkId]) -> Supplied {
        Supplied {
            context: context.iter().copied().collect(),
            profile: profile.iter().copied().collect(),
        }
    }

    fn edge(from: &str, to: &str, kind: &str) -> TopicEdgeDraft {
        TopicEdgeDraft {
            from: from.to_owned(),
            to: to.to_owned(),
            kind: kind.to_owned(),
        }
    }

    #[test]
    fn should_reject_a_graph_work_when_its_id_was_not_supplied() {
        let known = WorkId::new();
        let stranger = WorkId::new();
        let mut first = topic("Basics", Vec::new());
        for id in [
            known.to_string(),
            stranger.to_string(),
            "garbage".to_owned(),
        ] {
            first.graph_works.push(GraphWorkDraft {
                work_id: id,
                role: "core".to_owned(),
                ..GraphWorkDraft::default()
            });
        }
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![first],
                ..PlanDraft::default()
            },
            &supplied(&[known], &[]),
            &mut report,
        );
        assert_eq!(plan.graph_works.len(), 1);
        assert_eq!(plan.graph_works[0].work_id, known);
        assert_eq!(report.candidates.rejected_ids, 2);
        assert_eq!(report.candidates.seen, 3);
    }

    #[test]
    fn should_accept_a_graph_work_from_the_profile_when_the_context_lacks_it() {
        let read = WorkId::new();
        let mut first = topic("Basics", Vec::new());
        first.graph_works.push(GraphWorkDraft {
            work_id: read.to_string(),
            role: "survey".to_owned(),
            ..GraphWorkDraft::default()
        });
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![first],
                ..PlanDraft::default()
            },
            &supplied(&[], &[read]),
            &mut report,
        );
        assert_eq!(plan.graph_works.len(), 1);
    }

    #[test]
    fn should_reject_a_context_link_when_its_target_is_not_in_the_profile() {
        let read = WorkId::new();
        let outside = WorkId::new();
        let mut item = claim("A paper");
        item.context_links = vec![
            link(read, "builds_on"),
            link(outside, "extends"),
            link(read, "extends"),
            link(WorkId::new(), "nonsense"),
        ];
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![topic("Basics", vec![item])],
                ..PlanDraft::default()
            },
            &supplied(&[], &[read]),
            &mut report,
        );
        let links = &plan.claims[0].links;
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].work_id, read);
        assert_eq!(links[0].relation, ContextRelation::BuildsOn);
        assert_eq!(report.context_links.accepted, 1);
        assert_eq!(
            report.context_links.rejected, 3,
            "outside, repeat, bad relation"
        );
    }

    #[test]
    fn should_keep_three_links_and_cut_long_notes_when_checking_links() {
        let profile: Vec<WorkId> = (0..5).map(|_| WorkId::new()).collect();
        let mut item = claim("A paper");
        item.context_links = profile
            .iter()
            .map(|work| link(*work, "same_problem"))
            .collect();
        item.context_links[0].note = "é".repeat(500);
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![topic("Basics", vec![item])],
                ..PlanDraft::default()
            },
            &supplied(&[], &profile),
            &mut report,
        );
        assert_eq!(plan.claims[0].links.len(), 3);
        assert_eq!(plan.claims[0].links[0].note.chars().count(), 200);
        assert_eq!(report.context_links.rejected, 2);
    }

    #[test]
    fn should_count_every_claim_when_topics_and_claims_exceed_their_limits() {
        let mut topics: Vec<TopicDraft> = (0..14)
            .map(|index| {
                topic(
                    &format!("Topic {index}"),
                    vec![claim(&format!("Paper {index}"))],
                )
            })
            .collect();
        topics[0].claims = (0..10)
            .map(|index| claim(&format!("Many {index}")))
            .collect();
        topics.push(topic("topic 3", vec![claim("In a duplicate")]));
        topics.push(topic("   ", vec![claim("In an unnamed topic")]));
        topics[1].claims.push(claim("   "));
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics,
                ..PlanDraft::default()
            },
            &Supplied::default(),
            &mut report,
        );
        assert_eq!(plan.topics.len(), 12);
        assert_eq!(
            report.topics.dropped_over_limit, 4,
            "topics 12, 13, and the last two"
        );
        assert_eq!(
            report.topics.dropped_duplicate, 0,
            "the duplicate was already over the limit"
        );
        assert_eq!(report.candidates.over_limit, 2);
        assert_eq!(report.candidates.invalid, 1);
        assert_eq!(
            report.candidates.seen,
            plan.claims.len()
                + report.candidates.over_limit
                + report.candidates.invalid
                + report.candidates.topic_dropped,
            "every claim is accounted for"
        );
    }

    #[test]
    fn should_drop_duplicate_and_invalid_topics_when_labels_repeat_or_are_blank() {
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![
                    topic("Reward Models", vec![claim("One")]),
                    topic("  reward   models ", vec![claim("Two")]),
                    topic("", vec![claim("Three")]),
                    topic(&"x".repeat(81), vec![claim("Four")]),
                ],
                ..PlanDraft::default()
            },
            &Supplied::default(),
            &mut report,
        );
        assert_eq!(plan.topics.len(), 1);
        assert_eq!(report.topics.dropped_duplicate, 1);
        assert_eq!(report.topics.dropped_invalid, 2);
        assert_eq!(report.candidates.topic_dropped, 3);
        assert_eq!(report.candidates.seen, 4);
    }

    #[test]
    fn should_reject_a_claim_when_its_role_is_unknown_or_title_blank() {
        let mut odd = claim("Odd role");
        odd.role = "central".to_owned();
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![topic("T", vec![odd, claim("  "), claim("Fine")])],
                ..PlanDraft::default()
            },
            &Supplied::default(),
            &mut report,
        );
        assert_eq!(plan.claims.len(), 1);
        assert_eq!(report.candidates.invalid, 2);
    }

    #[test]
    fn should_normalize_claim_fields_when_the_model_pads_them() {
        let mut item = claim("  Attention Is All You Need  ");
        item.authors = vec![" Vaswani ".to_owned(), String::new()];
        item.year = Some(17);
        item.doi = Some("  ".to_owned());
        item.arxiv = Some(" 1706.03762 ".to_owned());
        item.rationale = "x".repeat(400);
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![topic("T", vec![item])],
                ..PlanDraft::default()
            },
            &Supplied::default(),
            &mut report,
        );
        let kept = &plan.claims[0];
        assert_eq!(kept.claim.title, "Attention Is All You Need");
        assert_eq!(kept.claim.authors, vec!["Vaswani".to_owned()]);
        assert_eq!(
            kept.claim.year, None,
            "an impossible year is dropped, not trusted"
        );
        assert_eq!(kept.claim.doi, None);
        assert_eq!(kept.claim.arxiv.as_deref(), Some("1706.03762"));
        assert_eq!(kept.rationale.chars().count(), 300);
    }

    #[test]
    fn should_drop_topic_edges_when_labels_are_unknown_or_kind_invalid() {
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![topic("A", Vec::new()), topic("B", Vec::new())],
                topic_edges: vec![
                    edge("a", "B", "prerequisite"),
                    edge("A", "Ghost", "subtopic"),
                    edge("A", "A", "subtopic"),
                    edge("A", "B", "covers"),
                    edge("B", "A", "subtopic"),
                ],
                ..PlanDraft::default()
            },
            &Supplied::default(),
            &mut report,
        );
        assert_eq!(plan.edges.len(), 2);
        assert_eq!(report.edges.planned, 5);
        assert_eq!(report.edges.dropped_unknown, 1);
        assert_eq!(report.edges.dropped_invalid, 2);
    }

    #[test]
    fn should_record_assumed_known_with_evidence_when_the_profile_backs_it() {
        let read = WorkId::new();
        let outside = WorkId::new();
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                assumed_known: vec![
                    AssumedKnownDraft {
                        topic: "Transformers".to_owned(),
                        reason: "Read the original paper.".to_owned(),
                        evidence: vec![read.to_string(), read.to_string(), outside.to_string()],
                    },
                    AssumedKnownDraft {
                        topic: "Optimization".to_owned(),
                        reason: "No evidence.".to_owned(),
                        evidence: vec![outside.to_string()],
                    },
                    AssumedKnownDraft {
                        topic: "  ".to_owned(),
                        reason: String::new(),
                        evidence: vec![read.to_string()],
                    },
                ],
                ..PlanDraft::default()
            },
            &supplied(&[], &[read]),
            &mut report,
        );
        assert_eq!(plan.assumed_known.len(), 1);
        assert_eq!(plan.assumed_known[0].evidence, vec![read]);
        assert_eq!(report.assumed_known_dropped, 2);
        assert_eq!(report.assumed_known_evidence_rejected, 2);
    }

    #[test]
    fn should_keep_unicode_labels_and_titles_when_checking_a_plan() {
        let mut report = Report::default();
        let plan = check_plan(
            PlanDraft {
                topics: vec![topic(
                    "Über-Modelle 強化学習",
                    vec![claim("Müller’s “Théorie”")],
                )],
                ..PlanDraft::default()
            },
            &Supplied::default(),
            &mut report,
        );
        assert_eq!(plan.topics[0].label.as_str(), "Über-Modelle 強化学習");
        assert_eq!(plan.claims[0].claim.title, "Müller’s “Théorie”");
    }

    fn bounds<'a>(
        offered: &'a HashSet<WorkId>,
        members: &'a HashSet<WorkId>,
        topics: &'a [String],
        profile: &'a HashSet<WorkId>,
        remaining: usize,
    ) -> FillBounds<'a> {
        FillBounds {
            offered,
            members,
            topics,
            profile,
            remaining,
        }
    }

    fn pick(work: &str, topic: &str) -> FillPickDraft {
        FillPickDraft {
            work_id: work.to_owned(),
            topic: topic.to_owned(),
            rationale: "Needed.".to_owned(),
            context_links: Vec::new(),
        }
    }

    #[test]
    fn should_reject_a_pick_when_it_was_not_offered() {
        let offered_work = WorkId::new();
        let offered: HashSet<WorkId> = [offered_work].into();
        let none = HashSet::new();
        let topics = vec!["Basics".to_owned()];
        let mut report = Report::default();
        let fill = check_fill(
            FillDraft {
                picks: vec![
                    pick(&offered_work.to_string(), "basics"),
                    pick(&WorkId::new().to_string(), "Basics"),
                    pick("not-an-id", "Basics"),
                    pick(&offered_work.to_string(), "Basics"),
                ],
                prerequisites: Vec::new(),
            },
            &bounds(&offered, &none, &topics, &none, 5),
            &mut report,
        );
        assert_eq!(fill.picks.len(), 1);
        assert_eq!(fill.picks[0].topic, Some(0));
        assert_eq!(report.fill.picks_seen, 4);
        assert_eq!(report.fill.rejected_ids, 2);
        assert_eq!(report.fill.duplicate, 1);
        assert_eq!(report.fill.admitted, 1);
    }

    #[test]
    fn should_stop_picking_when_the_remaining_size_is_reached() {
        let works: Vec<WorkId> = (0..4).map(|_| WorkId::new()).collect();
        let offered: HashSet<WorkId> = works.iter().copied().collect();
        let none = HashSet::new();
        let topics = vec!["Basics".to_owned()];
        let mut report = Report::default();
        let fill = check_fill(
            FillDraft {
                picks: works
                    .iter()
                    .map(|work| pick(&work.to_string(), "Elsewhere"))
                    .collect(),
                prerequisites: Vec::new(),
            },
            &bounds(&offered, &none, &topics, &none, 2),
            &mut report,
        );
        assert_eq!(fill.picks.len(), 2);
        assert_eq!(report.fill.over_size, 2);
        assert_eq!(report.fill.unassigned_topic, 2);
        assert!(fill.picks.iter().all(|pick| pick.topic.is_none()));
    }

    #[test]
    fn should_keep_only_prerequisites_among_known_works_when_checking_a_fill() {
        let member = WorkId::new();
        let picked = WorkId::new();
        let stranger = WorkId::new();
        let offered: HashSet<WorkId> = [picked].into();
        let members: HashSet<WorkId> = [member].into();
        let none = HashSet::new();
        let topics = Vec::new();
        let edge = |from: WorkId, to: WorkId| PrerequisiteDraft {
            from: from.to_string(),
            to: to.to_string(),
        };
        let mut report = Report::default();
        let fill = check_fill(
            FillDraft {
                picks: vec![pick(&picked.to_string(), "")],
                prerequisites: vec![
                    edge(member, picked),
                    edge(member, picked),
                    edge(picked, picked),
                    edge(picked, stranger),
                ],
            },
            &bounds(&offered, &members, &topics, &none, 3),
            &mut report,
        );
        assert_eq!(fill.prerequisites, vec![(member, picked)]);
        assert_eq!(report.edges.planned, 4);
        assert_eq!(report.edges.dropped_invalid, 2, "a repeat and a self edge");
        assert_eq!(report.edges.dropped_unknown, 1);
    }

    #[test]
    fn should_reject_fill_links_when_the_target_is_outside_the_profile() {
        let picked = WorkId::new();
        let read = WorkId::new();
        let offered: HashSet<WorkId> = [picked].into();
        let profile: HashSet<WorkId> = [read].into();
        let none = HashSet::new();
        let topics = Vec::new();
        let mut item = pick(&picked.to_string(), "");
        item.context_links = vec![link(read, "contrasts"), link(picked, "extends")];
        let mut report = Report::default();
        let fill = check_fill(
            FillDraft {
                picks: vec![item],
                prerequisites: Vec::new(),
            },
            &bounds(&offered, &none, &topics, &profile, 3),
            &mut report,
        );
        assert_eq!(fill.picks[0].links.len(), 1);
        assert_eq!(report.context_links.rejected, 1);
    }

    #[test]
    fn should_serialize_links_with_relation_names_when_building_node_json() {
        let work = WorkId::new();
        let json = links_json(&[CheckedLink {
            work_id: work,
            relation: ContextRelation::SameMethod,
            note: "Same trick.".to_owned(),
        }]);
        assert_eq!(json[0]["relation"], "same_method");
        assert_eq!(json[0]["work_id"], work.to_string());
        assert_eq!(links_json(&[]), json!([]));
    }
}
