export type AnalysisProvider = "codex" | "claude" | "heuristic";

export type ProcessingStatus =
  | { state: "discovered" }
  | { state: "queued"; provider: AnalysisProvider }
  | { state: "extracting" }
  | { state: "extracted" }
  | { state: "analyzing"; provider: AnalysisProvider }
  | { state: "ready" }
  | {
      state: "failed";
      stage: "discovery" | "extraction" | "analysis" | "persistence";
      message: string;
      retryable: boolean;
    };

export type PaperMetadata = {
  title: string;
  authors: string[];
  year: number | null;
  page_count: number | null;
  subject: string | null;
};
export type PaperOverview = {
  id: string;
  metadata: PaperMetadata;
  relative_path: string;
  status: ProcessingStatus;
  analyzed_at: string | null;
  one_line_summary: string | null;
};

export type LibraryResponse = {
  name: string;
  papers: PaperOverview[];
};

export type SectionKind =
  | "abstract"
  | "background"
  | "research_question"
  | "theory"
  | "methods"
  | "data"
  | "results"
  | "discussion"
  | "limitations"
  | "conclusion"
  | "references"
  | "appendix"
  | "other";

export type SectionFamily =
  | "context"
  | "question"
  | "method"
  | "evidence"
  | "interpretation"
  | "caveat"
  | "reference";

export type PageSpan = { start: number; end: number };

export type CitationStatus = "unverified" | "exact" | "normalized" | "ambiguous" | "missing";

export type TextRect = {
  x_min: number;
  y_min: number;
  x_max: number;
  y_max: number;
};

export type TextAnchor = {
  page: number;
  start_token: number;
  end_token: number;
  sentence_ids: string[];
  rects: TextRect[];
  exact_text: string;
};

export type KeyQuote = {
  text: string;
  page: number;
  explanation: string;
  significance: "thesis" | "definition" | "evidence" | "qualification" | "turning_point";
  anchor?: TextAnchor | null;
  validation?: CitationStatus;
};

export type PaperSection = {
  id: string;
  title: string;
  kind: SectionKind;
  family: SectionFamily;
  pages: PageSpan;
  summary: string;
  digest: string;
  source_span?: { start: TextAnchor; end: TextAnchor } | null;
  key_quotes: KeyQuote[];
  related_terms: string[];
  tile_width: number;
  tile_height: number;
};

export type Claim = {
  statement: string;
  support: string;
  strength: "speculative" | "suggestive" | "supported" | "strong";
  section_ids: string[];
};

export type GlossaryEntry = {
  term: string;
  plain_language: string;
  technical_definition: string;
  why_it_matters: string;
  section_ids: string[];
};

export type PaperAnalysis = {
  schema_version: number;
  provider: AnalysisProvider;
  generated_at: string;
  thesis: string;
  outsider_brief: string;
  author_abstract?: string | null;
  prerequisites: string[];
  sections: PaperSection[];
  claims: Claim[];
  glossary: GlossaryEntry[];
  caveats: string[];
  reading_path: string[];
};

export type PaperView = {
  paper: PaperOverview;
  analysis: PaperAnalysis | null;
};

export type LayoutSentence = {
  id: string;
  page: number;
  start_token: number;
  end_token: number;
  text: string;
  rects: TextRect[];
};

export type LayoutToken = {
  index: number;
  text: string;
  line: number;
  rects: TextRect[];
};

export type LayoutPage = {
  number: number;
  width: number;
  height: number;
  tokens: LayoutToken[];
  sentences: LayoutSentence[];
};

export type DocumentLayout = {
  schema_version: number;
  pages: LayoutPage[];
};

export type HighlightKind = "thesis" | "definition" | "evidence" | "qualification" | "note";

export type HighlightOrigin =
  | {
      type: "ai";
      provider: AnalysisProvider;
      section_id: string;
      quote_index: number;
    }
  | { type: "user" };

export type Highlight = {
  id: string;
  origin: HighlightOrigin;
  kind: HighlightKind;
  anchor: TextAnchor;
  text: string;
  note: string;
  created_at: string;
};

export type PaperMap = {
  layout: DocumentLayout;
  highlights: Highlight[];
};

export type Clarification = {
  selection: string;
  answer: string;
  concepts: GlossaryEntry[];
  connections: string[];
  limitation: string | null;
  provider: AnalysisProvider;
};
