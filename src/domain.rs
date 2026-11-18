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
    #[serde(default)]
    pub author_abstract: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemotePdfSource {
    pub original_url: String,
    pub final_url: String,
    pub imported_at: DateTime<Utc>,
    pub byte_length: u64,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStage {
    Discovery,
    Extraction,
    Analysis,
    Persistence,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisComponent {
    Abstract,
    Context,
    Structure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisJobKind {
    Initial,
    AbstractRefresh,
    ContextRefresh,
    StructureRefresh,
    Revision,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AnalysisJobStatus {
    Queued,
    Running {
        stage: ProcessingStage,
    },
    Completed,
    Failed {
        stage: ProcessingStage,
        message: String,
        retryable: bool,
    },
}

impl AnalysisJobStatus {
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Running { .. })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisTaskStatus {
    Pending,
    Active,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisTask {
    pub id: String,
    pub label: String,
    pub status: AnalysisTaskStatus,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisJob {
    pub paper_id: PaperId,
    pub paper_title: String,
    pub provider: AnalysisProvider,
    pub kind: AnalysisJobKind,
    pub status: AnalysisJobStatus,
    pub progress: u8,
    pub tasks: Vec<AnalysisTask>,
    pub resumable: bool,
    pub feedback: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProcessingQueue {
    pub jobs: Vec<AnalysisJob>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSession {
    pub provider: AnalysisProvider,
    pub session_id: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FeedbackRequest {
    pub feedback: String,
    pub provider: AnalysisProvider,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackStatus {
    Queued,
    Applied,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub id: String,
    pub feedback: String,
    pub provider: AnalysisProvider,
    pub status: FeedbackStatus,
    pub submitted_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub session_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisProvider {
    Codex,
    Claude,
    #[serde(rename = "openrouter")]
    #[default]
    OpenRouter,
    /// Deprecated: the offline reader is hidden from the UI. Kept as an
    /// internal fallback and test helper.
    Heuristic,
}

impl fmt::Display for AnalysisProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenRouter => "openrouter",
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
            "openrouter" | "deepseek" => Ok(Self::OpenRouter),
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
    pub abstract_extraction: Option<crate::abstracts::AbstractResult>,
    /// Externally grounded field-history or reception claims. Every note is
    /// retained only when all of its source IDs pass independent URL checks.
    #[serde(default)]
    pub context_notes: Vec<ContextNote>,
    /// Exact source records for the independently checked public links cited by
    /// `context_notes`.
    #[serde(default)]
    pub context_sources: Vec<ContextSource>,
    #[serde(default)]
    pub context_assessment: Option<crate::analysis::context::ContextAssessment>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    /// The paper's own headings as printed, in reading order: the outline
    /// view. Empty for a paper that prints no headings and for analyses made
    /// before it was recorded.
    #[serde(default)]
    pub outline: Vec<OutlineEntry>,
    /// The logical reading units. When the paper has a usable outline each
    /// unit groups a contiguous range of `outline` entries.
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
    /// What produced this analysis: a digest of the prompts and schemas plus
    /// the model and provider, so a later review can be tied to exactly the
    /// pipeline that wrote it. Absent on analyses made before it was recorded.
    #[serde(default)]
    pub pipeline_version: Option<String>,
    /// Deterministic plausibility warnings about the model output, such as a
    /// section map that covers too little of the paper or a skipped
    /// verification pass. Empty when nothing looked wrong.
    #[serde(default)]
    pub quality_warnings: Vec<QualityWarning>,
    /// Why the paper matters, from the context node. `None` on analyses made
    /// before it existed and when the node did not run.
    #[serde(default)]
    pub context: Option<PaperContext>,
    /// How closely the model's abstract agrees with the deterministic
    /// extraction. `None` on analyses made before it was recorded.
    #[serde(default)]
    pub abstract_agreement: Option<crate::abstracts::AbstractAgreement>,
    /// The pipeline version that last wrote each refreshable component
    /// (`abstract`, `context`, `structure`), so a grade can be tied to the
    /// pipeline that produced the part it grades. A component missing here
    /// was written with the whole analysis under `pipeline_version`.
    #[serde(default)]
    pub component_versions: std::collections::BTreeMap<AnalysisComponent, String>,
}

impl PaperAnalysis {
    /// Record that `version` last wrote `component`, and make it the
    /// analysis's current pipeline version.
    pub fn record_component_version(&mut self, component: AnalysisComponent, version: String) {
        self.component_versions.insert(component, version.clone());
        self.pipeline_version = Some(version);
    }

    /// Whether any stored warning means the analysis is not a clean success.
    #[must_use]
    pub fn degraded(&self) -> bool {
        self.quality_warnings
            .iter()
            .any(|warning| warning.kind.degrades())
    }
}

/// One deterministic sanity check the model output failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityWarning {
    pub kind: QualityWarningKind,
    /// The record field the warning is about, e.g. `sections` or `identity`.
    pub field: String,
    /// A plain-language account of what looked wrong, with the numbers.
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityWarningKind {
    /// The section map is empty.
    SectionsMissing,
    /// Far fewer reading units than the page count warrants.
    SectionsSparse,
    /// The section map leaves much of the main text uncovered.
    SectionsLowCoverage,
    /// No references, although the text has a references heading.
    ReferencesMissing,
    /// No abstract, although the text has an abstract heading.
    AbstractMissing,
    /// The title is empty, very short, or looks like an identifier or stamp.
    TitleImplausible,
    /// No outline, although the text shows numbered headings.
    OutlineMissing,
    /// Far fewer outline entries than the numbered headings the text shows.
    OutlineSparse,
    /// Headings the model returned were not found in the page text and were
    /// dropped by admission.
    OutlineEntriesDropped,
    /// The model's grouping of outline entries had gaps, overlaps or split
    /// references and was repaired deterministically.
    SectionsGroupingRepaired,
    /// Logical units leave main-text outline entries uncovered.
    SectionsGroupingInvalid,
    /// Every logical unit is a single heading: a table of contents, not a
    /// reading map.
    SectionsMirrorOutline,
    /// No authors were returned.
    AuthorsMissing,
    /// A verification pass did not run because the paper budget ran out.
    VerificationSkipped,
    /// A verification pass ran but returned no usable answer.
    VerificationFailed,
    /// The model's abstract and the deterministic extraction disagree.
    AbstractDisagreement,
    /// Context claims whose quote or dossier record did not check out were
    /// shown as unverified background instead.
    ContextClaimsDowngraded,
    /// The context node ran but left no context in the record.
    ContextMissing,
    /// A model call of the plan failed, so the fields it fills are missing
    /// or come from an earlier answer.
    CallFailed,
    /// The paper budget kept a model call of the plan from starting.
    CallSkipped,
    /// A kind written by a newer version.
    #[serde(other)]
    Other,
}

impl QualityWarningKind {
    /// Whether the warning means the result should not be shown as a clean
    /// success: a verification pass or a model call did not complete.
    #[must_use]
    pub const fn degrades(self) -> bool {
        matches!(
            self,
            Self::VerificationSkipped
                | Self::VerificationFailed
                | Self::CallFailed
                | Self::CallSkipped
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Before,
    After,
    #[default]
    Legacy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextNote {
    #[serde(default)]
    pub kind: ContextKind,
    pub text: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    /// Where the note comes from, for notes written by the context node.
    /// `None` on notes from the older web-research design, whose support is
    /// `source_ids`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<ContextBasis>,
    /// Neighbour-dossier work ids that support a `dossier` note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work_ids: Vec<String>,
}

/// Where a claim about the paper's place in its field comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextBasis {
    /// The paper says so, and a verbatim quote was admitted against its text.
    Paper,
    /// A record in the neighbour dossier supports it.
    Dossier,
    /// The model's own background knowledge: shown, labelled unverified.
    /// Unknown values read as this, the least trusted class.
    #[default]
    #[serde(other)]
    Background,
}

/// Why a claim was moved to `background`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextDowngrade {
    /// It claimed the paper's support, but no quote survived admission.
    QuoteNotFound,
    /// It claimed a dossier record, but none of its work ids is in the dossier.
    UnknownWork,
    /// A kind written by a newer version.
    #[serde(other)]
    Other,
}

/// A verbatim passage of the paper and its 1-based PDF page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextQuote {
    pub quote: String,
    pub page: u32,
}

/// One claim of the context, with its source class.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextClaim {
    pub text: String,
    #[serde(default)]
    pub basis: ContextBasis,
    /// Admitted quotes for a `paper` claim.
    #[serde(default)]
    pub evidence: Vec<ContextQuote>,
    /// Dossier work ids for a `dossier` claim; also kept on a `background`
    /// claim as pointers.
    #[serde(default)]
    pub work_ids: Vec<String>,
    /// Set when the claim was asked for as `paper` or `dossier` but its
    /// support did not check out, so it is shown as `background`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downgraded: Option<ContextDowngrade>,
}

/// How a neighbouring work relates to this paper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeighbourRelation {
    Predecessor,
    Successor,
    Critique,
    Replication,
    /// An overview or textbook treatment of the question.
    Survey,
    /// Independent work on the same question at about the same time.
    Parallel,
    #[default]
    #[serde(other)]
    Other,
}

/// A neighbouring work worth reading next to this paper.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadAlongside {
    /// The citation-graph Work id when the work is in the dossier, so the
    /// reader can open it or ask Lysilogy to acquire it.
    #[serde(default)]
    pub work_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub authors: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub relation: NeighbourRelation,
    pub why: String,
    #[serde(default)]
    pub basis: ContextBasis,
    /// The library paper that holds this work, when Lysilogy has it.
    #[serde(default)]
    pub local_paper_id: Option<String>,
}

/// What settling the context did, for reviews and prompt tuning.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextCounts {
    /// Works in the dossier the model was given.
    pub dossier_works: usize,
    /// Claims by final basis.
    pub paper_claims: usize,
    pub dossier_claims: usize,
    pub background_claims: usize,
    /// `paper` claims whose quotes all failed admission.
    pub downgraded_quote_not_found: usize,
    /// `dossier` claims none of whose work ids was in the dossier.
    pub downgraded_unknown_work: usize,
    /// Work ids removed from claims or reading suggestions because the
    /// dossier has no such record.
    pub unknown_work_ids: usize,
    /// Reading suggestions dropped beyond the cap or without a title.
    pub dropped_read_alongside: usize,
    /// Claims dropped because their text was empty.
    pub dropped_empty_claims: usize,
}

/// Why the paper matters: the first view's account of where it sits in its
/// field, written by the context node.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaperContext {
    /// Two to four claims: why a smart outsider should care.
    pub lead: Vec<ContextClaim>,
    pub before: Vec<ContextClaim>,
    pub contribution: Vec<ContextClaim>,
    pub after: Vec<ContextClaim>,
    pub read_alongside: Vec<ReadAlongside>,
    /// What the model said it does not know about the reception.
    pub reception_note: Option<String>,
    pub counts: ContextCounts,
    /// The neighbour dossier the context node read, in compact form, so a
    /// reviewer can check a dossier claim against what the model was given.
    /// `None` on contexts written before it was stored.
    pub dossier: Option<ContextDossier>,
}

/// Whether the neighbour dossier could be assembled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DossierStatus {
    /// It was assembled from the citation graph; it may still be short.
    #[default]
    Ok,
    /// The paper is not linked to a work in the citation graph.
    NotLinked,
    /// No citation graph is configured.
    NoGraph,
    /// Assembling it failed, so the model was given an empty dossier.
    Error,
    /// A status written by a newer version.
    #[serde(other)]
    Other,
}

/// The dossier the context node read: whether it could be assembled, and
/// each work it listed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextDossier {
    pub status: DossierStatus,
    pub works: Vec<DossierWorkSummary>,
}

/// One dossier work, as much as a reviewer needs to recognise it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DossierWorkSummary {
    /// The citation-graph Work id that claims cite.
    pub id: String,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub first_author: Option<String>,
    /// `true` for a later work that cites the paper, `false` for one it cites.
    pub citing: bool,
    /// The library paper that holds this work, when Lysilogy has it.
    pub local_paper_id: Option<String>,
    /// The citation label on the edge between this work and the paper, when
    /// the graph had one at analysis time: why the paper cites it, or (for a
    /// citing work) why it cites the paper.
    pub edge: Option<DossierEdgeLabel>,
}

/// A citation label as the context node read it from the graph.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DossierEdgeLabel {
    pub relation: String,
    pub reason: String,
    pub quote: Option<String>,
    /// `core_relation`, `edge_labeler` or `user`.
    pub path: Option<String>,
    /// `substantive`, `perfunctory` or `not_shown`, when the labeler said.
    pub engagement: Option<String>,
    /// `unreviewed`, `accepted` or `disputed`; rejected labels are left out.
    pub review_state: String,
}

impl DossierEdgeLabel {
    /// Whether the label says the citation is only a mention.
    #[must_use]
    pub fn is_perfunctory(&self) -> bool {
        matches!(
            self.engagement.as_deref(),
            Some("perfunctory" | "not_shown")
        )
    }
}

impl PaperContext {
    /// Every claim, in reading order.
    pub fn claims(&self) -> impl Iterator<Item = &ContextClaim> {
        self.lead
            .iter()
            .chain(&self.before)
            .chain(&self.contribution)
            .chain(&self.after)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextSource {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub authors: Vec<String>,
    pub year: Option<u16>,
    pub url: String,
    /// A concise account of which part of the contextual note this source
    /// supports. This remains analyzer-supplied and is not labeled as a
    /// deterministic semantic verification.
    pub supports: String,
    /// Time at which Lysilogy independently resolved redirects and received a
    /// successful response from this public URL.
    pub verified_at: DateTime<Utc>,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub relationship: Option<String>,
}

/// One printed heading of the paper.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutlineEntry {
    /// The printed number or letter, such as `2.1`, `IV.` or `A.`.
    #[serde(default)]
    pub number: Option<String>,
    /// The heading text as printed, without its number.
    pub title: String,
    /// 1 for a top-level heading, 2 for a subsection, and so on.
    pub level: u8,
    /// The 1-based PDF page the heading is printed on.
    pub page: u32,
    #[serde(default)]
    pub role: OutlineRole,
    /// Where the heading is printed, resolved from the PDF text layer.
    #[serde(default)]
    pub anchor: Option<TextAnchor>,
}

/// Which part of the paper a heading belongs to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutlineRole {
    /// Abstract, keywords and similar material before the main text.
    FrontMatter,
    #[default]
    Main,
    /// Acknowledgments, funding, author notes and declarations.
    BackMatter,
    References,
    Appendix,
    /// A role written by a newer version.
    #[serde(other)]
    Other,
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
    /// The question this unit answers for the reader: why its parts belong
    /// together.
    #[serde(default)]
    pub question: Option<String>,
    /// Indices into `PaperAnalysis::outline` of the printed headings this
    /// unit groups, in order. Empty when the unit was made without an outline.
    #[serde(default)]
    pub outline_entries: Vec<usize>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentCatalog {
    pub schema_version: u16,
    #[serde(default)]
    pub reader_baseline: Vec<String>,
    #[serde(default)]
    pub experiments: Vec<PromptExperiment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptExperiment {
    pub id: String,
    pub name: String,
    pub question: String,
    #[serde(default)]
    pub live_web: bool,
    pub variants: Vec<PromptVariant>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptVariant {
    pub id: String,
    pub label: String,
    pub instruction: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct StartExperimentRequest {
    pub experiment_id: String,
    #[serde(default)]
    pub provider: AnalysisProvider,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LearningRamp {
    pub foothold: LearningFoothold,
    #[serde(default)]
    pub concepts: Vec<RampConcept>,
    #[serde(default)]
    pub essential_passages: Vec<EssentialPassage>,
    #[serde(default)]
    pub reception: Vec<ReceptionItem>,
    #[serde(default)]
    pub counterarguments: Vec<Counterargument>,
    #[serde(default)]
    pub uncertainties: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LearningFoothold {
    pub question: String,
    pub answer: String,
    pub why_care: String,
    pub mechanism: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RampConcept {
    pub term: String,
    pub plain_language: String,
    pub technical_definition: String,
    pub why_now: String,
    pub bridge: Option<String>,
    pub bridge_limit: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EssentialPassage {
    pub quote: String,
    pub page: u32,
    pub role: String,
    #[serde(default)]
    pub context_before: Vec<String>,
    pub read_for: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReceptionItem {
    pub kind: String,
    pub claim: String,
    #[serde(default)]
    pub source_urls: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Counterargument {
    pub camp: Option<String>,
    pub objection: String,
    pub target: String,
    pub likely_reply: String,
    #[serde(default)]
    pub source_urls: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentArm {
    pub blind_label: String,
    pub variant_id: String,
    pub variant_label: String,
    pub variant_instruction: String,
    pub output: Option<LearningRamp>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentRun {
    pub schema_version: u16,
    pub id: String,
    pub paper_id: PaperId,
    pub paper_title: String,
    pub experiment_id: String,
    pub experiment_name: String,
    pub question: String,
    #[serde(default)]
    pub reader_baseline: Vec<String>,
    pub provider: AnalysisProvider,
    pub status: ExperimentStatus,
    pub model: String,
    pub reasoning_effort: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub arms: Vec<ExperimentArm>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentView {
    pub run: ExperimentRun,
    pub judged: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExperimentJudgmentRequest {
    pub overall: String,
    pub early_traction: String,
    pub rigor: String,
    pub confidence: u8,
    pub arm_scores: Vec<ExperimentArmScore>,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentArmScore {
    pub blind_label: String,
    pub paper_specificity_actionability: u8,
    pub early_traction: u8,
    pub fidelity_rigor: u8,
    pub dependency_flow: u8,
    pub economy: u8,
    pub provenance_uncertainty: u8,
    #[serde(default)]
    pub target_dimension: Option<u8>,
    #[serde(default)]
    pub hard_reject: bool,
    #[serde(default)]
    pub failure_tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentJudgment {
    pub run_id: String,
    pub experiment_id: String,
    pub overall: String,
    pub early_traction: String,
    pub rigor: String,
    pub confidence: u8,
    #[serde(default)]
    pub arm_scores: Vec<ExperimentArmScore>,
    pub note: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExperimentRecord {
    pub run: ExperimentRun,
    pub judgment: Option<ExperimentJudgment>,
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

#[cfg(test)]
mod fixture_tests;
