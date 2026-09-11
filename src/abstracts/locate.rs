use super::{
    AbstractCandidate, boundary, heading_remainder, normalize, selected_text, source_lines,
};
use crate::domain::ExtractedPaper;

#[must_use]
pub fn target_start_index(paper: &ExtractedPaper) -> usize {
    let title = normalize(&paper.metadata.title).to_lowercase();
    if title.len() < 8 {
        return 0;
    }
    paper
        .pages
        .iter()
        .position(|page| normalize(&page.text).to_lowercase().contains(&title))
        .unwrap_or(0)
}

#[must_use]
pub fn locate(paper: &ExtractedPaper) -> Option<AbstractCandidate> {
    let lines = source_lines(paper);
    for line in &lines {
        let Some(inline) = heading_remainder(&line.text) else {
            continue;
        };
        let start = if inline.is_empty() {
            line.id + 1
        } else {
            line.id
        };
        let end = lines
            .iter()
            .skip(start)
            .find(|line| !line.running_matter && boundary(&line.text))
            .map_or(lines.len(), |line| line.id)
            .checked_sub(1)?;
        let text = selected_text(&lines, start, end)?;
        if normalize(&text).chars().count() >= 30 {
            return Some(AbstractCandidate {
                start_line: start,
                end_line: end,
                text,
            });
        }
    }
    None
}
