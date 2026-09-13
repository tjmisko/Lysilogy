//! A source-coordinate reading index, deliberately separate from citation anchors.
//!
//! Offsets are UTF-16 code units, matching JavaScript strings; geometry is PDF points.
//! Native text is never silently replaced by model output. OCR pages carry provenance.

mod cache;
mod figures;
mod native;
mod ocr;
mod paragraphs;

#[cfg(test)]
pub(crate) mod test_support;

use std::{path::Path, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{io::AsyncReadExt, process::Command};
use unicode_segmentation::UnicodeSegmentation;

use crate::{Error, Result, domain::TextRect};

pub use cache::{IndexDocument, load_cached, load_or_build, load_or_build_priority};

pub const SCHEMA_VERSION: u16 = 5;
const MAX_PAGES: usize = 400;
const MAX_OCR_PAGES: usize = 12;
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMMAND_BYTES: usize = 48 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(35);
const BUILD_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReadingIndex {
    pub schema_version: u16,
    pub text: String,
    pub pages: Vec<ReadingPage>,
    pub tokens: Vec<ReadingToken>,
    pub objects: TextObjects,
    pub figures: Vec<Figure>,
    pub gaps: Vec<IndexGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReadingPage {
    pub number: u32,
    pub width: f32,
    pub height: f32,
    pub start: usize,
    pub end: usize,
    pub provenance: Provenance,
    pub confidence: Option<f32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Native,
    Ocr,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReadingToken {
    pub start: usize,
    pub end: usize,
    pub page: u32,
    pub rects: Vec<TextRect>,
    pub text: String,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TextObjects {
    pub word: Vec<TextRange>,
    #[serde(rename = "WORD")]
    pub big_word: Vec<TextRange>,
    pub sentence: Vec<TextRange>,
    pub paragraph: Vec<Paragraph>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Paragraph {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    /// Ordered pieces of one logical paragraph; omitted for contiguous text.
    /// The bounding start/end may contain floats that are NOT paragraph members.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TextRange>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Figure {
    #[serde(default)]
    pub kind: String,
    pub id: String,
    pub label: String,
    pub page: u32,
    pub caption: String,
    pub start: usize,
    pub end: usize,
    /// A conservative candidate region, not a verified image segmentation.
    pub rect: Option<TextRect>,
    pub confidence: String,
    pub references: Vec<FigureReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FigureReference {
    pub page: u32,
    pub start: usize,
    pub end: usize,
    pub rects: Vec<TextRect>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IndexGap {
    pub page: u32,
    pub reason: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct SourceStamp {
    bytes: u64,
    modified_nanos: u128,
}

#[derive(Deserialize, Serialize)]
struct CachedIndex {
    source: SourceStamp,
    #[serde(default)]
    generation: String,
    index: ReadingIndex,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BuildPriority {
    #[default]
    Interactive,
    Background,
}

#[derive(Clone, Debug)]
struct SourceWord {
    text: String,
    line: u32,
    block: u32,
    rect: TextRect,
}

#[derive(Clone, Debug)]
struct SourcePage {
    number: u32,
    width: f32,
    height: f32,
    words: Vec<SourceWord>,
    provenance: Provenance,
    confidence: Option<f32>,
}

async fn build(
    source: &Path,
    directory: &Path,
    stamp: &SourceStamp,
    priority: BuildPriority,
) -> Result<ReadingIndex> {
    let mut command = extraction_command("pdftotext", priority);
    command
        .args([
            "-bbox-layout",
            "-enc",
            "UTF-8",
            "-f",
            "1",
            "-l",
            &MAX_PAGES.to_string(),
        ])
        .arg(source)
        .arg("-");
    let bytes = bounded_command(&mut command, "pdftotext", MAX_COMMAND_BYTES).await?;
    let mut pages = native::parse(&String::from_utf8_lossy(&bytes))?;
    let mut gaps = Vec::new();
    if pages.len() == MAX_PAGES {
        gaps.push(IndexGap {
            page: u32::try_from(MAX_PAGES + 1).unwrap_or(u32::MAX),
            reason: "Reading index stops at 400 pages; later pages may be unindexed.".into(),
        });
    }
    let mut ocr_count = 0;
    let ocr_started = tokio::time::Instant::now();
    for page in &mut pages {
        if native::usable(page) {
            continue;
        }
        let cached = ocr::cached_page(directory, page, stamp).await;
        if cached.is_none()
            && (ocr_count >= MAX_OCR_PAGES || ocr_started.elapsed() > Duration::from_secs(100))
        {
            gaps.push(IndexGap { page: page.number, reason: "Native text is sparse or unusable; bounded OCR budget (12 pages / 100 seconds) was exhausted. Refresh the reading index to process the next batch.".into() });
            if page.words.is_empty() {
                page.provenance = Provenance::Unavailable;
            }
            continue;
        }
        let result = if let Some(cached) = cached {
            Ok(cached)
        } else {
            ocr_count += 1;
            ocr::page(source, directory, page, stamp, priority).await
        };
        match result {
            Ok(ocr) if ocr.words.len() > page.words.len() => {
                if ocr.confidence.is_some_and(|confidence| confidence < 0.75) {
                    gaps.push(IndexGap {
                        page: page.number,
                        reason:
                            "OCR text has low confidence; verify quotations against the page image."
                                .into(),
                    });
                }
                *page = ocr;
            }
            Ok(_) => {
                gaps.push(IndexGap { page: page.number, reason: "No additional readable text was detected by OCR; page may be blank, graphical, or unreadable.".into() });
                if page.words.is_empty() {
                    page.provenance = Provenance::Unavailable;
                }
            }
            Err(error) => {
                gaps.push(IndexGap {
                    page: page.number,
                    reason: format!("Local OCR unavailable: {error}"),
                });
                if page.words.is_empty() {
                    page.provenance = Provenance::Unavailable;
                }
            }
        }
    }
    let mut index = assemble(&pages);
    index.gaps.extend(gaps);
    index.figures = figures::find(&index);
    Ok(index)
}

// Background work remains bounded even on platforms without `nice`; priority
// changes apply only to children, never the server or an already running job.
fn extraction_command(program: &str, priority: BuildPriority) -> Command {
    #[cfg(unix)]
    if priority == BuildPriority::Background && Path::new("/usr/bin/nice").is_file() {
        let mut command = Command::new("/usr/bin/nice");
        command.args(["-n", "10", program]);
        return command;
    }
    let _ = priority;
    Command::new(program)
}

/// Hard output/time limits and kill-on-drop also apply if a build task is canceled.
async fn bounded_command(command: &mut Command, program: &str, limit: usize) -> Result<Vec<u8>> {
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::ProgramUnavailable(program.into())
            } else {
                Error::io(program, error)
            }
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Task("Command has no output pipe".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Task("Command has no error pipe".into()))?;
    let read = async {
        let mut bytes = Vec::new();
        stdout
            .take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| Error::io(program, error))?;
        if bytes.len() > limit {
            return Err(Error::Task(format!(
                "{program} output exceeded its bounded limit"
            )));
        }
        Ok(bytes)
    };
    let errors = async {
        let mut bytes = Vec::new();
        stderr
            .take(65537)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| Error::io(program, error))?;
        if bytes.len() > 65536 {
            return Err(Error::Task(format!(
                "{program} error output exceeded its bounded limit"
            )));
        }
        Ok(bytes)
    };
    tokio::time::timeout(COMMAND_TIMEOUT, async {
        let (bytes, errors) = tokio::try_join!(read, errors)?;
        let status = child
            .wait()
            .await
            .map_err(|error| Error::io(program, error))?;
        if !status.success() {
            return Err(Error::CommandFailed {
                program: program.into(),
                status: status.to_string(),
                stderr: String::from_utf8_lossy(&errors).trim().to_owned(),
            });
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| Error::Task(format!("{program} timed out after 35 seconds")))?
}

fn assemble(pages: &[SourcePage]) -> ReadingIndex {
    let mut index = ReadingIndex {
        schema_version: SCHEMA_VERSION,
        text: String::new(),
        pages: Vec::new(),
        tokens: Vec::new(),
        objects: TextObjects::default(),
        figures: Vec::new(),
        gaps: Vec::new(),
    };
    let mut offset = 0;
    let mut paragraph_bytes = Vec::<(usize, usize, usize)>::new();
    for page in pages {
        if index.text.len() >= MAX_TEXT_BYTES {
            index.gaps.push(IndexGap {
                page: page.number,
                reason:
                    "Reading-index text limit (4 MiB) reached; remaining pages were not indexed."
                        .into(),
            });
            break;
        }
        let start = offset;
        for (paragraph_number, paragraph) in native::paragraphs(page).into_iter().enumerate() {
            let continuation = paragraph_number == 0
                && paragraph.kind == "body"
                && index
                    .objects
                    .paragraph
                    .last()
                    .is_some_and(|previous| previous.kind == "body")
                && index.pages.last().is_some_and(|previous| {
                    previous.number + 1 == page.number && previous.end == offset
                })
                && paragraph
                    .words
                    .first()
                    .is_some_and(|word| word.text.chars().next().is_some_and(char::is_lowercase))
                && !index.text.ends_with(['.', '!', '?', ':', ';']);
            if !index.text.is_empty() {
                let separator = if continuation { " " } else { "\n\n" };
                index.text.push_str(separator);
                offset += separator.len();
            }
            let paragraph_start = offset;
            let byte_start = index.text.len();
            for (word_number, word) in paragraph.words.iter().enumerate() {
                let previous = word_number
                    .checked_sub(1)
                    .and_then(|number| paragraph.words.get(number));
                append_word(&mut index, word, previous.copied(), page, &mut offset);
            }
            if offset == paragraph_start {
                continue;
            }
            if continuation {
                if let Some(previous) = index.objects.paragraph.last_mut() {
                    previous.end = offset;
                }
                if let Some(previous) = paragraph_bytes.last_mut() {
                    previous.1 = index.text.len();
                }
            } else {
                index.objects.paragraph.push(Paragraph {
                    start: paragraph_start,
                    end: offset,
                    kind: paragraph.kind,
                    spans: Vec::new(),
                });
                paragraph_bytes.push((byte_start, index.text.len(), paragraph_start));
            }
        }
        index.pages.push(ReadingPage {
            number: page.number,
            width: page.width,
            height: page.height,
            start,
            end: offset,
            provenance: page.provenance,
            confidence: page.confidence,
        });
    }
    for (start, end, offset) in paragraph_bytes {
        objects(&index.text[start..end], offset, &mut index.objects);
    }
    paragraphs::link_continuations(&mut index);
    index
}

fn append_word(
    index: &mut ReadingIndex,
    word: &SourceWord,
    previous: Option<&SourceWord>,
    page: &SourcePage,
    offset: &mut usize,
) {
    let normalized = normalize(&word.text);
    if normalized.is_empty() {
        return;
    }
    let join = previous.is_some_and(|previous| {
        previous.line != word.line
            && previous.text.ends_with('-')
            && normalized.chars().next().is_some_and(char::is_lowercase)
    });
    if join {
        let preserve = previous.is_some_and(|previous| preserve_hyphen(&previous.text));
        if !preserve && index.text.ends_with('-') {
            index.text.pop();
            *offset -= 1;
            if let Some(token) = index.tokens.last_mut() {
                token.text.pop();
                token.end -= 1;
            }
        }
    } else if previous.is_some() {
        index.text.push(' ');
        *offset += 1;
    }
    let start = *offset;
    index.text.push_str(&normalized);
    *offset += normalized.encode_utf16().count();
    index.tokens.push(ReadingToken {
        start,
        end: *offset,
        page: page.number,
        rects: vec![word.rect],
        text: normalized,
        provenance: page.provenance,
    });
}

fn preserve_hyphen(word: &str) -> bool {
    [
        "self-", "cross-", "high-", "low-", "well-", "multi-", "single-", "state-", "goal-",
        "real-", "time-", "hand-", "finite-", "large-", "small-", "out-", "user-",
    ]
    .contains(&word.to_ascii_lowercase().as_str())
}

fn normalize(text: &str) -> String {
    text.replace('ﬀ', "ff")
        .replace('ﬁ', "fi")
        .replace('ﬂ', "fl")
        .replace('ﬃ', "ffi")
        .replace('ﬄ', "ffl")
        .replace(['ﬅ', 'ﬆ'], "st")
        .replace(['\u{00ad}', '\u{200b}', '\0'], "")
}

fn objects(text: &str, start: usize, objects: &mut TextObjects) {
    // Unicode-aware boundaries; punctuation runs are Vim words too (iw vs iW).
    let mut offset = start;
    let mut run: Option<(usize, bool)> = None;
    let mut big_start = None;
    for ch in text.chars().chain(std::iter::once(' ')) {
        let whitespace = ch.is_whitespace();
        let class = ch.is_alphanumeric() || ch == '_';
        if let Some((run_start, old_class)) = run
            && (whitespace || class != old_class)
        {
            objects.word.push(TextRange {
                start: run_start,
                end: offset,
            });
            run = None;
        }
        if !whitespace && run.is_none() {
            run = Some((offset, class));
        }
        if whitespace {
            if let Some(big) = big_start.take() {
                objects.big_word.push(TextRange {
                    start: big,
                    end: offset,
                });
            }
        } else if big_start.is_none() {
            big_start = Some(offset);
        }
        offset += ch.len_utf16();
    }
    let mut sentence_offset = start;
    for sentence in text.unicode_sentences() {
        let leading = sentence
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .map(char::len_utf16)
            .sum::<usize>();
        let trimmed = sentence.trim();
        let end = sentence_offset + leading + trimmed.encode_utf16().count();
        if end > sentence_offset + leading {
            objects.sentence.push(TextRange {
                start: sentence_offset + leading,
                end,
            });
        }
        sentence_offset += sentence.encode_utf16().count();
    }
}

#[cfg(test)]
mod tests;
