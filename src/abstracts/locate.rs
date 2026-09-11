use super::{
    AbstractCandidate, heading_remainder, normalize, selected_text, source_lines, title_key,
};
use crate::domain::ExtractedPaper;

#[must_use]
pub fn target_start_index(paper: &ExtractedPaper) -> usize {
    let title = title_key(&paper.metadata.title);
    if title.len() < 8 {
        return 0;
    }
    paper
        .pages
        .iter()
        .position(|page| {
            title_key(&page.text).contains(&title)
                && !page.text.contains("Recommended Citation")
                && !page.text.contains("This Article is brought to you")
        })
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
            .find(|line| !line.running_matter && line.section_heading)
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
    // Unlabeled candidates are proposed conservatively and require a separate boundary review.
    let title = title_key(&paper.metadata.title);
    let title_line = lines
        .iter()
        .position(|line| title_key(&line.text) == title)?;
    let start = lines
        .iter()
        .skip(title_line + 1)
        .find(|line| {
            !line.running_matter
                && !line.section_heading
                && line.text.split_whitespace().count() >= 8
                && !line.text.contains('@')
                && !line.text.contains("arXiv:")
                && !line.text.contains("http")
        })?
        .id;
    if lines[title_line + 1..start]
        .iter()
        .any(|line| line.section_heading)
    {
        return None;
    }
    let end = lines
        .iter()
        .skip(start + 1)
        .find(|line| line.section_heading)?
        .id
        .checked_sub(1)?;
    let text = selected_text(&lines, start, end)?;
    let count = text.split_whitespace().count();
    (40..=800).contains(&count).then_some(AbstractCandidate {
        start_line: start,
        end_line: end,
        text,
    })
}
