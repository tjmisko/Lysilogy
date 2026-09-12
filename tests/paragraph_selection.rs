//! Synthetic PDF shared with the browser regression. Extraction is real and
//! entirely local; the stdout fixture lets Playwright reuse the exact index.
#[tokio::test]
async fn paragraph_selection_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("paragraph.pdf");
    tokio::fs::write(
        &source,
        include_bytes!("fixtures/paragraph-continuation.pdf"),
    )
    .await
    .unwrap();
    let index =
        lysilogy::source_index::load_or_build(&source, &directory.path().join("index"), false)
            .await
            .unwrap();
    let paragraph = index
        .objects
        .paragraph
        .iter()
        .find(|p| {
            String::from_utf16_lossy(
                &index
                    .text
                    .encode_utf16()
                    .skip(p.start)
                    .take(10)
                    .collect::<Vec<_>>(),
            ) == "We compare"
        })
        .unwrap();
    assert_eq!(paragraph.spans.len(), 2, "{index:#?}");
    let selected = paragraph
        .spans
        .iter()
        .map(|span| {
            String::from_utf16_lossy(
                &index
                    .text
                    .encode_utf16()
                    .skip(span.start)
                    .take(span.end - span.start)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        selected,
        "We compare our single-view estimator to the off-the-shelf UmeTrack baseline [11], which was used to provide the original annotations. Our classifier improves the action recognition accuracy of hand poses estimated with UmeTrack."
    );
    assert!(index.gaps.is_empty());
    println!(
        "READING_INDEX_JSON {}",
        serde_json::to_string(&index).unwrap()
    );
}
