use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{
    Result,
    domain::{
        CitationStatus, DocumentLayout, ExtractedPage, ExtractedPaper, Highlight, HighlightOrigin,
        PaperAnalysis, PaperId, PaperMetadata,
    },
    error::Error,
    markdown,
};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const EXTRACTION_SCHEMA_VERSION: u16 = 6;

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExtractionMetadata {
    schema_version: u16,
    metadata: PaperMetadata,
}

impl ArtifactStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn initialize(&self) -> Result<()> {
        fs::create_dir_all(self.root.join("papers"))
            .await
            .map_err(|error| Error::io(&self.root, error))
    }

    pub async fn load_analysis(&self, id: &PaperId) -> Result<Option<PaperAnalysis>> {
        read_json_if_present(&self.analysis_path(id)).await
    }

    pub async fn save_analysis(&self, id: &PaperId, analysis: &PaperAnalysis) -> Result<()> {
        let directory = self.paper_dir(id);
        fs::create_dir_all(&directory)
            .await
            .map_err(|error| Error::io(&directory, error))?;

        let mut json = serde_json::to_vec_pretty(analysis)?;
        json.push(b'\n');
        write_atomic(&self.analysis_path(id), &json).await?;
        write_atomic(&self.digest_path(id), render_digest(analysis).as_bytes()).await?;
        self.sync_ai_highlights(id, analysis).await
    }

    pub async fn load_extraction(&self, id: &PaperId) -> Result<Option<ExtractedPaper>> {
        let metadata_path = self.extraction_metadata_path(id);
        let Some(manifest) = read_json_if_present::<ExtractionMetadata>(&metadata_path).await?
        else {
            return Ok(None);
        };
        if manifest.schema_version != EXTRACTION_SCHEMA_VERSION {
            return Ok(None);
        }

        let text_path = self.text_path(id);
        let text = match fs::read_to_string(&text_path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(text_path, error)),
        };
        let pages = text
            .split('\u{000c}')
            .enumerate()
            .filter_map(|(index, text)| {
                let text = text.trim().to_owned();
                (!text.is_empty()).then(|| ExtractedPage {
                    number: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    text,
                })
            })
            .collect();
        let Some(layout) = read_json_if_present::<DocumentLayout>(&self.layout_path(id)).await?
        else {
            return Ok(None);
        };
        Ok(Some(ExtractedPaper {
            metadata: manifest.metadata,
            pages,
            layout,
        }))
    }

    pub async fn save_extraction(&self, id: &PaperId, paper: &ExtractedPaper) -> Result<()> {
        let directory = self.paper_dir(id);
        fs::create_dir_all(&directory)
            .await
            .map_err(|error| Error::io(&directory, error))?;

        let text = paper
            .pages
            .iter()
            .map(|page| page.text.trim())
            .collect::<Vec<_>>()
            .join("\n\u{000c}\n");
        write_atomic(&self.text_path(id), text.as_bytes()).await?;
        write_atomic(
            &self.markdown_path(id),
            markdown::render_source(paper).as_bytes(),
        )
        .await?;
        let mut layout = serde_json::to_vec_pretty(&paper.layout)?;
        layout.push(b'\n');
        write_atomic(&self.layout_path(id), &layout).await?;

        let manifest = ExtractionMetadata {
            schema_version: EXTRACTION_SCHEMA_VERSION,
            metadata: paper.metadata.clone(),
        };
        let mut json = serde_json::to_vec_pretty(&manifest)?;
        json.push(b'\n');
        write_atomic(&self.extraction_metadata_path(id), &json).await
    }

    pub async fn load_markdown(&self, id: &PaperId) -> Result<Option<String>> {
        let path = self.markdown_path(id);
        match fs::read_to_string(&path).await {
            Ok(markdown) => Ok(Some(markdown)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::io(path, error)),
        }
    }

    pub async fn ensure_markdown(&self, id: &PaperId, paper: &ExtractedPaper) -> Result<String> {
        if let Some(markdown) = self.load_markdown(id).await? {
            return Ok(markdown);
        }
        let markdown = markdown::render_source(paper);
        write_atomic(&self.markdown_path(id), markdown.as_bytes()).await?;
        Ok(markdown)
    }

    pub async fn load_highlights(&self, id: &PaperId) -> Result<Vec<Highlight>> {
        let path = self.highlights_path(id);
        let contents = match fs::read_to_string(&path).await {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io(path, error)),
        };
        contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub async fn save_highlights(&self, id: &PaperId, highlights: &[Highlight]) -> Result<()> {
        let directory = self.paper_dir(id);
        fs::create_dir_all(&directory)
            .await
            .map_err(|error| Error::io(&directory, error))?;
        let mut ordered = highlights.to_vec();
        ordered.sort_by(|left, right| left.id.cmp(&right.id));
        let mut jsonl = String::new();
        for highlight in &ordered {
            jsonl.push_str(&serde_json::to_string(highlight)?);
            jsonl.push('\n');
        }
        write_atomic(&self.highlights_path(id), jsonl.as_bytes()).await?;
        write_atomic(
            &self.highlights_markdown_path(id),
            render_highlights(&ordered).as_bytes(),
        )
        .await
    }

    async fn sync_ai_highlights(&self, id: &PaperId, analysis: &PaperAnalysis) -> Result<()> {
        let mut highlights = self.load_highlights(id).await?;
        highlights.retain(|highlight| matches!(highlight.origin, HighlightOrigin::User));
        for section in &analysis.sections {
            for (index, quote) in section.key_quotes.iter().enumerate() {
                if !matches!(
                    quote.validation,
                    CitationStatus::Exact | CitationStatus::Normalized
                ) {
                    continue;
                }
                let Some(anchor) = quote.anchor.clone() else {
                    continue;
                };
                highlights.push(Highlight {
                    id: format!("ai-{}-{index}", section.id),
                    origin: HighlightOrigin::Ai {
                        provider: analysis.provider,
                        section_id: section.id.clone(),
                        quote_index: u32::try_from(index).unwrap_or(u32::MAX),
                    },
                    kind: quote.significance.into(),
                    anchor,
                    text: quote.text.clone(),
                    note: quote.explanation.clone(),
                    created_at: analysis.generated_at,
                });
            }
        }
        self.save_highlights(id, &highlights).await
    }

    #[must_use]
    pub fn paper_dir(&self, id: &PaperId) -> PathBuf {
        self.root.join("papers").join(id.as_str())
    }

    fn analysis_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("analysis.json")
    }

    fn digest_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("digest.md")
    }

    fn text_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("source.txt")
    }

    fn markdown_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("source.md")
    }

    fn layout_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("layout.json")
    }

    fn highlights_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("highlights.jsonl")
    }

    fn highlights_markdown_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("highlights.md")
    }

    fn extraction_metadata_path(&self, id: &PaperId) -> PathBuf {
        self.paper_dir(id).join("extraction.json")
    }
}

async fn read_json_if_present<T>(path: &Path) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = match fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io(path, error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(Error::from)
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let extension = format!("tmp-{}-{sequence}", std::process::id());
    let temporary = path.with_extension(extension);
    fs::write(&temporary, bytes)
        .await
        .map_err(|error| Error::io(&temporary, error))?;
    fs::rename(&temporary, path)
        .await
        .map_err(|error| Error::io(path, error))
}

fn render_digest(analysis: &PaperAnalysis) -> String {
    use std::fmt::Write;

    let mut markdown = format!("# Paper digest\n\n## TL;DR\n\n> {}\n\n", analysis.thesis);
    if let Some(author_abstract) = &analysis.author_abstract {
        let _ = write!(markdown, "## Author abstract\n\n{author_abstract}\n\n");
    }
    let _ = write!(
        markdown,
        "## Contextual supplement\n\n{}\n\n## Reading path\n\n",
        analysis.outsider_brief
    );
    for item in &analysis.reading_path {
        let _ = writeln!(markdown, "- {item}");
    }
    for section in &analysis.sections {
        let _ = write!(
            markdown,
            "\n## {}\n\n_Pages {}–{}_\n\n{}\n\n",
            section.title, section.pages.start, section.pages.end, section.digest
        );
        for quote in &section.key_quotes {
            let _ = write!(
                markdown,
                "> {}\n>\n> — p. {}\n\n{}\n\n",
                quote.text.replace('\n', " "),
                quote.page,
                quote.explanation
            );
        }
    }
    if !analysis.glossary.is_empty() {
        markdown.push_str("\n## Gloss\n\n");
        for entry in &analysis.glossary {
            let _ = writeln!(markdown, "- **{}** — {}", entry.term, entry.plain_language);
        }
    }
    markdown
}

fn render_highlights(highlights: &[Highlight]) -> String {
    use std::fmt::Write;

    let mut markdown = String::from(
        "# Highlights\n\n<!-- Generated view. highlights.jsonl is the canonical, line-oriented plaintext record. -->\n\n",
    );
    for highlight in highlights {
        let owner = match &highlight.origin {
            HighlightOrigin::Ai {
                provider,
                section_id,
                ..
            } => format!("AI ({provider}, section `{section_id}`)"),
            HighlightOrigin::User => "Reader".to_owned(),
        };
        let _ = write!(
            markdown,
            "## {} · p. {}\n\n> {}\n\n- Owner: {}\n- Kind: `{:?}`\n- Tokens: `{}`–`{}`\n",
            highlight.id,
            highlight.anchor.page,
            highlight.text.replace('\n', " "),
            owner,
            highlight.kind,
            highlight.anchor.start_token,
            highlight.anchor.end_token,
        );
        if !highlight.note.trim().is_empty() {
            let _ = writeln!(markdown, "- Note: {}", highlight.note.replace('\n', " "));
        }
        markdown.push('\n');
    }
    markdown
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;

    use super::*;
    use crate::domain::{AnalysisProvider, PaperAnalysis};

    #[tokio::test]
    async fn extraction_round_trips_as_plain_text() -> Result<()> {
        let directory = tempdir().map_err(|error| Error::io("tempdir", error))?;
        let store = ArtifactStore::new(directory.path());
        store.initialize().await?;
        let id = PaperId::from_relative_path(Path::new("paper.pdf"));
        let paper = ExtractedPaper {
            metadata: PaperMetadata {
                title: "Test paper".to_owned(),
                ..PaperMetadata::default()
            },
            pages: vec![
                ExtractedPage {
                    number: 1,
                    text: "First page".to_owned(),
                },
                ExtractedPage {
                    number: 2,
                    text: "Second page".to_owned(),
                },
            ],
            layout: DocumentLayout {
                schema_version: 1,
                pages: Vec::new(),
            },
        };
        store.save_extraction(&id, &paper).await?;
        let loaded = store
            .load_extraction(&id)
            .await?
            .ok_or_else(|| Error::Task("missing extraction".to_owned()))?;
        assert_eq!(loaded.pages.len(), 2);
        assert_eq!(loaded.pages[1].text, "Second page");
        let markdown = store
            .load_markdown(&id)
            .await?
            .ok_or_else(|| Error::Task("missing source.md".to_owned()))?;
        assert!(markdown.contains("# Test paper"));
        assert!(markdown.contains("## PDF page 2"));
        Ok(())
    }

    #[tokio::test]
    async fn analysis_writes_json_and_markdown() -> Result<()> {
        let directory = tempdir().map_err(|error| Error::io("tempdir", error))?;
        let store = ArtifactStore::new(directory.path());
        store.initialize().await?;
        let id = PaperId::from_relative_path(Path::new("paper.pdf"));
        let analysis = PaperAnalysis {
            schema_version: 1,
            provider: AnalysisProvider::Heuristic,
            generated_at: Utc::now(),
            thesis: "A test thesis".to_owned(),
            outsider_brief: "A test brief".to_owned(),
            author_abstract: Some("The authors' own test abstract is preserved here.".to_owned()),
            prerequisites: Vec::new(),
            sections: Vec::new(),
            claims: Vec::new(),
            glossary: Vec::new(),
            caveats: Vec::new(),
            reading_path: Vec::new(),
        };
        store.save_analysis(&id, &analysis).await?;
        assert!(store.load_analysis(&id).await?.is_some());
        let digest = fs::read_to_string(store.paper_dir(&id).join("digest.md"))
            .await
            .map_err(|error| Error::io("digest.md", error))?;
        assert!(digest.contains("A test thesis"));
        assert!(digest.contains("The authors' own test abstract"));
        Ok(())
    }
}
