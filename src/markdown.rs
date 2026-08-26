use std::fmt::Write;

use crate::domain::ExtractedPaper;

#[derive(Debug, Eq, PartialEq)]
enum Block {
    Heading(String),
    Paragraph(String),
    ListItem(String),
    Aside(String),
}

#[must_use]
pub fn render_source(paper: &ExtractedPaper) -> String {
    let mut output = format!("# {}\n\n", escape_text(&paper.metadata.title));
    write_metadata(&mut output, paper);
    output.push_str(
        "> **Conversion note:** Best-effort Markdown reconstructed from the PDF text layer. \
         Line wrapping and headings are inferred; use the page markers to verify quotations against the PDF.\n",
    );

    for page in &paper.pages {
        let _ = write!(output, "\n---\n\n## PDF page {}\n\n", page.number);
        for block in parse_page(&page.text, &paper.metadata.title) {
            write_block(&mut output, &block);
        }
    }
    output.trim_end().to_owned() + "\n"
}

fn write_metadata(output: &mut String, paper: &ExtractedPaper) {
    if !paper.metadata.authors.is_empty() {
        let _ = writeln!(
            output,
            "**Authors:** {}  ",
            escape_text(&paper.metadata.authors.join(", "))
        );
    }
    if let Some(year) = paper.metadata.year {
        let _ = writeln!(output, "**Year:** {year}  ");
    }
    let pages = paper
        .metadata
        .page_count
        .unwrap_or_else(|| u32::try_from(paper.pages.len()).unwrap_or(u32::MAX));
    let _ = writeln!(output, "**PDF pages:** {pages}\n");
}

fn parse_page(text: &str, document_title: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut first_content = true;
    for raw_line in text.lines() {
        let line = compact(raw_line);
        if line.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks);
            continue;
        }
        if first_content && same_title(&line, document_title) {
            first_content = false;
            continue;
        }
        first_content = false;
        if let Some(block) = special_block(&line) {
            flush_paragraph(&mut paragraph, &mut blocks);
            blocks.push(block);
            continue;
        }
        if begins_new_paragraph(&paragraph, &line) {
            flush_paragraph(&mut paragraph, &mut blocks);
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(&line);
    }
    flush_paragraph(&mut paragraph, &mut blocks);
    blocks
}

fn special_block(line: &str) -> Option<Block> {
    let lowered = line.to_ascii_lowercase();
    if lowered.contains("volume ")
        && (lowered.contains("communications of the acm") || lowered.contains("journal"))
    {
        return Some(Block::Aside(escape_text(line)));
    }
    if let Some((label, value)) = line.split_once(':')
        && is_metadata_label(label)
    {
        return Some(Block::Paragraph(format!(
            "**{}:** {}",
            readable_label(label),
            escape_text(value.trim())
        )));
    }
    if let Some(item) = bullet_item(line) {
        return Some(Block::ListItem(escape_text(item)));
    }
    if looks_like_heading(line) {
        return Some(Block::Heading(readable_heading(line)));
    }
    None
}

fn flush_paragraph(paragraph: &mut String, blocks: &mut Vec<Block>) {
    if !paragraph.is_empty() {
        blocks.push(Block::Paragraph(escape_text(paragraph)));
        paragraph.clear();
    }
}

fn write_block(output: &mut String, block: &Block) {
    match block {
        Block::Heading(value) => {
            let _ = write!(output, "### {value}\n\n");
        }
        Block::Paragraph(value) => {
            let _ = write!(output, "{value}\n\n");
        }
        Block::ListItem(value) => {
            let _ = write!(output, "- {value}\n\n");
        }
        Block::Aside(value) => {
            let _ = write!(output, "> {value}\n\n");
        }
    }
}

fn begins_new_paragraph(current: &str, next: &str) -> bool {
    let ends_sentence = current
        .trim_end_matches(['\'', '"', ')', ']'])
        .ends_with(['.', '?', '!']);
    let starts_sentence = next
        .trim_start_matches(['\'', '"', '(', '['])
        .chars()
        .next()
        .is_some_and(|character| character.is_uppercase() || character.is_ascii_digit());
    ends_sentence && starts_sentence
}

fn looks_like_heading(line: &str) -> bool {
    let trimmed = line.trim_matches(|character: char| !character.is_alphanumeric());
    let words = trimmed.split_whitespace().count();
    if !(1..=14).contains(&words) || trimmed.chars().count() > 110 {
        return false;
    }
    let lowered = trimmed.to_ascii_lowercase();
    let known = [
        "abstract",
        "introduction",
        "background",
        "methods",
        "methodology",
        "results",
        "discussion",
        "limitations",
        "conclusion",
        "conclusions",
        "references",
        "appendix",
        "editor",
    ]
    .contains(&lowered.as_str());
    known || is_numbered_heading(trimmed) || is_uppercase_heading(trimmed)
}

fn is_numbered_heading(line: &str) -> bool {
    line.split_whitespace().next().is_some_and(|first| {
        first
            .trim_end_matches('.')
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|value| value.is_ascii_digit()))
    })
}

fn is_uppercase_heading(line: &str) -> bool {
    let letters = line.chars().filter(char::is_ascii_alphabetic).count();
    letters >= 3
        && line
            .chars()
            .filter(char::is_ascii_alphabetic)
            .all(|character| character.is_ascii_uppercase())
}

fn bullet_item(line: &str) -> Option<&str> {
    line.strip_prefix("• ")
        .or_else(|| line.strip_prefix("· "))
        .or_else(|| line.strip_prefix("- "))
        .or_else(|| line.strip_prefix("* "))
}

fn is_metadata_label(value: &str) -> bool {
    matches!(
        compact(value).to_ascii_lowercase().as_str(),
        "keywords"
            | "key words"
            | "key words and phrases"
            | "cr categories"
            | "jel classification"
            | "doi"
    )
}

fn readable_label(value: &str) -> String {
    let value = compact(value);
    let mut characters = value.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

fn readable_heading(value: &str) -> String {
    let trimmed = value.trim_matches(|character: char| !character.is_alphanumeric());
    if is_uppercase_heading(trimmed) {
        readable_label(&trimmed.to_ascii_lowercase())
    } else {
        readable_label(trimmed)
    }
}

fn same_title(left: &str, right: &str) -> bool {
    let left = canonical_title(left);
    let right = canonical_title(right);
    left == right || left.contains(&right) || right.contains(&left)
}

fn canonical_title(value: &str) -> String {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| {
            let lowered = token.to_ascii_lowercase();
            if lowered.len() > 3 {
                lowered.trim_end_matches('s').to_owned()
            } else {
                lowered
            }
        })
        .collect()
}

fn compact(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

#[cfg(test)]
mod tests {
    use crate::domain::{DocumentLayout, ExtractedPage, PaperMetadata};

    use super::*;

    #[test]
    fn reconstructs_pages_headings_and_paragraphs() {
        let paper = ExtractedPaper {
            metadata: PaperMetadata {
                title: "A Useful Paper".to_owned(),
                authors: vec!["Ada Lovelace".to_owned()],
                year: Some(1843),
                page_count: Some(1),
                subject: None,
            },
            pages: vec![ExtractedPage {
                number: 1,
                text: "A Useful Paper\nABSTRACT\nFirst wrapped\nline. A new paragraph\nstarts here.\nKeywords: engines, notation"
                    .to_owned(),
            }],
            layout: DocumentLayout::default(),
        };
        let markdown = render_source(&paper);
        assert!(markdown.starts_with("# A Useful Paper"));
        assert!(markdown.contains("## PDF page 1"));
        assert!(markdown.contains("### Abstract"));
        assert!(markdown.contains("First wrapped line."));
        assert!(markdown.contains("**Keywords:** engines, notation"));
    }

    #[test]
    fn neutralizes_html_from_pdf_text() {
        assert_eq!(
            escape_text("<script>*x*</script>"),
            "&lt;script&gt;\\*x\\*&lt;/script&gt;"
        );
    }
}
