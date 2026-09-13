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
    assert_eq!(version, MIGRATIONS.len());
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

fn versioned(admission: &mut Admission, version: &str, hash: char) {
    let ObservationSource::PdfMetadata { paper_id } = &admission.observation.source else {
        unreachable!()
    };
    let EntityProjection::Work(work) = &mut admission.entity else {
        unreachable!()
    };
    work.versions.push(crate::kb::WorkVersion {
        id: version.into(),
        kind: crate::kb::VersionKind::Published,
        label: None,
        identifiers: Default::default(),
        date: None,
    });
    work.local_copies.push(crate::kb::LocalCopy {
        paper_id: paper_id.clone(),
        content_hash: hash.to_string().repeat(64),
        version_id: Some(version.into()),
    });
}

#[test]
fn should_retain_other_contributions_when_merged_producers_refresh_and_split() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let a = store.allocate_work("source-a").unwrap();
    let b = store.allocate_work("source-b").unwrap();
    let mut one = fixture(&store, &a, "A title");
    let mut two = fixture(&store, &b, "B competing title");
    versioned(&mut one, "v-a", 'a');
    versioned(&mut two, "v-b", 'b');
    let shared = crate::kb::Identifier::parse("doi:10.1234/shared").unwrap();
    for admission in [&mut one, &mut two] {
        let EntityProjection::Work(work) = &mut admission.entity else {
            unreachable!()
        };
        work.identifiers.insert(shared.clone());
        store.admit(admission, store.data_root()).unwrap();
    }
    store
        .record_decision(&merge_decision(a.clone(), b.clone()))
        .unwrap();
    for admission in [&two, &one, &two] {
        store.admit(admission, store.data_root()).unwrap();
        let merged = store.work(&b).unwrap().unwrap();
        assert_eq!(merged.id, a);
        assert_eq!(merged.title.as_deref(), Some("A title"));
        assert_eq!(merged.versions.len(), 2);
        assert_eq!(merged.local_copies.len(), 2);
        assert_eq!(store.assertions(b.as_str()).unwrap().len(), 2);
    }
    let EntityProjection::Work(value) = &mut one.entity else {
        unreachable!()
    };
    value.identifiers.clear();
    value.local_copies.clear();
    value.versions.clear();
    store.admit(&one, store.data_root()).unwrap();
    let merged = store.work(&a).unwrap().unwrap();
    assert!(merged.identifiers.contains(&shared));
    assert_eq!(merged.local_copies.len(), 1);
    assert_eq!(merged.versions[0].id, "v-b");
    assert_eq!(
        store.works_with_identifier(&shared, 10).unwrap().ids,
        [a.to_string()]
    );
    let created: WorkId = "WsplitContributions".parse().unwrap();
    store
        .record_decision(&Decision {
            id: "split-contributions".into(),
            recorded_at: Utc::now(),
            rationale: "separate independent assertions".into(),
            action: DecisionAction::Work(EntityDecision::Split {
                original: a.clone(),
                created: created.clone(),
                observation_ids: vec![two.observation.id.clone()],
            }),
        })
        .unwrap();
    assert!(store.work(&a).unwrap().unwrap().local_copies.is_empty());
    assert!(store.work(&a).unwrap().unwrap().identifiers.is_empty());
    assert_eq!(store.work(&created).unwrap().unwrap().local_copies.len(), 1);
    assert_eq!(store.work(&b).unwrap().unwrap().id, a);
    store.admit(&two, store.data_root()).unwrap();
    assert_eq!(
        store.works_with_identifier(&shared, 10).unwrap().ids,
        [created.to_string()]
    );
    let before = store.snapshot().unwrap();
    store.rebuild().unwrap();
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_preserve_canonical_state_when_version_or_copy_assertions_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_work("conflicts").unwrap();
    let mut one = fixture(&store, &id, "A source");
    versioned(&mut one, "published", 'a');
    store.admit(&one, store.data_root()).unwrap();
    let before = store.snapshot().unwrap();
    let journal = fs::read(directory.path().join("kb/decisions.jsonl")).unwrap();
    let mut two = fixture(&store, &id, "B source");
    versioned(&mut two, "published", 'b');
    let EntityProjection::Work(value) = &mut two.entity else {
        unreachable!()
    };
    value.versions[0].label = Some("conflicting version metadata".into());
    assert!(store.admit(&two, store.data_root()).is_err());
    let EntityProjection::Work(value) = &mut two.entity else {
        unreachable!()
    };
    value.versions[0].label = None;
    let EntityProjection::Work(original) = &one.entity else {
        unreachable!()
    };
    value.local_copies[0].paper_id = original.local_copies[0].paper_id.clone();
    assert!(store.admit(&two, store.data_root()).is_err());
    assert_eq!(before, store.snapshot().unwrap());
    assert_eq!(
        journal,
        fs::read(directory.path().join("kb/decisions.jsonl")).unwrap()
    );
    let EntityProjection::Work(value) = &mut one.entity else {
        unreachable!()
    };
    value.local_copies[0].version_id = Some("missing-version".into());
    assert!(store.admit(&one, store.data_root()).is_err());
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_return_nonunique_indexed_candidates_when_identifiers_collide() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let identifier = crate::kb::Identifier::parse("doi:10.1234/collision").unwrap();
    let mut ids = Vec::new();
    for label in ["left", "right"] {
        let id = store.allocate_work(label).unwrap();
        let mut admission = fixture(&store, &id, label);
        let EntityProjection::Work(work) = &mut admission.entity else {
            unreachable!()
        };
        work.identifiers.insert(identifier.clone());
        store.admit(&admission, store.data_root()).unwrap();
        ids.push(id.to_string());
    }
    ids.sort();
    assert_eq!(
        store.works_with_identifier(&identifier, 10).unwrap().ids,
        ids
    );
    let capped = store.works_with_identifier(&identifier, 1).unwrap();
    assert_eq!(capped.ids, ids[..1]);
    assert!(capped.truncated);
    let plan: String = store.connection().unwrap().query_row(
        "EXPLAIN QUERY PLAN SELECT entity_id FROM entity_identifiers WHERE identifier=?1 ORDER BY entity_id LIMIT 10",
        [identifier.key()], |row| row.get(3)).unwrap();
    assert!(plan.contains("entity_identifiers_lookup"), "{plan}");
    assert_eq!(store.summary().unwrap().aliases, 0);
}

#[test]
fn should_recompute_search_keys_when_a_producer_supplies_a_stale_title_key() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_work("title-key").unwrap();
    let mut admission = fixture(&store, &id, "State-of-the-art Methods");
    let EntityProjection::Work(work) = &mut admission.entity else {
        unreachable!()
    };
    work.title_key = Some("obsolete marker".into());
    store.admit(&admission, store.data_root()).unwrap();
    assert_eq!(
        store.search_titles("state-of-the-art", 10).unwrap().ids,
        [id.to_string()]
    );
    assert!(
        store
            .search_titles("obsolete marker", 10)
            .unwrap()
            .ids
            .is_empty()
    );
    assert_eq!(
        store.work(&id).unwrap().unwrap().title_key.as_deref(),
        Some("state of the art methods")
    );
    let before = store.snapshot().unwrap();
    store.rebuild().unwrap();
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_reject_changed_journal_prefix_when_a_live_handle_attempts_to_append() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    store.allocate_work("original-origin").unwrap();
    let before = store.snapshot().unwrap();
    let path = directory.path().join("kb/decisions.jsonl");
    let original = fs::read_to_string(&path).unwrap();
    let modified = original.replace("original-origin", "modified-origin");
    assert_eq!(original.len(), modified.len());
    fs::write(&path, &modified).unwrap();
    assert!(store.allocate_work("later-origin").is_err());
    assert!(store.rebuild().is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), modified);
    assert_eq!(before, store.snapshot().unwrap());
    drop(store);
    assert!(KbStore::open(directory.path()).is_err());
}

fn person_fixture(
    store: &KbStore,
    id: &PersonId,
    label: &str,
    family: &str,
    given: &str,
) -> Admission {
    let work = store
        .allocate_work(&format!("person-source-{label}"))
        .unwrap();
    let mut admission = fixture(store, &work, label);
    admission.entity = EntityProjection::Person(Person {
        id: id.clone(),
        display_name: format!("{given} {family}"),
        family_name: Some(family.into()),
        given_names: vec![given.into()],
        particles: vec![],
        suffix: None,
        name_variants: vec![crate::kb::NameVariant {
            name: label.into(),
            count: 2,
        }],
        identifiers: Default::default(),
    });
    admission
}

#[test]
fn should_reject_noncanonical_person_identifiers_when_admitting_or_querying_them() {
    use crate::kb::PersonIdentifier::{OpenalexAuthor, Orcid, SemanticScholarAuthor};
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_person("identifier-person").unwrap();
    let mut admission = person_fixture(&store, &id, "A person", "Example", "Alice");
    let valid = [
        Orcid("0000-0002-1825-0097".into()),
        Orcid("0000-0002-1694-233X".into()),
        OpenalexAuthor("A123".into()),
        SemanticScholarAuthor("456".into()),
    ];
    let EntityProjection::Person(person) = &mut admission.entity else {
        unreachable!()
    };
    person.identifiers.extend(valid.iter().cloned());
    store.admit(&admission, store.data_root()).unwrap();
    for identifier in valid {
        assert_eq!(
            store.persons_with_identifier(&identifier, 10).unwrap().ids,
            [id.to_string()]
        );
    }
    let before = store.snapshot().unwrap();
    let invalid = [
        Orcid("0000-0002-1825-0098".into()),
        Orcid("https://orcid.org/0000-0002-1825-0097".into()),
        Orcid("００００-0002-1825-0097".into()),
        Orcid("0000-0002-1694-233x".into()),
        OpenalexAuthor("https://openalex.org/A123".into()),
        OpenalexAuthor("a123".into()),
        OpenalexAuthor("A12\n".into()),
        OpenalexAuthor("W123".into()),
        SemanticScholarAuthor(" 456".into()),
        SemanticScholarAuthor("abc".into()),
    ];
    for identifier in invalid {
        let EntityProjection::Person(person) = &mut admission.entity else {
            unreachable!()
        };
        person.identifiers = [identifier.clone()].into();
        assert!(
            store.admit(&admission, store.data_root()).is_err(),
            "{identifier:?}"
        );
        assert!(store.persons_with_identifier(&identifier, 10).is_err());
        assert_eq!(before, store.snapshot().unwrap());
    }
}

#[test]
fn should_keep_coherent_person_components_when_merged_observations_refresh_and_split() {
    use crate::kb::PersonIdentifier;
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let a = store.allocate_person("person-a").unwrap();
    let b = store.allocate_person("person-b").unwrap();
    let mut one = person_fixture(&store, &a, "A name", "Example", "Alice");
    let mut two = person_fixture(&store, &b, "B name", "Other", "Bob");
    for (admission, identifier) in [(&mut one, "A123"), (&mut two, "A456")] {
        let EntityProjection::Person(person) = &mut admission.entity else {
            unreachable!()
        };
        person
            .identifiers
            .insert(PersonIdentifier::OpenalexAuthor(identifier.into()));
        store.admit(admission, store.data_root()).unwrap();
    }
    store
        .record_decision(&Decision {
            id: "person-merge".into(),
            recorded_at: Utc::now(),
            rationale: "fixture independent assertions".into(),
            action: DecisionAction::Person(EntityDecision::Merge {
                surviving: a.clone(),
                absorbed: b.clone(),
            }),
        })
        .unwrap();
    store.admit(&two, store.data_root()).unwrap();
    let merged = store.person(&b).unwrap().unwrap();
    assert_eq!(merged.id, a);
    assert_eq!(merged.display_name, "Alice Example");
    assert_eq!(merged.family_name.as_deref(), Some("Example"));
    assert_eq!(merged.given_names, ["Alice"]);
    assert_eq!(merged.identifiers.len(), 2);
    assert_eq!(merged.name_variants.len(), 2);
    let claims = store.assertions(a.as_str()).unwrap();
    assert_eq!(claims.len(), 2);
    assert_ne!(claims[0].revision, claims[1].revision);
    let EntityProjection::Person(other) = &claims[1].entity else {
        unreachable!()
    };
    assert_eq!(other.display_name, "Bob Other");
    let created: PersonId = "PsplitPerson".parse().unwrap();
    store
        .record_decision(&Decision {
            id: "person-split".into(),
            recorded_at: Utc::now(),
            rationale: "fixture split".into(),
            action: DecisionAction::Person(EntityDecision::Split {
                original: a.clone(),
                created: created.clone(),
                observation_ids: vec![two.observation.id.clone()],
            }),
        })
        .unwrap();
    store.admit(&two, store.data_root()).unwrap();
    assert_eq!(
        store.person(&created).unwrap().unwrap().display_name,
        "Bob Other"
    );
    assert_eq!(store.person(&a).unwrap().unwrap().identifiers.len(), 1);
    assert_eq!(store.person(&b).unwrap().unwrap().id, a);
    let before = store.snapshot().unwrap();
    store.rebuild().unwrap();
    assert_eq!(before, store.snapshot().unwrap());
}

#[test]
fn should_resume_projection_upgrade_when_a_prior_rebuild_did_not_complete() {
    let directory = tempfile::tempdir().unwrap();
    let store = KbStore::open(directory.path()).unwrap();
    let id = store.allocate_work("upgrade").unwrap();
    let mut admission = fixture(&store, &id, "Upgrade title");
    let EntityProjection::Work(work) = &mut admission.entity else {
        unreachable!()
    };
    work.identifiers
        .insert(crate::kb::Identifier::parse("doi:10.1234/upgrade").unwrap());
    store.admit(&admission, store.data_root()).unwrap();
    let before = store.snapshot().unwrap();
    store.connection().unwrap().execute_batch(
        "DELETE FROM entity_assertions; DELETE FROM entity_identifiers; UPDATE projection_state SET projection_version=0"
    ).unwrap();
    drop(store);
    let reopened = KbStore::open(directory.path()).unwrap();
    assert_eq!(before, reopened.snapshot().unwrap());
}

#[test]
fn should_reject_version_pinned_work_ids_when_arxiv_versions_belong_on_work_versions() {
    for (base, pinned) in [
        ("2608.00001", "2608.00001v2"),
        ("gr-qc/9901001", "gr-qc/9901001v12"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let store = KbStore::open(directory.path()).unwrap();
        let id = store.allocate_work("version-boundary").unwrap();
        let mut admission = fixture(&store, &id, "Version-specific source");
        let version_identifier = crate::kb::Identifier::parse(&format!("arxiv:{pinned}")).unwrap();
        let base_identifier = crate::kb::Identifier::parse(&format!("arxiv:{base}")).unwrap();
        let before = store.snapshot().unwrap();
        let journal_path = directory.path().join("kb/decisions.jsonl");
        let journal = fs::read(&journal_path).unwrap();
        let EntityProjection::Work(work) = &mut admission.entity else {
            unreachable!()
        };
        work.identifiers.insert(version_identifier.clone());
        assert!(store.admit(&admission, store.data_root()).is_err());
        assert_eq!(journal, fs::read(&journal_path).unwrap());
        assert_eq!(before, store.snapshot().unwrap());
        let EntityProjection::Work(work) = &mut admission.entity else {
            unreachable!()
        };
        work.identifiers.clear();
        work.versions.push(crate::kb::WorkVersion {
            id: "source-version".into(),
            kind: crate::kb::VersionKind::Preprint,
            label: None,
            identifiers: [version_identifier.clone()].into(),
            date: None,
        });
        store.admit(&admission, store.data_root()).unwrap();
        let projected = store.work(&id).unwrap().unwrap();
        assert!(projected.identifiers.is_empty());
        assert!(
            projected.versions[0]
                .identifiers
                .contains(&version_identifier)
        );
        assert!(
            store
                .works_with_identifier(&version_identifier, 10)
                .is_err()
        );
        assert!(
            store
                .works_with_identifier(&base_identifier, 10)
                .unwrap()
                .ids
                .is_empty()
        );
        let EntityProjection::Work(work) = &mut admission.entity else {
            unreachable!()
        };
        work.identifiers.insert(base_identifier.clone());
        store.admit(&admission, store.data_root()).unwrap();
        assert_eq!(
            store
                .works_with_identifier(&base_identifier, 10)
                .unwrap()
                .ids,
            [id.to_string()]
        );
        let before = store.snapshot().unwrap();
        store.rebuild().unwrap();
        assert_eq!(before, store.snapshot().unwrap());
    }
}
