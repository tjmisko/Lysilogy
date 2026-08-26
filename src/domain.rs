use std::{fmt, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Stable, path-derived identifier that does not expose a source path to clients.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaperId(String);

impl PaperId {
    #[must_use]
    pub fn from_relative_path(path: &Path) -> Self {
        let normalized = path.to_string_lossy().replace('\\', "/");
        // FNV-1a keeps IDs deterministic across machines without tying artifacts to
        // an absolute vault path or requiring a database-generated key.
        let digest = normalized
            .as_bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        Self(format!("{digest:016x}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PaperId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::str::FromStr for PaperId {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let valid =
            value.len() == 16 && value.chars().all(|character| character.is_ascii_hexdigit());
        if valid {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err("paper IDs must contain exactly 16 hexadecimal characters".to_owned())
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PaperMetadata {
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub page_count: Option<u32>,
    pub subject: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperOverview {
    pub id: PaperId,
    pub metadata: PaperMetadata,
    pub relative_path: String,
    pub status: ProcessingStatus,
    pub analyzed_at: Option<DateTime<Utc>>,
    pub one_line_summary: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProcessingStatus {
    #[default]
    Discovered,
    Queued {
        provider: AnalysisProvider,
    },
    Extracting,
    Extracted,
    Analyzing {
        provider: AnalysisProvider,
    },
    Ready,
    Failed {
        stage: ProcessingStage,
        message: String,
        retryable: bool,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStage {
    Discovery,
    Extraction,
    Analysis,
    Persistence,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisProvider {
    Codex,
    Claude,
    #[default]
    Heuristic,
}

impl fmt::Display for AnalysisProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Heuristic => "heuristic",
        })
    }
}

impl std::str::FromStr for AnalysisProvider {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "heuristic" | "offline" => Ok(Self::Heuristic),
            _ => Err(format!("unknown analysis provider: {value}")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperView {
    pub paper: PaperOverview,
    pub analysis: Option<PaperAnalysis>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperAnalysis {
    pub schema_version: u16,
    pub provider: AnalysisProvider,
    pub generated_at: DateTime<Utc>,
    /// A one-sentence, plain-language account of the paper's central contribution.
    pub thesis: String,
    /// One or two high-leverage sentences that add context the authored abstract
    /// does not provide, such as the field before/after or later reception.
    pub outsider_brief: String,
    /// The authors' own abstract, accepted only when it can be found in the
    /// extracted source text. Older analyses and papers without abstracts omit it.
    #[serde(default)]
    pub author_abstract: Option<String>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default)]
    pub sections: Vec<PaperSection>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub glossary: Vec<GlossaryEntry>,
    #[serde(default)]
    pub caveats: Vec<String>,
    #[serde(default)]
    pub reading_path: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperSection {
    pub id: String,
    pub title: String,
    pub kind: SectionKind,
    pub family: SectionFamily,
    pub pages: PageSpan,
    pub summary: String,
    pub digest: String,
    #[serde(default)]
    pub source_span: Option<SectionSourceSpan>,
    #[serde(default)]
    pub key_quotes: Vec<KeyQuote>,
    #[serde(default)]
    pub related_terms: Vec<String>,
    #[serde(default = "default_tile_width")]
    pub tile_width: u8,
    #[serde(default = "default_tile_height")]
    pub tile_height: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SectionSourceSpan {
    pub start: TextAnchor,
    pub end: TextAnchor,
}

const fn default_tile_width() -> u8 {
    1
}

const fn default_tile_height() -> u8 {
    1
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionKind {
    Abstract,
    Background,
    ResearchQuestion,
    Theory,
    Methods,
    Data,
    Results,
    Discussion,
    Limitations,
    Conclusion,
    References,
    Appendix,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionFamily {
    Context,
    Question,
    Method,
    #[default]
    Evidence,
    Interpretation,
    Caveat,
    Reference,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageSpan {
    pub start: u32,
    pub end: u32,
}

impl PageSpan {
    #[must_use]
    pub fn normalized(start: u32, end: u32, maximum: u32) -> Self {
        let maximum = maximum.max(1);
        let start = start.clamp(1, maximum);
        let end = end.clamp(start, maximum);
        Self { start, end }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyQuote {
    pub text: String,
    pub page: u32,
    pub explanation: String,
    pub significance: QuoteSignificance,
    /// Deterministic coordinates resolved from the PDF text layer. Analyzer
    /// output never supplies this field; normalization does.
    #[serde(default)]
    pub anchor: Option<TextAnchor>,
    #[serde(default)]
    pub validation: CitationStatus,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuoteSignificance {
    Thesis,
    Definition,
    Evidence,
    Qualification,
    #[default]
    TurningPoint,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationStatus {
    #[default]
    Unverified,
    Exact,
    Normalized,
    Ambiguous,
    Missing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextRect {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextAnchor {
    pub page: u32,
    pub start_token: u32,
    pub end_token: u32,
    #[serde(default)]
    pub sentence_ids: Vec<String>,
    #[serde(default)]
    pub rects: Vec<TextRect>,
    pub exact_text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocumentLayout {
    pub schema_version: u16,
    #[serde(default)]
    pub pages: Vec<LayoutPage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayoutPage {
    pub number: u32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub tokens: Vec<LayoutToken>,
    #[serde(default)]
    pub sentences: Vec<LayoutSentence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayoutToken {
    pub index: u32,
    pub text: String,
    pub line: u32,
    #[serde(default)]
    pub rects: Vec<TextRect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayoutSentence {
    pub id: String,
    pub page: u32,
    pub start_token: u32,
    pub end_token: u32,
    pub text: String,
    #[serde(default)]
    pub rects: Vec<TextRect>,
}

impl LayoutSentence {
    #[must_use]
    pub fn anchor(&self) -> TextAnchor {
        TextAnchor {
            page: self.page,
            start_token: self.start_token,
            end_token: self.end_token,
            sentence_ids: vec![self.id.clone()],
            rects: self.rects.clone(),
            exact_text: self.text.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightKind {
    Thesis,
    Definition,
    #[default]
    Evidence,
    Qualification,
    Note,
}

impl From<QuoteSignificance> for HighlightKind {
    fn from(significance: QuoteSignificance) -> Self {
        match significance {
            QuoteSignificance::Thesis => Self::Thesis,
            QuoteSignificance::Definition => Self::Definition,
            QuoteSignificance::Evidence | QuoteSignificance::TurningPoint => Self::Evidence,
            QuoteSignificance::Qualification => Self::Qualification,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HighlightOrigin {
    Ai {
        provider: AnalysisProvider,
        section_id: String,
        quote_index: u32,
    },
    User,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Highlight {
    pub id: String,
    pub origin: HighlightOrigin,
    pub kind: HighlightKind,
    pub anchor: TextAnchor,
    pub text: String,
    #[serde(default)]
    pub note: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PaperMap {
    pub layout: DocumentLayout,
    pub highlights: Vec<Highlight>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateHighlightRequest {
    pub start_sentence_id: String,
    pub end_sentence_id: Option<String>,
    #[serde(default)]
    pub kind: HighlightKind,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claim {
    pub statement: String,
    pub support: String,
    pub strength: EvidenceStrength,
    #[serde(default)]
    pub section_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    Speculative,
    Suggestive,
    #[default]
    Supported,
    Strong,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlossaryEntry {
    pub term: String,
    pub plain_language: String,
    pub technical_definition: String,
    pub why_it_matters: String,
    #[serde(default)]
    pub section_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyzeRequest {
    #[serde(default)]
    pub provider: AnalysisProvider,
    #[serde(default)]
    pub force: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ClarifyRequest {
    pub section_id: Option<String>,
    pub selection: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub provider: AnalysisProvider,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clarification {
    pub selection: String,
    pub answer: String,
    #[serde(default)]
    pub concepts: Vec<GlossaryEntry>,
    #[serde(default)]
    pub connections: Vec<String>,
    pub limitation: Option<String>,
    pub provider: AnalysisProvider,
}

#[derive(Clone, Debug)]
pub struct ExtractedPage {
    pub number: u32,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ExtractedPaper {
    pub metadata: PaperMetadata,
    pub pages: Vec<ExtractedPage>,
    pub layout: DocumentLayout,
}

impl ExtractedPaper {
    #[must_use]
    pub fn full_text(&self) -> String {
        use std::fmt::Write;

        let mut text = String::new();
        for page in &self.pages {
            let _ = write!(text, "\n\n--- PAGE {} ---\n{}", page.number, page.text);
        }
        text
    }
}
