#[tokio::test]
async fn cursor_objects_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cursor-objects.pdf");
    tokio::fs::write(&source, include_bytes!("fixtures/cursor-objects.pdf"))
        .await
        .unwrap();
    let index =
        lysilogy::source_index::load_or_build(&source, &directory.path().join("index"), false)
            .await
            .unwrap();
    assert_eq!(index.figures.len(), 2, "{:#?}", index.figures);
    let text = |spans: &[lysilogy::source_index::TextRange]| {
        spans
            .iter()
            .map(|s| {
                String::from_utf16_lossy(
                    &index
                        .text
                        .encode_utf16()
                        .skip(s.start)
                        .take(s.end - s.start)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let figure = index.figures.iter().find(|f| f.kind == "figure").unwrap();
    assert!(text(&figure.spans).contains("Step 1:"), "{figure:#?}");
    assert!(
        text(&figure.spans).contains("Action classifier"),
        "{figure:#?}"
    );
    assert!(!text(&figure.spans).contains("results in this column"));
    let table = index.figures.iter().find(|f| f.kind == "table").unwrap();
    assert!(text(&table.spans).contains("54.70"), "{table:#?}");
    assert!(!text(&table.spans).contains("paragraph after"));
    println!(
        "CURSOR_INDEX_JSON {}",
        serde_json::to_string(&index).unwrap()
    );
}
