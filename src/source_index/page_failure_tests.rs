use super::*;

fn native_page(text: &str, coordinate: &str) -> String {
    format!(
        r#"<page width="600" height="800"><flow><block xMin="40"><line xMin="40"><word xMin="40" yMin="100" xMax="90" yMax="110">{text}</word><word xMin="100" yMin="100" xMax="{coordinate}" yMax="110">second</word></line></block><block xMin="40"><line xMin="40"><word xMin="40" yMin="130" xMax="90" yMax="140">Tail</word></line></block></flow></page>"#
    )
}

fn failed_document() -> native::NativePages {
    native::parse(&format!(
        "<doc>{}</doc>",
        native_page("RejectedNative", "99")
    ))
    .unwrap()
}

#[test]
fn should_retain_page_identity_and_block_geometry_when_one_native_page_is_rejected() {
    let valid = native_page("Preserved", "160");
    let reference = native::parse(&format!("<doc>{valid}{valid}{valid}</doc>")).unwrap();
    let xml = format!(
        "<doc>{valid}{}{valid}</doc>",
        native_page("RejectedNative", "99")
    );
    let actual = native::parse(&xml).unwrap();
    assert_eq!(actual.pages.len(), 3);
    assert_eq!(actual.gaps.len(), 1);
    assert_eq!(actual.gaps[0].page, 2);
    assert!(actual.gaps[0].reason.contains("ordered finite coordinates"));
    assert_eq!(actual.pages[1].provenance, Provenance::Unavailable);
    assert!(actual.pages[1].words.is_empty());
    for at in [0, 2] {
        let expected = &reference.pages[at];
        let page = &actual.pages[at];
        assert_eq!(page.number, expected.number);
        assert_eq!(page.width.to_bits(), expected.width.to_bits());
        assert_eq!(page.height.to_bits(), expected.height.to_bits());
        assert_eq!(page.words.len(), expected.words.len());
        for (word, original) in page.words.iter().zip(&expected.words) {
            assert_eq!(word.text, original.text);
            assert_eq!(word.line, original.line);
            assert_eq!(word.block, original.block);
            assert_eq!(word.rect, original.rect);
        }
        assert_eq!(page.words[0].block, 1);
        assert_eq!(page.words[2].block, 2);
    }
    let mut index = assemble_with_native_failures(&actual.pages, &[2]);
    index.gaps.extend(actual.gaps);
    assert!(!index.text.contains("RejectedNative"));
    assert_eq!(index.pages[1].start, index.pages[1].end);
    assert!(index.tokens.iter().all(|token| token.page != 2));
    assert_eq!(index.pages[2].number, 3);
    assert_eq!(index.gaps.len(), 1);
}

#[test]
fn should_preserve_native_failure_diagnostic_when_local_ocr_replaces_the_failed_page() {
    let mut native = failed_document();
    let original_gap = serde_json::to_value(&native.gaps[0]).unwrap();
    let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n1\t1\t0\t0\t0\t0\t0\t0\t1200\t1600\t-1\t\n5\t1\t1\t1\t1\t1\t100\t200\t80\t20\t95\tRecognized\n";
    let recognized = ocr::parse_tsv(tsv, &native.pages[0]);
    apply_ocr_result(&mut native.pages[0], recognized, &mut native.gaps);
    assert_eq!(native.pages[0].provenance, Provenance::Ocr);
    assert_eq!(native.pages[0].words[0].text, "Recognized");
    assert_eq!(
        native.pages[0].words[0].rect,
        TextRect {
            x_min: 50.0,
            y_min: 100.0,
            x_max: 90.0,
            y_max: 110.0
        }
    );
    assert_eq!(serde_json::to_value(&native.gaps[0]).unwrap(), original_gap);
    let mut index = assemble_with_native_failures(&native.pages, &[1]);
    index.gaps.extend(native.gaps);
    assert_eq!(index.pages[0].provenance, Provenance::Ocr);
    assert_eq!(index.tokens[0].provenance, Provenance::Ocr);
    assert!(!index.text.contains("RejectedNative"));
    assert_eq!(index.gaps.len(), 1);
}

#[test]
fn should_leave_page_unavailable_with_both_diagnostics_when_local_ocr_fails_or_is_empty() {
    for empty in [false, true] {
        let mut native = failed_document();
        let original_gap = serde_json::to_value(&native.gaps[0]).unwrap();
        let result = if empty {
            Ok(SourcePage {
                provenance: Provenance::Ocr,
                ..native.pages[0].clone()
            })
        } else {
            Err(Error::Task("fixture OCR unavailable".into()))
        };
        apply_ocr_result(&mut native.pages[0], result, &mut native.gaps);
        assert_eq!(native.pages[0].provenance, Provenance::Unavailable);
        assert!(native.pages[0].words.is_empty());
        assert_eq!(native.gaps.len(), 2);
        assert_eq!(serde_json::to_value(&native.gaps[0]).unwrap(), original_gap);
        assert!(native.gaps[1].reason.contains(if empty {
            "No additional readable text"
        } else {
            "fixture OCR unavailable"
        }));
    }
}

#[test]
fn should_keep_prose_separate_when_an_unavailable_native_failure_interrupts_it() {
    let body = |text: &str| {
        format!(
            r#"<page width="600" height="800"><line xMin="40"><word xMin="40" yMin="100" xMax="440" yMax="110">{text}</word></line></page>"#
        )
    };
    let xml = format!(
        "<doc>{}{}{}</doc>",
        body("The preceding argument continues"),
        native_page("RejectedNative", "99"),
        body("with an uncertain missing intermediate derivation.")
    );
    let native = native::parse(&xml).unwrap();
    let index = assemble_with_native_failures(&native.pages, &[2]);
    assert_eq!(index.objects.paragraph.len(), 2);
    assert!(
        index
            .objects
            .paragraph
            .iter()
            .all(|paragraph| paragraph.spans.is_empty())
    );
    assert!(index.text.contains("continues\n\nwith"));
    // This guard applies only to newly isolated native failures. Existing cache
    // schema and successful legacy sparse-page output remain unchanged.
    let legacy = assemble(&native.pages);
    assert!(legacy.text.contains("continues with"));
    assert_eq!(SCHEMA_VERSION, 6);
}

#[test]
fn should_keep_native_block_mapping_on_the_real_page_when_metadata_contains_a_page_lookalike() {
    let xml = format!(
        "<html><head><title>&lt;page width=\"1\"&gt;</title></head><body><!-- <page width=\"1\" height=\"1\"><line xMin=\"0\"> --><doc>{}</doc></body></html>",
        native_page("Preserved", "160")
    );
    let native = native::parse(&xml).unwrap();
    assert!(native.gaps.is_empty());
    assert_eq!(native.pages.len(), 1);
    assert_eq!(native.pages[0].words[0].block, 1);
    assert_eq!(native.pages[0].words[2].block, 2);
}
