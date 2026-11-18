use std::path::Path;

use chrono::{Duration, Utc};
use serde_json::{Value, json};
use tempfile::tempdir;

use super::*;
use crate::{
    Error, Result,
    domain::{PaperMetadata, PaperOverview},
    store::ArtifactStore,
};

fn analysis(generated_at: &str, version: Option<&str>) -> Value {
    let mut value = json!({
        "schema_version": 5,
        "provider": "openrouter",
        "generated_at": generated_at,
        "thesis": "Import competition from China lowered US local employment.",
        "outsider_brief": "",
        "author_abstract": "We analyze the effect of rising Chinese import competition.",
        "sections": [{"id": "s1", "title": "The China shock", "summary": "Motivation."}],
        "outline": [{"title": "I. Introduction", "level": 1, "page": 1}],
        "logical_sections": [{"title": "Why trade shocks are local", "covers": ["I", "II"]}],
        "context": {"read_alongside": [{"title": "Topalova 2010"}]},
        "glossary": [{"term": "commuting zone"}],
        "claims": [{"text": "not reviewed"}]
    });
    if let Some(version) = version {
        value["pipeline_version"] = json!(version);
    }
    value
}

fn subject(id: &PaperId) -> ReviewSubject {
    ReviewSubject {
        paper_id: id.clone(),
        paper_title: "The China Syndrome".to_owned(),
        manifest_id: Some("autor2013".to_owned()),
    }
}

fn score(value: u32) -> GradeValue {
    GradeValue::Number(value)
}

fn graded(values: &[(&str, GradeValue)]) -> ComponentGrade {
    ComponentGrade {
        values: values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
        ..ComponentGrade::default()
    }
}

fn issue(id: &str, component: &str, severity: &str) -> IssueFlag {
    IssueFlag {
        id: id.to_owned(),
        component: component.to_owned(),
        severity: severity.to_owned(),
        tag: Some("generic_text".to_owned()),
        note: "Could describe any trade paper.".to_owned(),
        quote: Some("an influential paper".to_owned()),
    }
}

fn review_at(
    rubric: &Rubric,
    id: &PaperId,
    version: Option<&str>,
    input: ReviewInput,
    minutes: i64,
) -> Review {
    let snapshot = snapshot_from_analysis(&analysis("2026-11-01T00:00:00Z", version));
    Review::from_input(
        rubric,
        subject(id),
        snapshot,
        input,
        None,
        Utc::now() + Duration::minutes(minutes),
    )
}

fn overview(filename: &str, title: &str) -> PaperOverview {
    PaperOverview {
        id: PaperId::from_relative_path(Path::new(filename)),
        metadata: PaperMetadata {
            title: title.to_owned(),
            ..PaperMetadata::default()
        },
        relative_path: format!("review-set/{filename}"),
        status: crate::domain::ProcessingStatus::default(),
        analyzed_at: None,
        one_line_summary: None,
        author_abstract: None,
    }
}

#[test]
fn should_parse_the_bundled_rubric_and_manifest() -> Result<()> {
    let rubric = Rubric::bundled()?;
    let ids = rubric
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["abstract", "thesis", "context", "outline", "logical"]);
    assert!(
        rubric
            .component("context")
            .is_some_and(|c| c.hard_reject.is_some())
    );
    let manifest = Manifest::bundled()?;
    assert!(manifest.papers.len() >= 9);
    assert!(manifest.papers.iter().any(|paper| paper.id == "card1990"));
    Ok(())
}

#[test]
fn should_pin_reviewed_fields_and_leave_out_the_rest() {
    let snapshot = snapshot_from_analysis(&analysis("2026-11-01T00:00:00Z", Some("core-v3")));
    for key in [
        "thesis",
        "author_abstract",
        "sections",
        "outline",
        "logical_sections",
        "context",
    ] {
        assert!(snapshot.field(key).is_some(), "{key} should be pinned");
    }
    assert!(snapshot.field("glossary").is_none());
    assert!(snapshot.field("claims").is_none());
    assert_eq!(snapshot.pipeline_version.as_deref(), Some("core-v3"));
    assert_eq!(
        snapshot.generated_at.as_deref(),
        Some("2026-11-01T00:00:00Z")
    );
    assert_eq!(snapshot.provider.as_deref(), Some("openrouter"));
    assert!(snapshot.snapshot_id.starts_with("s-"));
}

#[test]
fn should_change_snapshot_id_only_when_reviewed_output_changes() {
    let base = analysis("2026-11-01T00:00:00Z", None);
    let same = snapshot_from_analysis(&base).snapshot_id;
    assert_eq!(same, snapshot_from_analysis(&base).snapshot_id);
    let mut glossary_only = base.clone();
    glossary_only["glossary"] = json!([]);
    assert_eq!(same, snapshot_from_analysis(&glossary_only).snapshot_id);
    let rerun = analysis("2026-11-02T00:00:00Z", None);
    assert_ne!(same, snapshot_from_analysis(&rerun).snapshot_id);
    let mut regrouped = base;
    regrouped["logical_sections"] = json!([]);
    assert_ne!(same, snapshot_from_analysis(&regrouped).snapshot_id);
    let versioned = analysis("2026-11-01T00:00:00Z", Some("core-v3"));
    assert_ne!(same, snapshot_from_analysis(&versioned).snapshot_id);
}

#[test]
fn should_read_odd_analysis_shapes_without_failing() {
    let snapshot = snapshot_from_analysis(&json!({"pipeline": {"version": 7}, "thesis": null}));
    assert_eq!(snapshot.pipeline_version.as_deref(), Some("7"));
    assert!(snapshot.field("thesis").is_none());
    let empty = snapshot_from_analysis(&json!("not an object"));
    assert!(empty.fields.is_empty());
}

#[tokio::test]
async fn should_round_trip_reviews_and_keep_earlier_snapshots() -> Result<()> {
    let directory = tempdir().map_err(|error| Error::io("tempdir", error))?;
    let store = ArtifactStore::new(directory.path());
    store.initialize().await?;
    let rubric = Rubric::bundled()?;
    let id = PaperId::from_relative_path(Path::new("autor2013-china-syndrome.pdf"));
    let first = snapshot_from_analysis(&analysis("2026-11-01T00:00:00Z", Some("core-v3")));
    let input = ReviewInput {
        snapshot_id: first.snapshot_id.clone(),
        components: [("context".to_owned(), graded(&[("accuracy", score(3))]))].into(),
        overall: Some(3),
        notes: "  Good orientation.  ".to_owned(),
        issues: vec![issue("i1", "context", "major")],
    };
    rubric.validate(&input)?;
    let created = Utc::now();
    let review = Review::from_input(
        &rubric,
        subject(&id),
        first.clone(),
        input.clone(),
        None,
        created,
    );
    save_review(&store, &review).await?;

    let loaded = load_review(&store, &id, &first.snapshot_id)
        .await?
        .ok_or_else(|| Error::Task("review missing".to_owned()))?;
    assert_eq!(loaded.notes, "Good orientation.");
    assert_eq!(loaded.components["context"].values["accuracy"], score(3));
    assert_eq!(loaded.snapshot, first);

    let later = created + Duration::minutes(5);
    let updated = Review::from_input(
        &rubric,
        subject(&id),
        first.clone(),
        input,
        Some(&loaded),
        later,
    );
    assert_eq!(updated.created_at, loaded.created_at);
    assert_eq!(updated.updated_at, later);

    // Re-analysis yields a new snapshot with no review; the old one survives
    // with the text it graded.
    let second = snapshot_from_analysis(&analysis("2026-11-05T00:00:00Z", Some("core-v4")));
    assert!(
        load_review(&store, &id, &second.snapshot_id)
            .await?
            .is_none()
    );
    let all = load_all_reviews(&store).await?;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].pipeline_version(), "core-v3");
    assert_eq!(all[0].snapshot.field("thesis"), first.field("thesis"),);
    assert!(load_review(&store, &id, "../escape").await.is_err());
    Ok(())
}

#[tokio::test]
async fn should_skip_malformed_review_files() -> Result<()> {
    let directory = tempdir().map_err(|error| Error::io("tempdir", error))?;
    let store = ArtifactStore::new(directory.path());
    store.initialize().await?;
    let id = PaperId::from_relative_path(Path::new("paper.pdf"));
    let reviews = store.paper_dir(&id).join("reviews");
    tokio::fs::create_dir_all(&reviews)
        .await
        .map_err(|error| Error::io(&reviews, error))?;
    tokio::fs::write(reviews.join("s-broken.json"), b"{not json")
        .await
        .map_err(|error| Error::io(&reviews, error))?;
    assert!(load_paper_reviews(&store, &id).await?.is_empty());
    Ok(())
}

#[test]
fn should_reject_grades_that_do_not_fit_the_rubric() -> Result<()> {
    let rubric = Rubric::bundled()?;
    let with = |component: &str, grade: ComponentGrade| ReviewInput {
        snapshot_id: "s-1".to_owned(),
        components: [(component.to_owned(), grade)].into(),
        ..ReviewInput::default()
    };
    let rejected = [
        with("glossary", graded(&[])),
        with("context", graded(&[("accuracy", score(5))])),
        with("context", graded(&[("accuracy", GradeValue::Flag(true))])),
        with("context", graded(&[("vibes", score(2))])),
        with(
            "logical",
            graded(&[("granularity", GradeValue::Choice("huge".to_owned()))]),
        ),
        with(
            "outline",
            ComponentGrade {
                hard_reject: true,
                ..ComponentGrade::default()
            },
        ),
        ReviewInput {
            overall: Some(9),
            ..ReviewInput::default()
        },
        ReviewInput {
            issues: vec![issue("i1", "context", "catastrophic")],
            ..ReviewInput::default()
        },
        ReviewInput {
            issues: vec![
                issue("i1", "context", "minor"),
                issue("i1", "outline", "minor"),
            ],
            ..ReviewInput::default()
        },
    ];
    for input in &rejected {
        assert!(rubric.validate(input).is_err(), "accepted {input:?}");
    }
    let accepted = ReviewInput {
        snapshot_id: "s-1".to_owned(),
        components: [
            (
                "abstract".to_owned(),
                graded(&[
                    ("exact", GradeValue::Flag(false)),
                    ("error_type", GradeValue::Choice("truncated".to_owned())),
                ]),
            ),
            (
                "outline".to_owned(),
                graded(&[("missing_headings", score(12))]),
            ),
            (
                "context".to_owned(),
                ComponentGrade {
                    not_produced: true,
                    hard_reject: true,
                    ..ComponentGrade::default()
                },
            ),
        ]
        .into(),
        overall: Some(0),
        issues: vec![issue("i1", GENERAL_COMPONENT, "blocker")],
        ..ReviewInput::default()
    };
    rubric.validate(&accepted)
}

#[test]
fn should_summarise_by_pipeline_version_and_component() -> Result<()> {
    let rubric = Rubric::bundled()?;
    let first = PaperId::from_relative_path(Path::new("a.pdf"));
    let second = PaperId::from_relative_path(Path::new("b.pdf"));
    let review = |id: &PaperId, version: Option<&str>, accuracy: u32, exact: bool, minutes| {
        review_at(
            &rubric,
            id,
            version,
            ReviewInput {
                snapshot_id: String::new(),
                components: [
                    (
                        "context".to_owned(),
                        graded(&[("accuracy", score(accuracy)), ("orientation", score(2))]),
                    ),
                    (
                        "abstract".to_owned(),
                        graded(&[("exact", GradeValue::Flag(exact))]),
                    ),
                    (
                        "logical".to_owned(),
                        graded(&[("granularity", GradeValue::Choice("too_many".to_owned()))]),
                    ),
                    (
                        "outline".to_owned(),
                        graded(&[("missing_headings", score(2))]),
                    ),
                ]
                .into(),
                overall: Some(accuracy),
                notes: String::new(),
                issues: vec![issue("i1", "context", "major")],
            },
            minutes,
        )
    };
    let reviews = vec![
        review(&first, None, 1, false, 0),
        review(&second, None, 3, true, 1),
        review(&first, Some("core-v4"), 4, true, 2),
    ];
    let summary = summarize(&rubric, &reviews);
    assert_eq!(summary.reviews, 3);
    let versions = summary
        .groups
        .iter()
        .map(|group| group.pipeline_version.as_str())
        .collect::<Vec<_>>();
    assert_eq!(versions, [UNVERSIONED, "core-v4"]);
    let old = &summary.groups[0];
    assert_eq!((old.reviews, old.papers), (2, 2));
    assert_eq!(old.overall.mean, Some(2.0));
    let context = &old.components["context"];
    assert_eq!(
        context.dimensions["accuracy"],
        DimensionSummary::Score {
            n: 2,
            mean: Some(2.0)
        }
    );
    assert_eq!(context.score_mean, Some(2.0));
    assert_eq!(context.issues["major"], 2);
    assert_eq!(
        old.components["abstract"].dimensions["exact"],
        DimensionSummary::PassFail {
            n: 2,
            passed: 1,
            pass_rate: Some(0.5)
        }
    );
    assert_eq!(
        old.components["logical"].dimensions["granularity"],
        DimensionSummary::Choice {
            n: 2,
            counts: [("too_many".to_owned(), 2)].into()
        }
    );
    assert_eq!(
        old.components["outline"].dimensions["missing_headings"],
        DimensionSummary::Count {
            n: 2,
            total: 4,
            mean: Some(2.0)
        }
    );
    assert_eq!(old.components["thesis"].graded, 0);
    assert_eq!(summary.issues_by_component["context"]["major"], 3);
    assert_eq!(summary.issues_by_tag["generic_text"], 3);
    let text = render_summary(&rubric, &summary);
    assert!(text.contains("== pipeline core-v4 (1 reviews, 1 papers) overall 4.00"));
    assert!(text.contains("1/2 pass (50%)"));
    Ok(())
}

#[test]
fn should_export_issues_with_the_output_they_flag() -> Result<()> {
    let rubric = Rubric::bundled()?;
    let id = PaperId::from_relative_path(Path::new("a.pdf"));
    let review = review_at(
        &rubric,
        &id,
        Some("core-v4"),
        ReviewInput {
            issues: vec![issue("i1", "logical", "minor")],
            ..ReviewInput::default()
        },
        0,
    );
    let exported = export(&rubric, json!({}), vec![review], Utc::now());
    assert_eq!(exported.issues.len(), 1);
    let output = &exported.issues[0].reviewed_output;
    assert!(output.contains_key("logical_sections"));
    assert!(output.contains_key("sections"));
    assert!(!output.contains_key("thesis"));
    assert_eq!(exported.issues[0].pipeline_version, "core-v4");
    assert_eq!(exported.summary.reviews, 1);
    Ok(())
}

#[test]
fn should_match_library_papers_to_the_manifest() -> Result<()> {
    let manifest = Manifest::bundled()?;
    let by_file = overview("card1990-mariel-boatlift.pdf", "Untitled scan");
    assert_eq!(
        manifest_match(&manifest, &by_file).map(|entry| entry.id.as_str()),
        Some("card1990")
    );
    let by_title = overview(
        "downloaded.pdf",
        "The Market for “Lemons”: Quality Uncertainty and the Market Mechanism",
    );
    assert_eq!(
        manifest_match(&manifest, &by_title).map(|entry| entry.id.as_str()),
        Some("akerlof1970")
    );
    assert!(manifest_match(&manifest, &overview("other.pdf", "Something else")).is_none());
    assert!(manifest_match(&manifest, &overview("other.pdf", "")).is_none());
    Ok(())
}

#[tokio::test]
async fn should_report_progress_across_the_evaluation_set() -> Result<()> {
    let directory = tempdir().map_err(|error| Error::io("tempdir", error))?;
    let store = ArtifactStore::new(directory.path());
    store.initialize().await?;
    let manifest = Manifest::bundled()?;
    let rubric = Rubric::bundled()?;
    let analysed = overview("autor2013-china-syndrome.pdf", "The China Syndrome");
    let present = overview("card1990-mariel-boatlift.pdf", "Mariel");
    let paper_dir = store.paper_dir(&analysed.id);
    tokio::fs::create_dir_all(&paper_dir)
        .await
        .map_err(|error| Error::io(&paper_dir, error))?;
    let current = analysis("2026-11-01T00:00:00Z", None);
    tokio::fs::write(paper_dir.join("analysis.json"), current.to_string())
        .await
        .map_err(|error| Error::io(&paper_dir, error))?;
    let review = review_at(&rubric, &analysed.id, None, ReviewInput::default(), 0);
    save_review(&store, &review).await?;

    let rows = progress(&store, &manifest, &[analysed.clone(), present.clone()]).await?;
    assert_eq!(rows.len(), manifest.papers.len());
    let row = |id: &str| rows.iter().find(|row| row.manifest_id == id).cloned();
    let autor = row("autor2013").ok_or_else(|| Error::Task("autor2013".to_owned()))?;
    assert_eq!(autor.paper_id.as_ref(), Some(&analysed.id));
    assert!(autor.analysed && autor.reviewed && !autor.reviewed_earlier);
    let card = row("card1990").ok_or_else(|| Error::Task("card1990".to_owned()))?;
    assert!(card.paper_id.is_some() && !card.analysed && !card.reviewed);
    let akerlof = row("akerlof1970").ok_or_else(|| Error::Task("akerlof1970".to_owned()))?;
    assert!(akerlof.paper_id.is_none());

    // A new analysis leaves the old review behind as an earlier one.
    tokio::fs::write(
        paper_dir.join("analysis.json"),
        analysis("2026-11-09T00:00:00Z", None).to_string(),
    )
    .await
    .map_err(|error| Error::io(&paper_dir, error))?;
    let rows = progress(&store, &manifest, &[analysed]).await?;
    let autor = rows
        .iter()
        .find(|row| row.manifest_id == "autor2013")
        .ok_or_else(|| Error::Task("autor2013".to_owned()))?;
    assert!(!autor.reviewed && autor.reviewed_earlier);
    Ok(())
}
