use super::*;
use crate::domain::{DocumentLayout, ExtractedPage, PaperMetadata};
fn paper(pages: &[&str]) -> ExtractedPaper {
    ExtractedPaper {
        metadata: PaperMetadata {
            title: "Target discovery".to_owned(),
            ..PaperMetadata::default()
        },
        pages: pages
            .iter()
            .enumerate()
            .map(|(i, text)| ExtractedPage {
                number: u32::try_from(i + 1).unwrap(),
                text: (*text).to_owned(),
            })
            .collect(),
        layout: DocumentLayout::default(),
    }
}
#[test]
fn structured_abstract_keeps_internal_headings_and_stops_at_body() {
    let p = paper(&[
        "Abstract\nBackground\nThe prior methods leave a specific gap in our understanding.\nMethods\nWe measured a population using a controlled intervention.\nResults\nThe effect persisted under the stated assumptions.\nConclusions\nThese findings qualify the earlier account.\n1 Introduction\nBody text.",
    ]);
    let result = extract(&p);
    assert_eq!(result.status, AbstractStatus::Accepted);
    let text = result.text.unwrap();
    assert!(text.contains("Methods"));
    assert!(text.ends_with("earlier account."));
}
#[test]
fn verifies_inline_multpage_and_target_article() {
    let p = paper(&[
        "Abstract\nThis abstract belongs to an adjacent article and must be ignored.\nIntroduction",
        "Target discovery\nAbstract—Our central result explains why the measured response changes.",
        "The conclusion retains a qualification across page boundaries.\nKeywords: inference",
    ]);
    let result = extract(&p);
    assert_eq!(result.status, AbstractStatus::Accepted);
    assert_eq!(result.start_page, Some(2));
    assert_eq!(result.end_page, Some(3));
}
#[test]
fn independently_rejects_truncation_contamination_and_rewriting() {
    let p = paper(&[
        "Abstract\nThe first sentence states a sufficiently detailed central result.\nThe second sentence limits its scope to the observed population.\nIntroduction\nThe body provides a separate argument.",
    ]);
    let original = locate(&p).unwrap();
    let mut bad = original.clone();
    bad.end_line -= 1;
    bad.text = "The first sentence states a sufficiently detailed central result.".to_owned();
    assert_eq!(verify(&p, Some(&bad)).status, AbstractStatus::NeedsReview);
    bad = original.clone();
    bad.start_line += 1;
    bad.text = "The second sentence limits its scope to the observed population.".to_owned();
    assert_eq!(verify(&p, Some(&bad)).status, AbstractStatus::NeedsReview);
    bad = original.clone();
    bad.text = bad.text.replace("limits", "expands");
    assert_eq!(verify(&p, Some(&bad)).status, AbstractStatus::NeedsReview);
    bad = original;
    bad.end_line += 2;
    assert_eq!(verify(&p, Some(&bad)).status, AbstractStatus::NeedsReview);
}
#[test]
fn absence_and_unclear_boundaries_do_not_become_abstracts() {
    assert_eq!(
        extract(&paper(&[
            "Introduction\nA paper without an authored abstract."
        ]))
        .status,
        AbstractStatus::NotFound
    );
    assert_eq!(extract(&paper(&[""])).status, AbstractStatus::NeedsOcr);
    assert_eq!(
        extract(&paper(&[
            "Abstract\nSome sufficiently long text without a demonstrable ending."
        ]))
        .status,
        AbstractStatus::NeedsReview
    );
}
#[test]
fn allows_ligatures_but_not_scientific_hyphen_changes() {
    let p = paper(&[
        "Abstract\nThe ﬁnite-sample estimator preserves the sign of the measured effect.\nIntroduction",
    ]);
    let mut candidate = locate(&p).unwrap();
    candidate.text = candidate.text.replace('ﬁ', "fi");
    assert_eq!(
        verify(&p, Some(&candidate)).status,
        AbstractStatus::Accepted
    );
    candidate.text = candidate.text.replace("finite-sample", "finitesample");
    assert_eq!(
        verify(&p, Some(&candidate)).status,
        AbstractStatus::NeedsReview
    );
}
