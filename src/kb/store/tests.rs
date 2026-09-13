use super::*;
use crate::domain::PaperId;
use crate::kb::{
    AcquisitionState, Authorship, Citation, CitationEvidence, DecisionAction, EntityDecision,
    Observation, ObservationPayload, ObservationSource, WorkType,
};
use chrono::Utc;
use serde_json::json;

fn work(id: WorkId, title: &str) -> Work {
    Work {
        id,
        identifiers: Default::default(),
        title: Some(title.into()),
        title_key: Some(title.to_lowercase()),
        year: Some(2020),
        venue: None,
        kind: WorkType::Preprint,
        versions: vec![],
        local_copies: vec![],
        acquisition: AcquisitionState::Unresolved,
    }
}
fn fixture(store: &KbStore, id: &WorkId, name: &str) -> Admission {
    let paper_id = PaperId::from_relative_path(Path::new(name));
    let payload = json!({"title":name});
    let relative = format!("papers/{paper_id}/metadata.json");
    let bytes = serde_json::to_vec(&payload).unwrap();
    let path = store.data_root().join(&relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, &bytes).unwrap();
    Admission {
        observation: Observation {
            id: format!("obs-{name}"),
            source: ObservationSource::PdfMetadata { paper_id },
            retrieved_at: Utc::now(),
            payload: ObservationPayload::Json(payload),
        },
        entity: EntityProjection::Work(work(id.clone(), name)),
        source: ArtifactSource {
            relative_path: relative,
            sha256: digest(&bytes),
        },
        authorships: vec![],
        citations: vec![],
    }
}
fn evidence(admission: &Admission) -> CitationEvidence {
    let ObservationSource::PdfMetadata { paper_id } = &admission.observation.source else {
        unreachable!()
    };
    CitationEvidence::Local {
        paper_id: paper_id.clone(),
        bibliography_entry_id: "entry-1".into(),
        mentions: vec![],
    }
}
fn merge_decision(surviving: WorkId, absorbed: WorkId) -> Decision {
    Decision {
        id: "merge-1".into(),
        recorded_at: Utc::now(),
        rationale: "fixture exact identifiers".into(),
        action: DecisionAction::Work(EntityDecision::Merge {
            surviving,
            absorbed,
        }),
    }
}

#[test]
fn should_produce_identical_entities_when_rebuilding_from_scratch() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let a = store.allocate_work("local:a").unwrap();
    let b = store.allocate_work("local:b").unwrap();
    let admission = fixture(&store, &a, "Canonical title");
    store.admit(&admission, store.data_root()).unwrap();
    store
        .record_decision(&merge_decision(a.clone(), b.clone()))
        .unwrap();
    fs::write(
        directory.path().join("paper-identities.json"),
        b"fixture identity registry untouched",
    )
    .unwrap();
    fs::write(
        directory.path().join("kb/lists/reading.json"),
        br#"{"id":"reading","items":[]}"#,
    )
    .unwrap();
    store.rebuild().unwrap();
    let before = store.snapshot().unwrap();
    // Source updates/expiry cannot erase an already-admitted revision.
    fs::remove_file(directory.path().join(&admission.source.relative_path)).unwrap();
    store.rebuild().unwrap();
    assert_eq!(before, store.snapshot().unwrap());
    let database = store.database_path();
    drop(store);
    fs::remove_file(&database).unwrap();
    let rebuilt = KbStore::open(directory.path()).unwrap();
    rebuilt.rebuild().unwrap();
    assert_eq!(before, rebuilt.snapshot().unwrap());
    assert_eq!(rebuilt.allocate_work("local:a").unwrap(), a);
    assert_eq!(rebuilt.work(&b).unwrap().unwrap().id, a);
    assert_eq!(
        fs::read(directory.path().join("paper-identities.json")).unwrap(),
        b"fixture identity registry untouched"
    );
}

#[test]
fn should_apply_pending_migrations_when_opening_an_older_database() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("kb")).unwrap();
    let connection = Connection::open(directory.path().join("kb/kb.sqlite")).unwrap();
    connection.execute_batch(MIGRATIONS[0]).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);
    let store = KbStore::open(directory.path()).unwrap();
    let version: usize = store
        .connection()
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
    let wal: String = store
        .connection()
        .unwrap()
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(wal, "wal");
    let id = store.allocate_work("search").unwrap();
    let admission = fixture(&store, &id, "Trigram foundations");
    store.admit(&admission, store.data_root()).unwrap();
    assert_eq!(
        store.search_titles("gram found", 10).unwrap().ids,
        [id.to_string()]
    );
}

#[test]
fn should_return_a_two_hop_neighborhood_when_edges_exist_in_both_directions() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let ids = (0..6)
        .map(|index| store.allocate_work(&format!("node-{index}")).unwrap())
        .collect::<Vec<_>>();
    let mut admission = fixture(&store, &ids[0], "edges");
    for (citing, cited) in [(0, 1), (2, 0), (1, 3), (4, 2), (3, 5), (1, 0)] {
        admission.citations.push(Citation {
            citing: ids[citing].clone(),
            cited: ids[cited].clone(),
            evidence: vec![evidence(&admission)],
        });
    }
    store.admit(&admission, store.data_root()).unwrap();
    let graph = store
        .two_hop(&ids[0], NeighborhoodLimits::default())
        .unwrap();
    assert_eq!(graph.nodes.len(), 5);
    assert_eq!(graph.edges.len(), 5);
    assert!(!graph.nodes_truncated);
    assert!(!graph.edges_truncated);
    for index in 0..5 {
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == ids[index])
                .unwrap()
                .distance,
            if index == 0 {
                0
            } else if index < 3 {
                1
            } else {
                2
            }
        );
    }
    assert!(!graph.nodes.iter().any(|node| node.id == ids[5]));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.citing == ids[2] && edge.cited == ids[0])
    );
}

#[test]
fn should_report_explicit_truncation_when_neighborhood_limits_are_reached() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let ids = (0..4)
        .map(|index| store.allocate_work(&format!("cap-{index}")).unwrap())
        .collect::<Vec<_>>();
    let mut admission = fixture(&store, &ids[0], "caps");
    for id in &ids[1..] {
        admission.citations.push(Citation {
            citing: ids[0].clone(),
            cited: id.clone(),
            evidence: vec![evidence(&admission)],
        });
    }
    store.admit(&admission, store.data_root()).unwrap();
    let graph = store
        .two_hop(
            &ids[0],
            NeighborhoodLimits {
                max_nodes: 2,
                max_edges: 0,
            },
        )
        .unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph.edges.is_empty());
    assert!(graph.nodes_truncated);
    assert!(graph.edges_truncated);
}

#[test]
fn should_preserve_projection_when_canonical_rebuild_validation_fails() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    store.allocate_work("rollback").unwrap();
    let before = store.snapshot().unwrap();
    use std::io::Write as _;
    fs::OpenOptions::new()
        .append(true)
        .open(directory.path().join("kb/decisions.jsonl"))
        .unwrap()
        .write_all(b"{\"unfinished\":")
        .unwrap();
    assert!(store.rebuild().is_err());
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_reject_changed_origin_when_an_observation_id_is_reused() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_work("original").unwrap();
    let original = fixture(&store, &id, "original");
    store.admit(&original, store.data_root()).unwrap();
    let before = store.snapshot().unwrap();
    let mut changed = fixture(&store, &id, "replacement");
    changed.observation.id = original.observation.id;
    assert!(store.admit(&changed, store.data_root()).is_err());
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_preserve_observation_binding_when_payload_revisions_change() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_work("revision").unwrap();
    let original = fixture(&store, &id, "revision");
    store.admit(&original, store.data_root()).unwrap();
    let mut changed = original.clone();
    changed.entity = EntityProjection::Work(work(id.clone(), "Revised title"));
    changed.observation.payload = ObservationPayload::Json(json!({"title":"Revised title"}));
    let bytes = serde_json::to_vec(&json!({"title":"Revised title"})).unwrap();
    fs::write(directory.path().join(&changed.source.relative_path), &bytes).unwrap();
    changed.source.sha256 = digest(&bytes);
    store.admit(&changed, store.data_root()).unwrap();
    assert_eq!(store.summary().unwrap().observations, 1);
    assert_eq!(
        store.work(&id).unwrap().unwrap().title.as_deref(),
        Some("Revised title")
    );
    let before = store.snapshot().unwrap();
    store.rebuild().unwrap();
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_reject_admission_when_source_hash_or_payload_does_not_match() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_work("verify").unwrap();
    let mut admission = fixture(&store, &id, "verify");
    admission.source.sha256 = "0".repeat(64);
    assert!(store.admit(&admission, store.data_root()).is_err());
    let mut admission = fixture(&store, &id, "verify");
    admission.observation.payload = ObservationPayload::Text("wrong".into());
    assert!(store.admit(&admission, store.data_root()).is_err());
    assert_eq!(store.summary().unwrap().observations, 0);
}

#[test]
fn should_reject_symlinks_when_opening_database_or_canonical_sources() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), directory.path().join("kb")).unwrap();
    assert!(KbStore::open(directory.path()).is_err());
    fs::remove_file(directory.path().join("kb")).unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_work("symlink").unwrap();
    let admission = fixture(&store, &id, "symlink");
    let source = directory.path().join(&admission.source.relative_path);
    fs::rename(&source, outside.path().join("source.json")).unwrap();
    symlink(outside.path().join("source.json"), &source).unwrap();
    assert!(store.admit(&admission, store.data_root()).is_err());
}

#[test]
fn should_keep_distinct_constraints_when_an_alias_is_merged_again() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let a = store.allocate_work("a").unwrap();
    let b = store.allocate_work("b").unwrap();
    let c = store.allocate_work("c").unwrap();
    store
        .record_decision(&Decision {
            id: "distinct".into(),
            recorded_at: Utc::now(),
            rationale: "fixture different identities".into(),
            action: DecisionAction::Work(EntityDecision::Distinct {
                left: b.clone(),
                right: c.clone(),
            }),
        })
        .unwrap();
    store
        .record_decision(&merge_decision(a.clone(), b))
        .unwrap();
    let before = store.snapshot().unwrap();
    let mut rejected = merge_decision(a, c);
    rejected.id = "rejected".into();
    assert!(store.record_decision(&rejected).is_err());
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_retain_both_sources_when_a_merge_collapses_citation_edges() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let a = store.allocate_work("a").unwrap();
    let b = store.allocate_work("b").unwrap();
    let c = store.allocate_work("c").unwrap();
    for (id, title) in [(&a, "source-a"), (&b, "source-b")] {
        let mut admission = fixture(&store, id, title);
        admission.citations.push(Citation {
            citing: id.clone(),
            cited: c.clone(),
            evidence: vec![evidence(&admission)],
        });
        store.admit(&admission, store.data_root()).unwrap();
    }
    store.record_decision(&merge_decision(a, b)).unwrap();
    assert_eq!(store.summary().unwrap().citations, 1);
    assert_eq!(store.snapshot().unwrap()["citation_evidence"].len(), 2);
    let before = store.snapshot().unwrap();
    store.rebuild().unwrap();
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_reject_invalid_lists_when_rebuild_would_replace_the_previous_projection() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    store.allocate_work("keep").unwrap();
    let before = store.snapshot().unwrap();
    fs::write(
        directory.path().join("kb/lists/example.json"),
        br#"{"id":"wrong-id"}"#,
    )
    .unwrap();
    assert!(store.rebuild().is_err());
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_allocate_one_id_when_two_store_handles_share_an_origin() {
    let directory = tempfile::tempdir().unwrap();
    let a = KbStore::open(directory.path()).unwrap();
    let b = KbStore::open(directory.path()).unwrap();
    let first = std::thread::spawn(move || a.allocate_work("shared"));
    let second = std::thread::spawn(move || b.allocate_work("shared"));
    assert_eq!(
        first.join().unwrap().unwrap(),
        second.join().unwrap().unwrap()
    );
}

#[test]
fn should_index_names_when_an_admitted_person_is_rebuilt() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let work_id = store.allocate_work("author-work").unwrap();
    let person_id = store.allocate_person("author-person").unwrap();
    let mut admission = fixture(&store, &work_id, "author-observation");
    admission.entity = EntityProjection::Person(Person {
        id: person_id.clone(),
        display_name: "Müller Example".into(),
        family_name: Some("Example".into()),
        given_names: vec!["Müller".into()],
        particles: vec![],
        suffix: None,
        name_variants: vec![],
        identifiers: Default::default(),
    });
    admission.authorships.push(Authorship {
        work_id,
        person_id: person_id.clone(),
        position: 0,
        raw_name: "M. Example".into(),
    });
    store.admit(&admission, store.data_root()).unwrap();
    store.rebuild().unwrap();
    assert_eq!(
        store.search_names("Example", 10).unwrap().ids,
        [person_id.to_string()]
    );
}

#[test]
fn should_move_selected_observation_evidence_when_a_work_is_split() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let original = store.allocate_work("split-original").unwrap();
    let target = store.allocate_work("split-target").unwrap();
    let one = fixture(&store, &original, "First identity");
    store.admit(&one, store.data_root()).unwrap();
    let mut two = fixture(&store, &original, "Second identity");
    two.citations.push(Citation {
        citing: original.clone(),
        cited: target.clone(),
        evidence: vec![evidence(&two)],
    });
    store.admit(&two, store.data_root()).unwrap();
    let created: WorkId = "WsplitNew".parse().unwrap();
    store
        .record_decision(&Decision {
            id: "split-one".into(),
            recorded_at: Utc::now(),
            rationale: "separate source observations".into(),
            action: DecisionAction::Work(EntityDecision::Split {
                original: original.clone(),
                created: created.clone(),
                observation_ids: vec![two.observation.id.clone()],
            }),
        })
        .unwrap();
    assert_eq!(
        store.work(&original).unwrap().unwrap().title.as_deref(),
        Some("First identity")
    );
    assert_eq!(
        store.work(&created).unwrap().unwrap().title.as_deref(),
        Some("Second identity")
    );
    let graph = store
        .two_hop(&created, NeighborhoodLimits::default())
        .unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.citing == created && edge.cited == target)
    );
    // A producer may still hold the pre-split entity ID; the canonical binding wins.
    store.admit(&two, store.data_root()).unwrap();
    assert_eq!(
        store.work(&original).unwrap().unwrap().title.as_deref(),
        Some("First identity")
    );
    let before = store.snapshot().unwrap();
    store.rebuild().unwrap();
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_reject_unrelated_citation_provenance_when_admitting_a_source() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let a = store.allocate_work("a").unwrap();
    let b = store.allocate_work("b").unwrap();
    let mut one = fixture(&store, &a, "one");
    let two = fixture(&store, &b, "two");
    one.citations.push(Citation {
        citing: a,
        cited: b,
        evidence: vec![evidence(&two)],
    });
    assert!(store.admit(&one, store.data_root()).is_err());
    assert_eq!(store.summary().unwrap().citations, 0);
}

#[test]
fn should_keep_one_snapshot_when_another_connection_commits_between_frontier_and_edges() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let a = store.allocate_work("reader-a").unwrap();
    let b = store.allocate_work("reader-b").unwrap();
    let c = store.allocate_work("reader-c").unwrap();
    let mut admission = fixture(&store, &a, "snapshot");
    admission.citations.push(Citation {
        citing: a.clone(),
        cited: b.clone(),
        evidence: vec![evidence(&admission)],
    });
    store.admit(&admission, store.data_root()).unwrap();
    let mut writer = Connection::open(store.database_path()).unwrap();
    let a_write = a.clone();
    let c_write = c.clone();
    query::FRONTIER_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let transaction = writer.transaction().unwrap();
            transaction
                .execute("DELETE FROM citation_evidence", [])
                .unwrap();
            transaction.execute("DELETE FROM citations", []).unwrap();
            transaction
                .execute(
                    "INSERT INTO citations VALUES(?1,?2)",
                    params![a_write.as_str(), c_write.as_str()],
                )
                .unwrap();
            transaction.commit().unwrap();
        }))
    });
    let graph = store.two_hop(&a, NeighborhoodLimits::default()).unwrap();
    assert_eq!(
        graph.edges,
        vec![NeighborhoodEdge {
            citing: a.clone(),
            cited: b
        }]
    );
    let next = store.two_hop(&a, NeighborhoodLimits::default()).unwrap();
    assert_eq!(
        next.edges,
        vec![NeighborhoodEdge {
            citing: a,
            cited: c
        }]
    );
}
