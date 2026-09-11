use super::{
    AbstractCandidate, AbstractResult, AbstractStatus, boundary, heading_remainder, normalize,
    selected_text, source_lines,
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
    if text != normalize(&source) {
        result.checks.push("source_text_mismatch".to_owned());
    }
    let first = &lines[candidate.start_line];
    let previous = lines[..candidate.start_line]
        .iter()
        .rev()
        .find(|line| !line.running_matter);
    let starts_at_heading = heading_remainder(&first.text).is_some()
        || previous.is_some_and(|line| heading_remainder(&line.text).is_some_and(str::is_empty));
    if !starts_at_heading {
        result.checks.push("start_boundary_unconfirmed".to_owned());
    }
    let after = lines[candidate.end_line + 1..]
        .iter()
        .find(|line| !line.running_matter);
    if !after.is_some_and(|line| boundary(&line.text)) {
        result.checks.push("end_boundary_unconfirmed".to_owned());
    }
    if lines[candidate.start_line..=candidate.end_line]
        .iter()
        .any(|line| !line.running_matter && boundary(&line.text))
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
