use super::{
    AbstractCandidate, AbstractResult, AbstractStatus, heading_remainder, normalize, selected_text,
    source_lines,
};
use crate::domain::ExtractedPaper;

/// Checks source, beginning, and end independently; it never calls the locator.
#[must_use]
pub fn verify(paper: &ExtractedPaper, candidate: Option<&AbstractCandidate>) -> AbstractResult {
    let lines = source_lines(paper);
    let mut result = AbstractResult {
        schema_version: 1,
        source_fingerprint: super::source_fingerprint(paper),
        status: AbstractStatus::NotFound,
        text: None,
        start_page: None,
        end_page: None,
        candidate: candidate.cloned(),
        checks: Vec::new(),
        review: None,
    };
    let Some(candidate) = candidate else {
        if lines.iter().all(|line| line.text.trim().is_empty()) {
            result.status = AbstractStatus::NeedsOcr;
        }
        return result;
    };
    result.status = AbstractStatus::NeedsReview;
    let Some(source) = selected_text(&lines, candidate.start_line, candidate.end_line) else {
        result.checks.push("invalid_source_range".to_owned());
        return result;
    };
    let text = normalize(&candidate.text);
    if !(30..=12_000).contains(&text.chars().count()) {
        result.checks.push("invalid_length".to_owned());
    }
    let content = |value: &str| {
        normalize(value)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
    };
    if content(&text) != content(&source) {
        result.checks.push("source_text_mismatch".to_owned());
    }
    let first = &lines[candidate.start_line];
    let previous = lines[..candidate.start_line]
        .iter()
        .rev()
        .find(|line| !line.running_matter);
    let starts_at_heading = heading_remainder(&first.text).is_some()
        || previous.is_some_and(|line| heading_remainder(&line.text).is_some_and(str::is_empty))
        || verified_unlabeled_opening(paper, &lines, candidate);
    if !starts_at_heading {
        result.checks.push("start_boundary_unconfirmed".to_owned());
    }
    let after = lines[candidate.end_line + 1..]
        .iter()
        .find(|line| !line.running_matter);
    if !after.is_some_and(|line| line.section_heading) {
        result.checks.push("end_boundary_unconfirmed".to_owned());
    }
    if lines[candidate.start_line..=candidate.end_line]
        .iter()
        .any(|line| !line.running_matter && line.section_heading)
    {
        result
            .checks
            .push("body_or_keywords_in_abstract".to_owned());
    }
    if result.checks.is_empty() {
        result.status = AbstractStatus::Accepted;
        // Keep authored paragraphs/subheadings; normalize each extraction line for display.
        result.text = Some(candidate.text.trim().to_owned());
        result.start_page = Some(first.page);
        result.end_page = Some(lines[candidate.end_line].page);
        result.checks = vec![
            "source_text_verified".to_owned(),
            "boundaries_verified".to_owned(),
        ];
    }
    result
}

// An unlabeled opening needs independent front-matter and typographic evidence.
// A title followed by an arbitrary introductory paragraph is insufficient.
fn verified_unlabeled_opening(
    paper: &ExtractedPaper,
    lines: &[super::SourceLine],
    candidate: &AbstractCandidate,
) -> bool {
    if paper.layout.pages.is_empty() || candidate.start_line == 0 {
        return false;
    }
    let Some(first) = lines.get(candidate.start_line) else {
        return false;
    };
    let Some(after) = lines.get(candidate.end_line + 1) else {
        return false;
    };
    if first.page != after.page || !after.section_heading {
        return false;
    }
    let before = &lines[..candidate.start_line];
    let title = super::title_key(&paper.metadata.title);
    let has_title = before
        .iter()
        .any(|line| super::title_key(&line.text) == title);
    let has_authors = paper
        .metadata
        .authors
        .iter()
        .all(|author| before.iter().any(|line| line.text.contains(author)));
    let preview = candidate.text.to_lowercase();
    let words = preview.split_whitespace().count();
    has_title
        && !paper.metadata.authors.is_empty()
        && has_authors
        && (60..=600).contains(&words)
        && (before
            .iter()
            .any(|line| line.text.contains('@') || line.text.contains("http"))
            || after.text.to_lowercase().replace(' ', "") == "tableofcontents")
        && !before
            .iter()
            .skip_while(|line| super::title_key(&line.text) != title)
            .skip(1)
            .any(|line| line.section_heading)
        && [
            "this paper",
            "this article",
            "we present",
            "we propose",
            "we show",
            "this study",
        ]
        .iter()
        .any(|cue| preview.contains(cue))
}
