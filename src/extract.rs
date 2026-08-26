use std::{ffi::OsString, path::Path};

use tokio::process::Command;

use crate::{
    Result,
    domain::{DocumentLayout, ExtractedPage, ExtractedPaper, PaperMetadata},
    error::Error,
    layout::parse_bbox_layout,
};

const DEFAULT_MAX_EXTRACTED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct PdfExtractor {
    pdftotext: OsString,
    pdfinfo: OsString,
    max_extracted_bytes: usize,
}

impl Default for PdfExtractor {
    fn default() -> Self {
        Self {
            pdftotext: OsString::from("pdftotext"),
            pdfinfo: OsString::from("pdfinfo"),
            max_extracted_bytes: DEFAULT_MAX_EXTRACTED_BYTES,
        }
    }
}

impl PdfExtractor {
    #[must_use]
    pub fn with_programs(pdftotext: impl Into<OsString>, pdfinfo: impl Into<OsString>) -> Self {
        Self {
            pdftotext: pdftotext.into(),
            pdfinfo: pdfinfo.into(),
            max_extracted_bytes: DEFAULT_MAX_EXTRACTED_BYTES,
        }
    }

    pub async fn extract(
        &self,
        source: &Path,
        fallback_metadata: &PaperMetadata,
    ) -> Result<ExtractedPaper> {
        let text_future = self.extract_text(source);
        let layout_future = self.extract_layout(source);
        let metadata_future = self.extract_metadata(source);
        let (text, layout, extracted_metadata) =
            tokio::try_join!(text_future, layout_future, metadata_future)?;

        if text.len() > self.max_extracted_bytes {
            return Err(Error::InvalidRequest(format!(
                "extracted text exceeds {} MiB safety limit for {}",
                self.max_extracted_bytes / (1024 * 1024),
                source.display()
            )));
        }

        let pages = split_pages(&text);
        if pages.iter().all(|page| page.text.trim().is_empty()) {
            return Err(Error::EmptyExtraction(source.to_owned()));
        }
        let mut metadata = fallback_metadata.clone();
        merge_pdf_metadata(&mut metadata, &extracted_metadata);
        metadata.page_count = u32::try_from(layout.pages.len())
            .ok()
            .or_else(|| u32::try_from(pages.len()).ok())
            .or(metadata.page_count);
        Ok(ExtractedPaper {
            metadata,
            pages,
            layout,
        })
    }

    async fn extract_text(&self, source: &Path) -> Result<String> {
        let output = Command::new(&self.pdftotext)
            // The raw content stream preserves the PDF's authored reading
            // order and avoids the character-spacing artifacts produced by
            // visual layout mode on older journal scans.
            .args(["-raw", "-enc", "UTF-8"])
            .arg(source)
            .arg("-")
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| command_io_error(&self.pdftotext, error))?;
        ensure_success(&self.pdftotext, &output)?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .replace('\0', "")
            .replace("\r\n", "\n")
            .replace('\r', "\n"))
    }

    async fn extract_layout(&self, source: &Path) -> Result<DocumentLayout> {
        let output = Command::new(&self.pdftotext)
            .args(["-bbox-layout", "-enc", "UTF-8"])
            .arg(source)
            .arg("-")
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| command_io_error(&self.pdftotext, error))?;
        ensure_success(&self.pdftotext, &output)?;
        if output.stdout.len() > self.max_extracted_bytes {
            return Err(Error::InvalidRequest(format!(
                "PDF coordinate text exceeds {} MiB safety limit for {}",
                self.max_extracted_bytes / (1024 * 1024),
                source.display()
            )));
        }
        parse_bbox_layout(&String::from_utf8_lossy(&output.stdout))
    }

    async fn extract_metadata(&self, source: &Path) -> Result<PaperMetadata> {
        let output = Command::new(&self.pdfinfo)
            .arg(source)
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| command_io_error(&self.pdfinfo, error))?;
        ensure_success(&self.pdfinfo, &output)?;
        Ok(parse_pdf_info(&String::from_utf8_lossy(&output.stdout)))
    }
}

fn command_io_error(program: &std::ffi::OsStr, error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Error::ProgramUnavailable(program.to_string_lossy().into_owned())
    } else {
        Error::io(program.to_string_lossy().into_owned(), error)
    }
}

fn ensure_success(program: &std::ffi::OsStr, output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    Err(Error::CommandFailed {
        program: program.to_string_lossy().into_owned(),
        status: output.status.code().map_or_else(
            || "terminated by signal".to_owned(),
            |code| code.to_string(),
        ),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

fn split_pages(text: &str) -> Vec<ExtractedPage> {
    let mut pieces = text.split('\u{000c}').collect::<Vec<_>>();
    if pieces.last().is_some_and(|page| page.trim().is_empty()) {
        pieces.pop();
    }
    if pieces.is_empty() {
        pieces.push(text);
    }
    pieces
        .into_iter()
        .enumerate()
        .map(|(index, page)| ExtractedPage {
            number: u32::try_from(index + 1).unwrap_or(u32::MAX),
            text: clean_page(page),
        })
        .collect()
}

fn clean_page(page: &str) -> String {
    let mut lines = Vec::<String>::new();
    let mut blank_lines = 0_u8;
    for line in page.lines() {
        let line = repair_missing_punctuation_spaces(line.trim_end());
        if line.trim().is_empty() {
            blank_lines = blank_lines.saturating_add(1);
            if blank_lines > 2 {
                continue;
            }
        } else {
            blank_lines = 0;
        }
        let continuation = line
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_lowercase);
        if continuation
            && let Some(previous) = lines.last_mut()
            && previous.ends_with('-')
        {
            previous.pop();
            previous.push_str(line.trim_start());
        } else {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_owned()
}

fn repair_missing_punctuation_spaces(line: &str) -> String {
    let characters = line.chars().collect::<Vec<_>>();
    let mut repaired = String::with_capacity(line.len());
    for (index, character) in characters.iter().copied().enumerate() {
        repaired.push(character);
        let Some(next) = characters.get(index + 1).copied() else {
            continue;
        };
        if next.is_whitespace() {
            continue;
        }

        let previous = index
            .checked_sub(1)
            .and_then(|previous| characters.get(previous))
            .copied();
        let prose_separator = matches!(character, ',' | ';' | ':')
            && previous.is_some_and(char::is_alphabetic)
            && next.is_alphabetic();
        let abbreviation = character == '.'
            && next.is_alphabetic()
            && ["i.e.", "e.g."].iter().any(|suffix| {
                repaired
                    .get(repaired.len().saturating_sub(suffix.len())..)
                    .is_some_and(|ending| ending.eq_ignore_ascii_case(suffix))
            });
        if prose_separator || abbreviation {
            repaired.push(' ');
        }
    }
    repaired
}

fn parse_pdf_info(info: &str) -> PaperMetadata {
    let mut metadata = PaperMetadata::default();
    for line in info.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Title" if !value.is_empty() => value.clone_into(&mut metadata.title),
            "Author" if !value.is_empty() => metadata.authors = vec![value.to_owned()],
            "Subject" if !value.is_empty() => metadata.subject = Some(value.to_owned()),
            "Pages" => metadata.page_count = value.parse().ok(),
            _ => {}
        }
    }
    metadata
}

fn merge_pdf_metadata(target: &mut PaperMetadata, extracted: &PaperMetadata) {
    if !extracted.title.trim().is_empty() && !is_container_title(&extracted.title) {
        target.title.clone_from(&extracted.title);
    }
    if !extracted.authors.is_empty() {
        target.authors.clone_from(&extracted.authors);
    }
    target.page_count = extracted.page_count.or(target.page_count);
    if extracted.subject.is_some() {
        target.subject.clone_from(&extracted.subject);
    }
}

fn is_container_title(title: &str) -> bool {
    let lowered = title.trim().to_ascii_lowercase();
    lowered.starts_with("letters to the editor:")
        || lowered.starts_with("microsoft word -")
        || matches!(lowered.as_str(), "untitled" | "document")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pdfinfo_fields() {
        let metadata = parse_pdf_info(
            "Title: A Useful Paper\nAuthor: Ada Lovelace\nPages: 12\nSubject: Testing\n",
        );
        assert_eq!(metadata.title, "A Useful Paper");
        assert_eq!(metadata.authors, ["Ada Lovelace"]);
        assert_eq!(metadata.page_count, Some(12));
        assert_eq!(metadata.subject.as_deref(), Some("Testing"));
    }

    #[test]
    fn preserves_page_numbers_around_blank_pages() {
        let pages = split_pages("one\u{000c}\u{000c}three\u{000c}");
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[1].number, 2);
        assert!(pages[1].text.is_empty());
    }

    #[test]
    fn repairs_common_pdf_text_spacing_artifacts() {
        assert_eq!(
            clean_page("disastrous effects,and i.e.everything follows"),
            "disastrous effects, and i.e. everything follows"
        );
        assert_eq!(
            clean_page("version 2.1 and std::io"),
            "version 2.1 and std::io"
        );
    }

    #[test]
    fn keeps_filename_title_for_container_metadata() {
        let mut target = PaperMetadata {
            title: "GOTO Statements Considered Harmful".to_owned(),
            ..PaperMetadata::default()
        };
        let extracted = PaperMetadata {
            title: "Letters to the editor: go to statement considered harmful".to_owned(),
            ..PaperMetadata::default()
        };
        merge_pdf_metadata(&mut target, &extracted);
        assert_eq!(target.title, "GOTO Statements Considered Harmful");
    }
}
