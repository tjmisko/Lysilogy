use super::*;
use crate::{
    analysis::{AnalysisService, LocalCliAnalyzer},
    domain::{DocumentLayout, ExtractedPage, PaperMetadata},
    extract::PdfExtractor,
    reader_tools::{CutParagraph, CutSegment, SegmentKind},
};
use std::{os::unix::fs::PermissionsExt, path::Path};
use tempfile::TempDir;

async fn fixture(output: &serde_json::Value) -> (TempDir, AppState, PaperId, PaperId) {
    let directory = tempfile::tempdir().unwrap();
    let library = directory.path().join("library");
    let data = directory.path().join("data");
    tokio::fs::create_dir(&library).await.unwrap();
    for name in ["current.pdf", "cited.pdf"] {
        tokio::fs::write(library.join(name), b"unused: extraction is cached")
            .await
            .unwrap();
    }
    let command = directory.path().join("reader-stub");
    let output = serde_json::to_string(output)
        .unwrap()
        .replace('\'', "'\\''");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > arguments.txt\ncat > prompt.txt\nprintf '%s' '{output}'\n"
    );
    tokio::fs::write(&command, script).await.unwrap();
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
    let state = AppState::with_services(
        &library,
        &data,
        PdfExtractor::default(),
        AnalysisService::new(LocalCliAnalyzer::with_commands(&command, &command)),
    )
    .await
    .unwrap();
    let current = PaperId::from_relative_path(Path::new("current.pdf"));
    let cited = PaperId::from_relative_path(Path::new("cited.pdf"));
    for id in [&current, &cited] {
        state.store.save_extraction(id, &source()).await.unwrap();
    }
    (directory, state, current, cited)
}

fn source() -> ExtractedPaper {
    ExtractedPaper {
        metadata: PaperMetadata { title: "Mechanisms".to_owned(), page_count: Some(10), ..PaperMetadata::default() },
        pages: (1..=10).map(|number| ExtractedPage {
            number, text: format!("Passage {number} describes a distinct mechanism with evidence and qualifications."),
        }).collect(),
        layout: DocumentLayout::default(),
    }
}

fn cut() -> CutDraft {
    CutDraft {
        paragraphs: source()
            .pages
            .iter()
            .map(|page| CutParagraph {
                cut_page: 1,
                segments: vec![CutSegment {
                    kind: SegmentKind::Source,
                    text: page.text.clone(),
                    source_page: Some(page.number),
                }],
            })
            .collect(),
    }
}

async fn add_reference(state: &AppState, id: &PaperId) -> SavedReference {
    save_reference(
        State(state.clone()),
        AxumPath(id.to_string()),
        Json(SaveReference {
            citation: "Cited Author — Mechanisms".to_owned(),
            source_page: Some(1),
            note: "Check the assumption".to_owned(),
        }),
    )
    .await
    .unwrap()
    .1
    .0
}

async fn wait_for_job(state: &AppState, id: &PaperId, job: &ToolJob) -> ReaderTools {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let tools = state.store.load_reader_tools(id).await.unwrap();
            if tools
                .jobs
                .iter()
                .any(|entry| entry.id == job.id && entry.status != ToolJobStatus::Running)
            {
                return tools;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn queued_supercut_validates_output_and_preserves_canonical_artifacts() {
    let (_directory, state, current, _) = fixture(&serde_json::to_value(cut()).unwrap()).await;
    for filename in ["analysis.json", "digest.md", "highlights.jsonl"] {
        tokio::fs::write(
            state.store.paper_dir(&current).join(filename),
            b"canonical marker",
        )
        .await
        .unwrap();
    }
    let (status, Json(job)) = start_job(
        State(state.clone()),
        AxumPath(current.to_string()),
        Json(StartJob {
            provider: AnalysisProvider::Claude,
            action: ToolAction::Supercut {
                format: CutFormat::TenParagraphs,
            },
        }),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::ACCEPTED);
    let tools = wait_for_job(&state, &current, &job).await;
    assert_eq!(tools.jobs[0].status, ToolJobStatus::Completed);
    assert_eq!(tools.supercuts.len(), 1);
    assert_eq!(
        tools.supercuts[0].source_words,
        tools.supercuts[0].total_words
    );
    for filename in ["analysis.json", "digest.md", "highlights.jsonl"] {
        assert_eq!(
            tokio::fs::read(state.store.paper_dir(&current).join(filename))
                .await
                .unwrap(),
            b"canonical marker"
        );
    }
    let prompt = tokio::fs::read_to_string(
        state
            .store
            .paper_dir(&current)
            .join("reader-jobs")
            .join(&job.id)
            .join("prompt.txt"),
    )
    .await
    .unwrap();
    assert!(prompt.contains("You are Lysilogos"));
    assert!(prompt.contains("--- PAGE 10 ---"));
}

#[tokio::test]
async fn invalid_model_output_becomes_a_durable_retryable_failure() {
    let mut output = cut();
    output.paragraphs[0].segments[0].text = "A fabricated result.".to_owned();
    let (_directory, state, current, _) = fixture(&serde_json::to_value(output).unwrap()).await;
    let (_, Json(job)) = start_job(
        State(state.clone()),
        AxumPath(current.to_string()),
        Json(StartJob {
            provider: AnalysisProvider::Claude,
            action: ToolAction::Supercut {
                format: CutFormat::TenParagraphs,
            },
        }),
    )
    .await
    .unwrap();
    let tools = wait_for_job(&state, &current, &job).await;
    assert_eq!(tools.jobs[0].status, ToolJobStatus::Failed);
    assert!(
        tools.jobs[0]
            .error
            .as_ref()
            .unwrap()
            .contains("source passage")
    );
    assert!(tools.supercuts.is_empty());
}

#[tokio::test]
async fn references_persist_link_and_delete_without_a_model() {
    let (_directory, state, current, cited) = fixture(&serde_json::json!({})).await;
    let saved = add_reference(&state, &current).await;
    assert_eq!(
        state
            .store
            .load_reader_tools(&current)
            .await
            .unwrap()
            .references
            .len(),
        1
    );
    let Json(tools) = link_reference(
        State(state.clone()),
        AxumPath((current.to_string(), saved.id.clone())),
        Json(LinkReference {
            linked_paper_id: cited.clone(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(tools.references[0].linked_paper_id, Some(cited));
    assert!(
        link_reference(
            State(state.clone()),
            AxumPath((current.to_string(), saved.id.clone())),
            Json(LinkReference {
                linked_paper_id: current.clone()
            })
        )
        .await
        .is_err()
    );
    assert_eq!(
        remove_reference(
            State(state.clone()),
            AxumPath((current.to_string(), saved.id))
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    assert!(
        state
            .store
            .load_reader_tools(&current)
            .await
            .unwrap()
            .references
            .is_empty()
    );
}

#[tokio::test]
async fn search_records_unavailable_papers_without_inventing_a_download() {
    let candidate = serde_json::json!({"title":"Mechanisms","landing_url":null,"pdf_url":null,"explanation":"No public copy found."});
    let (_directory, state, current, _) = fixture(&candidate).await;
    let saved = add_reference(&state, &current).await;
    let (_, Json(job)) = start_job(
        State(state.clone()),
        AxumPath(current.to_string()),
        Json(StartJob {
            provider: AnalysisProvider::Claude,
            action: ToolAction::FindReference {
                reference_id: saved.id,
            },
        }),
    )
    .await
    .unwrap();
    let tools = wait_for_job(&state, &current, &job).await;
    assert_eq!(tools.jobs[0].status, ToolJobStatus::Completed);
    assert_eq!(
        tools.references[0].candidate.as_ref().unwrap().title,
        "Mechanisms"
    );
    assert!(tools.references[0].linked_paper_id.is_none());
    let arguments = tokio::fs::read_to_string(
        state
            .store
            .paper_dir(&current)
            .join("reader-jobs")
            .join(&job.id)
            .join("arguments.txt"),
    )
    .await
    .unwrap();
    assert!(arguments.contains("WebSearch,WebFetch"));
}

#[tokio::test]
async fn reference_search_rejects_private_downloads_before_connecting() {
    let candidate = serde_json::json!({"title":"Mechanisms","landing_url":null,"pdf_url":"http://127.0.0.1/paper.pdf","explanation":"Untrusted candidate"});
    let (_directory, state, current, _) = fixture(&candidate).await;
    let saved = add_reference(&state, &current).await;
    let (_, Json(job)) = start_job(
        State(state.clone()),
        AxumPath(current.to_string()),
        Json(StartJob {
            provider: AnalysisProvider::Claude,
            action: ToolAction::FindReference {
                reference_id: saved.id,
            },
        }),
    )
    .await
    .unwrap();
    let tools = wait_for_job(&state, &current, &job).await;
    assert_eq!(tools.jobs[0].status, ToolJobStatus::Failed);
    assert!(tools.references[0].candidate.is_none());
    assert!(tools.references[0].linked_paper_id.is_none());
}

#[tokio::test]
async fn connection_reads_both_papers_and_persists_verified_evidence() {
    let current = PaperId::from_relative_path(Path::new("current.pdf"));
    let cited = PaperId::from_relative_path(Path::new("cited.pdf"));
    let output = serde_json::json!({
        "verdict":"qualifies","connector":"The cited paper adds a scope condition.","limitation":"This comparison does not establish independent replication.",
        "evidence":[
            {"paper_id":current,"source_page":1,"quote":source().pages[0].text},
            {"paper_id":cited,"source_page":2,"quote":source().pages[1].text}
        ]
    });
    let (_directory, state, current, cited) = fixture(&output).await;
    let saved = add_reference(&state, &current).await;
    let _ = link_reference(
        State(state.clone()),
        AxumPath((current.to_string(), saved.id.clone())),
        Json(LinkReference {
            linked_paper_id: cited.clone(),
        }),
    )
    .await
    .unwrap();
    let (_, Json(job)) = start_job(
        State(state.clone()),
        AxumPath(current.to_string()),
        Json(StartJob {
            provider: AnalysisProvider::Claude,
            action: ToolAction::ConnectReference {
                reference_id: saved.id,
                question: "Does the cited result support this mechanism?".to_owned(),
            },
        }),
    )
    .await
    .unwrap();
    let tools = wait_for_job(&state, &current, &job).await;
    assert_eq!(tools.jobs[0].status, ToolJobStatus::Completed);
    assert_eq!(
        tools.references[0]
            .connection
            .as_ref()
            .unwrap()
            .evidence
            .len(),
        2
    );
    let directory = state
        .store
        .paper_dir(&current)
        .join("reader-jobs")
        .join(&job.id);
    let prompt = tokio::fs::read_to_string(directory.join("prompt.txt"))
        .await
        .unwrap();
    assert!(prompt.contains(current.as_str()) && prompt.contains(cited.as_str()));
    assert!(
        !tokio::fs::read_to_string(directory.join("arguments.txt"))
            .await
            .unwrap()
            .contains("WebSearch")
    );
}

#[tokio::test]
async fn restart_fails_running_tasks_and_preserves_saved_references() {
    let (_directory, state, current, _) = fixture(&serde_json::json!({})).await;
    let saved = add_reference(&state, &current).await;
    let mut tools = state.store.load_reader_tools(&current).await.unwrap();
    tools.jobs.push(ToolJob {
        id: "interrupted".to_owned(),
        action: ToolAction::FindReference {
            reference_id: saved.id,
        },
        provider: AnalysisProvider::Claude,
        status: ToolJobStatus::Running,
        created_at: Utc::now(),
        error: None,
    });
    state
        .store
        .save_reader_tools(&current, &tools)
        .await
        .unwrap();
    assert!(
        start_job(
            State(state.clone()),
            AxumPath(current.to_string()),
            Json(StartJob {
                provider: AnalysisProvider::Claude,
                action: ToolAction::Supercut {
                    format: CutFormat::SixPages
                },
            })
        )
        .await
        .is_err()
    );
    state.store.initialize().await.unwrap();
    let recovered = state.store.load_reader_tools(&current).await.unwrap();
    assert_eq!(recovered.jobs[0].status, ToolJobStatus::Failed);
    assert_eq!(recovered.references.len(), 1);
}

#[tokio::test]
async fn routes_reject_bad_ids_and_offline_generation() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (_directory, state, current, _) = fixture(&serde_json::json!({})).await;
    let app = super::super::build_router(state.clone(), None);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/papers/not-an-id/reader-tools")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        start_job(
            State(state),
            AxumPath(current.to_string()),
            Json(StartJob {
                provider: AnalysisProvider::Heuristic,
                action: ToolAction::Supercut {
                    format: CutFormat::TenParagraphs
                },
            })
        )
        .await
        .is_err()
    );
}
