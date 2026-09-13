use super::parse_pages;
use crate::layout::{parse_bbox_layout, parse_verbatim_bbox_layout};

fn word(text: &str, coordinates: &str) -> String {
    format!("<word {coordinates}>{text}</word>")
}

fn page(content: &str) -> String {
    format!(
        r#"<page width="600" height="800"><flow><block xMin="0"><line xMin="0">{content}</line></block></flow></page>"#
    )
}

fn valid_word(text: &str) -> String {
    word(text, r#"xMin="40" yMin="100" xMax="90" yMax="110""#)
}

#[test]
fn should_preserve_every_valid_page_token_when_one_page_has_invalid_word_geometry() {
    let first = page(&format!("{}{}", valid_word("𝑋̂"), valid_word("a\u{1}b")));
    let last = page(&valid_word("Final"));
    for coordinates in [
        r#"xMin="90" yMin="100" xMax="89" yMax="110""#,
        r#"xMin="40" yMin="111" xMax="90" yMax="110""#,
        r#"xMin="NaN" yMin="100" xMax="90" yMax="110""#,
        r#"xMin="40" yMin="100" xMax="inf" yMax="110""#,
        r#"xMin="40" yMin="invalid" xMax="90" yMax="110""#,
        r#"xMin="40" yMin="100" xMax="90""#,
        r#"xMin="40" xMin="41" yMin="100" xMax="90" yMax="110""#,
    ] {
        let middle = page(&format!(
            "{}{}",
            valid_word("WithholdEntirePage"),
            word("̂", coordinates)
        ));
        let xml = format!("<doc>{first}{middle}{last}</doc>");
        let parsed = parse_pages(&xml).unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(parsed[1].failure.is_some(), "{coordinates}");
        assert!(parsed[1].page.tokens.is_empty());
        assert!(parsed[1].page.sentences.is_empty());
        assert_eq!(parsed[1].page.number, 2);
        assert_eq!(parsed[1].page.width.to_bits(), 600_f32.to_bits());
        assert_eq!(parsed[1].page.height.to_bits(), 800_f32.to_bits());
        for at in [0, 2] {
            // Compare sentence identities at their original page positions too.
            let clean = format!("<doc>{first}{}{last}</doc>", page(""));
            let expected = parse_verbatim_bbox_layout(&clean).unwrap();
            assert_eq!(
                serde_json::to_value(&parsed[at].page).unwrap(),
                serde_json::to_value(&expected.pages[at]).unwrap()
            );
            assert!(parsed[at].failure.is_none());
        }
        if !coordinates.contains("xMin=\"41\"") {
            assert!(
                parse_bbox_layout(&xml).is_err(),
                "saved anchor parser must remain strict"
            );
            assert!(parse_verbatim_bbox_layout(&xml).is_err());
        }
    }
}

#[test]
fn should_preserve_existing_verbatim_layouts_when_real_native_fixtures_are_valid() {
    for xml in [
        include_str!("../../../tests/fixtures/debate-frontmatter.html"),
        include_str!("../../../tests/fixtures/hierarchy-introduction.html"),
        include_str!("../../../tests/fixtures/goodhart-paragraphs.html"),
    ] {
        let strict = parse_verbatim_bbox_layout(xml).unwrap();
        let isolated = parse_pages(xml).unwrap();
        assert_eq!(isolated.len(), strict.pages.len());
        for (actual, expected) in isolated.iter().zip(strict.pages) {
            assert!(actual.failure.is_none(), "{:?}", actual.failure);
            assert_eq!(
                serde_json::to_value(&actual.page).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }
}

#[test]
fn should_reject_document_when_page_boundaries_or_dimensions_are_untrustworthy() {
    let valid = page(&valid_word("Valid"));
    for xml in [
        format!("<doc>{valid}"),
        format!("<doc>{valid}</page></doc>"),
        format!("<doc>{valid}<page width=\"600\" height=\"800\">{valid}</page></doc>"),
        format!("<doc>{valid}<page width=\"600\" height=\"800\"></doc>"),
        format!("<html><body>{valid}</body></html>"),
        format!("<doc>{valid}</doc><doc>{valid}</doc>"),
        format!("<html><body><doc>{valid}</doc></body><head></head></html>"),
        format!("<html><head></head><head></head><body><doc>{valid}</doc></body></html>"),
        format!("<doc>{valid}</doc>unframed trailing text"),
        format!("<doc>{valid}<page width=\"600\" width=\"601\" height=\"800\"></page></doc>"),
        format!(
            "<doc>{valid}<page width=\"600\" height=\"800\"><word xMin=\"unclosed></page></doc>"
        ),
    ] {
        assert!(parse_pages(&xml).is_err(), "{xml}");
    }
    for dimension in ["NaN", "inf", "0", "-1", "invalid"] {
        for field in ["width", "height"] {
            let broken = valid.replace(
                &format!(
                    "{field}=\"{}\"",
                    if field == "width" { "600" } else { "800" }
                ),
                &format!("{field}=\"{dimension}\""),
            );
            assert!(parse_pages(&format!("<doc>{valid}{broken}</doc>")).is_err());
        }
    }
}

#[test]
fn should_isolate_malformed_local_nesting_when_the_page_frame_is_complete() {
    let valid = page(&valid_word("Valid"));
    for body in [
        "<line xMin=\"0\"><word xMin=\"0\"></line></word>",
        "<line xMin=\"0\"><line xMin=\"0\"></line></line>",
        "<line xMin=\"0\">unwrapped text</line>",
        "<line xMin=\"0\"></line></block>",
        "<!-- <line xMin=\"0\"></line> -->",
        "<line xMin=\"0\"><word xMin=\"0\" /></line>",
        "<line xMin=\"0\"><word xMin=\"0\">unfinished",
    ] {
        let xml =
            format!("<doc>{valid}<page width=\"600\" height=\"800\">{body}</page>{valid}</doc>");
        let pages = parse_pages(&xml).unwrap();
        assert_eq!(pages.len(), 3);
        assert!(pages[0].failure.is_none());
        assert!(pages[1].failure.is_some(), "{body}");
        assert!(pages[1].page.tokens.is_empty());
        assert!(pages[2].failure.is_none());
        assert_eq!(pages[2].page.number, 3);
    }
}

#[test]
fn should_ignore_page_lookalikes_when_they_are_inside_document_metadata_or_comments() {
    let xml = format!(
        r#"<?xml version="1.0"?><!DOCTYPE html PUBLIC "x" "https://example.invalid/no-fetch"><html><head><title>&lt;page width="1"&gt;</title></head><body><!-- <page width="1" height="1"><line xMin="1"> --><doc>{}</doc></body></html>"#,
        page(&valid_word("OnlyPage"))
    );
    let pages = parse_pages(&xml).unwrap();
    assert_eq!(pages.len(), 1);
    assert!(pages[0].failure.is_none());
    assert_eq!(pages[0].page.tokens[0].text, "OnlyPage");
}
