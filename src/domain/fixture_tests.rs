//! The shared reader fixtures in `tests/fixtures/reader/` are loaded by these
//! tests and by the web's `scripts/reader-output.test.mjs` and
//! `scripts/reader-output-smoke.mjs`, so a field renamed or added on one side
//! without the other fails a test instead of silently reading as empty.

use std::collections::BTreeSet;

use serde_json::Value;

use super::{ContextBasis, OutlineRole, PaperAnalysis, QualityWarningKind};

const CURRENT: &str = include_str!("../../tests/fixtures/reader/analysis-current.json");
const LEGACY: &str = include_str!("../../tests/fixtures/reader/analysis-legacy.json");

/// Every object key path in a JSON value, with array elements merged under `[]`.
fn key_paths(value: &Value, prefix: &str, paths: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                paths.insert(path.clone());
                key_paths(child, &path, paths);
            }
        }
        Value::Array(items) => {
            for item in items {
                key_paths(item, &format!("{prefix}[]"), paths);
            }
        }
        _ => {}
    }
}

fn paths_of(value: &Value) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    key_paths(value, "", &mut paths);
    paths
}

#[test]
fn should_load_every_new_field_from_the_current_reader_fixture() {
    let analysis: PaperAnalysis = serde_json::from_str(CURRENT).expect("current fixture loads");

    assert_eq!(analysis.outline.len(), 13);
    assert_eq!(analysis.outline[0].role, OutlineRole::FrontMatter);
    assert_eq!(analysis.outline[11].role, OutlineRole::References);
    assert!(analysis.outline[1].anchor.is_some());
    assert_eq!(analysis.sections[0].outline_entries, vec![1, 2, 3]);
    assert!(analysis.sections[0].question.is_some());
    assert!(analysis.sections[4].question.is_none());

    let context = analysis.context.as_ref().expect("context");
    let bases: BTreeSet<_> = context.claims().map(|claim| format!("{:?}", claim.basis)).collect();
    assert_eq!(bases.len(), 3, "the fixture exercises every source class");
    assert_eq!(context.lead[1].basis, ContextBasis::Paper);
    assert_eq!(context.lead[1].evidence[0].page, 4);
    assert!(context.before[1].downgraded.is_some());
    assert_eq!(context.read_alongside.len(), 3);
    assert_eq!(
        context.read_alongside[0].local_paper_id.as_deref(),
        Some("fixture-local-1")
    );
    assert!(context.reception_note.is_some());
    assert_eq!(context.counts.dossier_works, 37);

    assert!(analysis.pipeline_version.is_some());
    assert_eq!(analysis.quality_warnings.len(), 2);
    assert!(analysis
        .quality_warnings
        .iter()
        .all(|warning| warning.kind != QualityWarningKind::Other));
    assert_eq!(
        analysis.abstract_agreement.as_ref().and_then(|agreement| agreement.token_f1),
        Some(0.97)
    );
}

#[test]
fn should_round_trip_the_current_reader_fixture_without_losing_or_adding_fields() {
    let fixture: Value = serde_json::from_str(CURRENT).expect("fixture is JSON");
    let analysis: PaperAnalysis = serde_json::from_value(fixture.clone()).expect("fixture loads");
    let written = serde_json::to_value(&analysis).expect("analysis serializes");

    let fixture_paths = paths_of(&fixture);
    let written_paths = paths_of(&written);
    let dropped: Vec<_> = fixture_paths.difference(&written_paths).collect();
    let unexercised: Vec<_> = written_paths.difference(&fixture_paths).collect();
    assert!(
        dropped.is_empty(),
        "fields in the fixture (and so in web/src/types.ts) that PaperAnalysis drops: {dropped:?}"
    );
    assert!(
        unexercised.is_empty(),
        "PaperAnalysis fields the reader fixture does not exercise; add them to \
         tests/fixtures/reader/analysis-current.json and web/src/types.ts: {unexercised:?}"
    );
}

#[test]
fn should_load_a_legacy_analysis_written_before_the_outline_and_context_node() {
    let analysis: PaperAnalysis = serde_json::from_str(LEGACY).expect("legacy fixture loads");

    assert!(analysis.outline.is_empty());
    assert!(analysis.context.is_none());
    assert!(analysis.abstract_agreement.is_none());
    assert!(analysis.pipeline_version.is_none());
    assert!(analysis.quality_warnings.is_empty());
    assert_eq!(analysis.sections.len(), 1);
    assert!(analysis.sections[0].outline_entries.is_empty());
    assert!(analysis.sections[0].question.is_none());
    assert_eq!(analysis.context_notes[0].basis, None);
}
