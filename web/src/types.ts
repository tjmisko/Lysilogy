export type AnalysisProvider = "openrouter" | "codex" | "claude" | "heuristic";
export type ProcessingStage = "discovery" | "extraction" | "analysis" | "persistence";

export type ProcessingStatus =
  | { state: "discovered" }
  | { state: "queued"; provider: AnalysisProvider }
  | { state: "extracting" }
  | { state: "extracted" }
  | { state: "analyzing"; provider: AnalysisProvider }
  | { state: "ready" }
  | {
      state: "failed";
      stage: ProcessingStage;
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

export type LibraryDirectory = {
  id: string;
  name: string;
  path: string;
  primary: boolean;
  available: boolean;
  papers: number;
};

export type LibraryResponse = {
  name: string;
  papers: PaperOverview[];
  /** Absent when talking to an older backend. */
  directories?: LibraryDirectory[];
};

export type RemotePdfSource = {
  original_url: string;
  final_url: string;
  imported_at: string;
  byte_length: number;
};

export type ImportPdfResponse = {
  paper: PaperOverview;
  library: LibraryResponse;
  source: RemotePdfSource;
};

export type AnalysisTaskStatus = "pending" | "active" | "completed" | "failed";

export type AnalysisTask = {
  id: string;
  label: string;
  status: AnalysisTaskStatus;
  detail: string | null;
};

export type AnalysisJobStatus =
  | { state: "queued" }
  | { state: "running"; stage: ProcessingStage }
  | { state: "completed" }
  | {
      state: "failed";
      stage: ProcessingStage;
      message: string;
      retryable: boolean;
    };

export type AnalysisJob = {
  paper_id: string;
  paper_title: string;
  provider: AnalysisProvider;
  kind: "initial" | "revision" | "abstract_refresh" | "context_refresh" | "structure_refresh";
  status: AnalysisJobStatus;
  progress: number;
  tasks: AnalysisTask[];
  resumable: boolean;
  feedback: string | null;
  created_at: string;
  updated_at: string;
};

export type ProcessingQueue = {
  jobs: AnalysisJob[];
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

export type ContextNote = {
  kind?: "before" | "after" | "legacy";
  text: string;
  source_ids: string[];
};

export type ContextSource = {
  id: string;
  title: string;
  authors: string[];
  year: number | null;
  url: string;
  supports: string;
  verified_at: string;
  excerpt?: string | null;
  location?: string | null;
  relationship?: string | null;
};

export type PaperAnalysis = {
  schema_version: number;
  provider: AnalysisProvider;
  generated_at: string;
  thesis: string;
  outsider_brief: string;
  author_abstract?: string | null;
  abstract_extraction?: {
    status: "accepted" | "not_found" | "needs_review" | "needs_ocr";
    start_page: number | null;
    end_page: number | null;
    checks: string[];
    review: string | null;
  } | null;
  context_notes?: ContextNote[];
  context_sources?: ContextSource[];
  context_assessment?: {
    metrics: { proposed_claims: number; cited_claims: number; proposed_links: number;
      supported_links: number; fully_supported_claims: number; unassessed_claims: number;
      accepted_claims: number; published_claims: number; };
    evidence_gaps: string[];
    writer_model: string;
    assessed_at: string;
  } | null;
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

export type PromptVariant = {
  id: string;
  label: string;
  instruction: string;
};

export type PromptExperiment = {
  id: string;
  name: string;
  question: string;
  live_web: boolean;
  variants: PromptVariant[];
};

export type ExperimentCatalog = {
  schema_version: number;
  reader_baseline: string[];
  experiments: PromptExperiment[];
};

export type LearningRamp = {
  foothold: {
    question: string;
    answer: string;
    why_care: string;
    mechanism: string;
  };
  concepts: Array<{
    term: string;
    plain_language: string;
    technical_definition: string;
    why_now: string;
    bridge: string | null;
    bridge_limit: string | null;
  }>;
  essential_passages: Array<{
    quote: string;
    page: number;
    role: "question" | "mechanism" | "evidence" | "result" | "qualification";
    context_before: string[];
    read_for: string;
  }>;
  reception: Array<{
    kind: "use" | "extension" | "shorthand" | "critique" | "misuse" | "unknown";
    claim: string;
    source_urls: string[];
  }>;
  counterarguments: Array<{
    camp: string | null;
    objection: string;
    target: "assumption" | "method" | "evidence" | "inference" | "scope";
    likely_reply: string;
    source_urls: string[];
  }>;
  uncertainties: string[];
};

export type ExperimentRun = {
  schema_version: number;
  id: string;
  paper_id: string;
  paper_title: string;
  experiment_id: string;
  experiment_name: string;
  question: string;
  reader_baseline: string[];
  provider: AnalysisProvider;
  status: "running" | "completed" | "failed";
  model: string;
  reasoning_effort: string;
  created_at: string;
  completed_at: string | null;
  arms: Array<{
    blind_label: string;
    variant_id: string;
    variant_label: string;
    variant_instruction: string;
    output: LearningRamp | null;
    error: string | null;
  }>;
};

export type ExperimentView = {
  run: ExperimentRun;
  judged: boolean;
};

export type ExperimentJudgment = {
  overall: "A" | "B" | "tie";
  early_traction: "A" | "B" | "tie";
  rigor: "A" | "B" | "tie";
  confidence: number;
  arm_scores: ExperimentArmScore[];
  note: string;
};

export type ExperimentArmScore = {
  blind_label: "A" | "B";
  paper_specificity_actionability: number;
  early_traction: number;
  fidelity_rigor: number;
  dependency_flow: number;
  economy: number;
  provenance_uncertainty: number;
  target_dimension: number | null;
  hard_reject: boolean;
  failure_tags: string[];
};

export type FieldState = "known" | "unknown" | "not_checked" | "conflicting" | "not_applicable";
export type WorkKind = "article" | "book" | "chapter" | "thesis" | "report" | "dataset" | "other";

/** A bibliographic Work as served by the graph API. */
export type Work = {
  id: string;
  kind: WorkKind;
  visibility: "private" | "public";
  display_title: string | null;
  title_state: FieldState;
  display_year: number | null;
  year_state: FieldState;
  display_authors?: string[] | null;
  created_at: string;
  updated_at: string;
};

/** A paper held in the local library that corresponds to a Work. */
export type LocalPaper = {
  paper_id: string;
  title: string;
};

/** Local papers keyed by Work id. Works without a local paper have no entry. */
export type LocalPapersByWork = Record<string, LocalPaper[]>;

export type AcquisitionState =
  | "unresolved" | "identifier_found" | "link_found" | "downloaded" | "mapped" | "unavailable";

/** Acquisition state of a Work as shown on graph nodes. */
export type AcquisitionBrief = {
  state: AcquisitionState;
  unavailable_reason: string | null;
};

/** Brief acquisition states keyed by Work id. Works without a row have no entry. */
export type AcquisitionsByWork = Record<string, AcquisitionBrief>;

export type WorkCitations = {
  work: Work;
  outgoing: Work[];
  incoming: Work[];
  edges: GraphEdge[];
  local_papers: LocalPapersByWork;
  acquisitions: AcquisitionsByWork;
  /** Added in Ramp D; optional so older mocks and servers still parse. */
  marks?: Record<string, MarkView>;
  work_tags?: Record<string, string[]>;
};

/** What the server knows about a Work's open-access acquisition. No paths. */
export type AcquisitionView = {
  state: AcquisitionState;
  unavailable_reason: string | null;
  checked_at: string | null;
  attempts: number;
  last_error: string | null;
  source: string | null;
  pdf_url: string | null;
  bytes: number | null;
  updated_at: string;
};

export type AcquisitionJobState = "idle" | "running" | "succeeded" | "failed";

export type AcquisitionStatus = {
  work_id: string;
  job: { state: AcquisitionJobState; summary?: unknown; message?: string };
  acquisition: AcquisitionView | null;
  /** The library paper holding the copy, once it is in the catalog. */
  paper_id: string | null;
};

export type FetchWorkOptions = {
  retry_unavailable?: boolean;
  model_fallback?: "claude" | "codex";
};

export type EdgeLabelMention = {
  id: string;
  /** The citing paper, only when it is in the library. */
  paper_id: string | null;
  page: number | null;
  passage_start: number | null;
  passage_end: number | null;
  marker_start: number | null;
  marker_end: number | null;
  passage: string | null;
};

export type EdgeLabelOrigin = "ai" | "user" | "imported";
export type EdgeLabelReviewState = "unreviewed" | "accepted" | "rejected" | "disputed";

export type EdgeLabel = {
  id: string;
  relation: string;
  reason: string;
  quote: string | null;
  origin: EdgeLabelOrigin;
  review_state: EdgeLabelReviewState;
  model: string | null;
  created_at: string;
  mention: EdgeLabelMention | null;
};

export type EdgeLabelEdit = {
  reason: string;
  relation?: string;
  quote?: string;
};

export type GraphEdge = {
  citing: string;
  cited: string;
  sources: ("provider" | "local")[];
  mention_count: number;
  labels: EdgeLabel[];
};

export type ProminenceSignals = {
  citing_works_count: number;
  cited_works_count: number;
  cited_by_global_count: number | null;
  display_year: number | null;
  local_available: boolean;
  mention_count: number;
  method_citation_count: number;
  background_citation_count: number;
};

export type ProminenceView = {
  score: number;
  signals: ProminenceSignals;
};

export type WorkMarkName = "pinned" | "downplayed" | "hidden";

export type MarkView = {
  mark: WorkMarkName;
  note: string | null;
  updated_at?: string;
};

export type TagView = {
  id: string;
  name: string;
  color: string;
  kind: string;
  work_count: number;
  created_at: string;
  updated_at: string;
};

export type NeighborClusterKind = "tag" | "decade" | "downplayed" | "hidden";

export type NeighborCluster = {
  key: string;
  kind: NeighborClusterKind;
  label: string;
  count: number;
  work_ids: string[];
  sample_titles: string[];
};

export type NeighborRanking = {
  limit: number | null;
  total: number;
  visible: number;
  pinned: number;
  hidden: number;
  filtered_out: number;
};

export type GraphNeighbors = {
  center: string;
  direction: "references" | "citations" | "both";
  nodes: Work[];
  edges: GraphEdge[];
  local_papers: LocalPapersByWork;
  acquisitions: AcquisitionsByWork;
  /** Ranking fields arrive with Ramp D servers; the UI tolerates their absence. */
  ranking?: NeighborRanking;
  prominence?: Record<string, ProminenceView>;
  marks?: Record<string, MarkView>;
  work_tags?: Record<string, string[]>;
  clusters?: NeighborCluster[];
  /** Expansion state per Work (Ramp E servers); the UI tolerates its absence. */
  expansions?: Record<string, ExpansionView>;
};

export type ExpansionViewState = "expanded" | "unavailable" | "known";

export type ExpansionView = {
  state: ExpansionViewState;
  method: "provider" | "local" | "existing" | null;
  references_count: number;
  unavailable_reason: string | null;
  expanded_at: string | null;
};

export type ExpansionReport = {
  work_id: string;
  cached: boolean;
  state: "expanded" | "unavailable";
  method: "provider" | "local" | "existing" | null;
  unavailable_reason: string | null;
  references: number;
  new_works: number;
  paper_id: string | null;
  warnings: string[];
};

export type WalkReport = {
  seed: string;
  expansions: ExpansionReport[];
  levels: number;
  truncated: boolean;
};

export type ExpansionJob = {
  state: AcquisitionJobState;
  report?: WalkReport;
  message?: string;
};

export type ExpansionStatus = {
  work_id: string;
  job: ExpansionJob;
  expansion: ExpansionView | null;
};

export type ExpansionDirection = "references" | "cited_by";

export type ExpandWorkOptions = {
  /** Defaults to references on the server. */
  direction?: ExpansionDirection;
  depth?: number;
  fanout?: number;
  refresh?: boolean;
  local?: boolean;
};

export type WorkLinkMethod = "identifier" | "title_match" | "manual" | "acquisition";

export type PaperWork = {
  paper_id: string;
  work: Work | null;
  link: {
    method: WorkLinkMethod;
    detail: string | null;
    linked_at: string;
  } | null;
};

export type LiteratureStatus =
  | { kind: "linked" }
  | { kind: "unmatched"; reason: string };

export type LiteratureSummary = {
  status: LiteratureStatus;
  document_id: string;
  work_id: string | null;
  link_method: WorkLinkMethod | null;
  link_detail: string | null;
  openalex_id: string | null;
  references: number;
  new_stub_works: number;
  hydrated_works: number;
  warnings: string[];
};

export type LiteratureOptions = {
  local?: boolean;
  labels?: "claude" | "codex";
  label_limit?: number;
};

/** The counts the server reports for a local-evidence run. */
export type LocalEvidenceSummary = {
  entries: number;
  mentions: number;
  unresolved_mentions: number;
  resolved_entries: number;
  ambiguous_entries: number;
  merged_stubs: number;
  local_edges: number;
  edges_also_from_provider: number;
  warnings: string[];
  labels: Record<string, unknown> | null;
};

export type LiteratureJob = {
  state: "idle" | "running" | "succeeded" | "failed";
  summary?: LiteratureSummary;
  local?: LocalEvidenceSummary;
  message?: string;
};

export type ProvenanceColumnKey =
  | "user"
  | "publisher"
  | "paper"
  | "openalex"
  | "library_metadata"
  | "citing_bibliography";

export type ClaimConfidence = "explicit" | "heuristic" | "model";

export type ProvenanceView = {
  work_id: string;
  /** Trust order, highest first. Only the user column is editable. */
  columns: { key: ProvenanceColumnKey; label: string; editable: boolean }[];
  /** title, year, authors, venue, doi. */
  fields: FieldProvenance[];
};

export type FieldProvenance = {
  field: "title" | "year" | "authors" | "venue" | "doi";
  selected: {
    value: unknown;
    display: string | null;
    state: FieldState;
    claim_id: string | null;
    origin: ProvenanceColumnKey | null;
    confidence: ClaimConfidence | null;
  };
  /** created_at order. */
  claims: ClaimView[];
};

export type ClaimView = {
  id: string;
  origin: ProvenanceColumnKey;
  extractor: string;
  confidence: ClaimConfidence;
  value: unknown;
  /** null when the stored value is unreadable for the field. */
  display: string | null;
  review_state: "unreviewed" | "accepted" | "rejected" | "disputed";
  retracted_at: string | null;
  agreement: "selected" | "agrees" | "disagrees" | "excluded";
  excluded_reason: "rejected" | "retracted" | "unreadable" | null;
  source_record: { id: string; kind: string; source_key: string; retrieved_at: string } | null;
  locator: string | null;
  entry: {
    id: string;
    entry_key: string;
    raw_text: string;
    citing_work_id: string | null;
    citing_title: string | null;
  } | null;
  created_at: string;
};

// ---- Reading lists (spec: docs/reading-lists/spec.md) -------------------------------------

export type ReadingStatus = "unread" | "reading" | "read";
export type ReadingStatusCounts = Record<ReadingStatus, number>;
export type ListOrigin = "manual" | "walk" | "generated";
export type ListNodeKind = "work" | "topic" | "claim";
export type ListEdgeKind = "prerequisite" | "covers" | "subtopic";
export type ListNodeSource = "user" | "walk" | "citation" | "agent";
export type ListEdgeSource = "user" | "agent";
export type ListReview = "accepted" | "proposed";
export type ClaimState = "unverified" | "ambiguous" | "contradicted" | "pending";
export type ContextRelation = "builds_on" | "extends" | "contrasts" | "same_method" | "same_problem";

/** A list as the server lists it (the stored list plus counts). */
export type ListSummaryView = {
  id: string;
  workspace_id: string;
  title: string;
  description: string | null;
  origin: ListOrigin;
  seed_work_id: string | null;
  goal: string | null;
  archived_at: string | null;
  created_at: string;
  updated_at: string;
  seed_title: string | null;
  /** Work and claim nodes. */
  item_count: number;
  topic_count: number;
  claim_count: number;
  proposed_count: number;
  status_counts: ReadingStatusCounts;
};

/** A stored list without counts, as list search returns it. */
export type ListRecord = Omit<
  ListSummaryView,
  "seed_title" | "item_count" | "topic_count" | "claim_count" | "proposed_count" | "status_counts"
>;

/** The fields a claim node carries: what was asserted, as entered. */
export type ClaimFields = {
  title: string;
  authors: string[];
  year: number | null;
  venue: string | null;
  doi: string | null;
  arxiv: string | null;
  raw: string | null;
};

/** A provider record offered for an ambiguous claim. */
export type ClaimCandidate = {
  provider_id: string;
  title: string;
  year: number | null;
  authors: string[];
  doi?: string | null;
};

/** A link from a node to a Work the reader has read (D16). */
export type ContextLink = { work_id: string; relation: ContextRelation; note: string };

export type ListPathStep = { work_id: string; title: string | null };

/** One node of a list, flattened with the display fields the server adds. */
export type ListNodeView = {
  id: string;
  list_id: string;
  kind: ListNodeKind;
  work_id: string | null;
  label: string | null;
  summary: string | null;
  claim: ClaimFields | null;
  claim_state: ClaimState | null;
  candidates: ClaimCandidate[];
  position: number;
  note: string | null;
  rationale: string | null;
  context_links: ContextLink[];
  source: ListNodeSource;
  review: ListReview;
  via_work_id: string | null;
  /** The citation walk through this Work, both ends included; null when not walked. */
  path: string[] | null;
  generation_id: string | null;
  added_at: string;
  updated_at: string;
  work: Work | null;
  /** Counting from 1 in position order. */
  ordinal: number;
  depth: number;
  path_steps: ListPathStep[];
};

export type ListEdgeView = {
  list_id: string;
  from: string;
  to: string;
  kind: ListEdgeKind;
  source: ListEdgeSource;
  review: ListReview;
  created_at: string;
};

export type ReadingStateView = { status: ReadingStatus; changed_at: string | null };

export type ListDetail = {
  list: ListSummaryView;
  nodes: ListNodeView[];
  edges: ListEdgeView[];
  /** Derived (citing, cited) pairs of member Work ids. */
  citations: [string, string][];
  /** Keyed by Work id, for every Work node. */
  reading_states: Record<string, ReadingStateView>;
  local_papers: LocalPapersByWork;
  acquisitions: AcquisitionsByWork;
  expansions: Record<string, ExpansionView>;
};

export type ListCreate = { title: string; description?: string; seed_work_id?: string };
export type ListPatch = { title?: string; description?: string | null; archived?: boolean };
export type ListsQuery = { work?: string; archived?: boolean | "only" };

export type ClaimInput = {
  title: string;
  authors?: string[];
  year?: number | null;
  venue?: string | null;
  doi?: string | null;
  arxiv?: string | null;
  raw?: string | null;
};

/** Exactly one of work_id, paper_id, topic, claim. */
export type ListNodeAdd =
  | { work_id: string; via_work_id?: string; topic_ids?: string[]; note?: string }
  | { paper_id: string; topic_ids?: string[]; note?: string }
  | { topic: { label: string; summary?: string } }
  | { claim: ClaimInput; note?: string };

export type ListNodePatch = {
  note?: string | null;
  label?: string;
  summary?: string | null;
  review?: "accepted";
  resolve_to?: string;
};

/** What deleting a node removed; enough for the client to undo it. */
export type RemovedListNode = {
  node: Omit<ListNodeView, "work" | "ordinal" | "depth" | "path_steps">;
  edges: ListEdgeView[];
  was_seed: boolean;
};

export type ListEdgeRef = { from: string; to: string; kind: ListEdgeKind };

export type ListOrderStrategy = "topological" | "year" | "foundational_first";
export type ListOrderRequest = { node_ids: string[] } | { strategy: ListOrderStrategy };

export type VerifyReport = {
  checked: number;
  resolved: number;
  still_pending: number;
  reclassified: number;
  duplicates: number;
  replaced: { from: string; to: string }[];
};

export type WorkReadingState = { work_id: string; status: ReadingStatus; changed_at: string };

export type ListMembership = {
  list_id: string;
  title: string;
  archived: boolean;
  node_id: string;
  topics: string[];
  note: string | null;
};

export type WorkMemberships = { work_id: string; memberships: ListMembership[] };
export type PaperMemberships = { paper_id: string; work_id: string | null; memberships: ListMembership[] };

export type WorkHit = {
  work_id: string | null;
  paper_id: string | null;
  title: string;
  year: number | null;
  authors: string[];
  source: "library" | "graph" | "provider";
  in_graph: boolean;
  openalex_id: string | null;
  doi: string | null;
};

export type GenerationLevel = "intro" | "survey" | "deep";
export type GenerationState = "running" | "succeeded" | "failed" | "over_budget";

export type GenerateRequest = {
  goal: string;
  /** 8 to 60; the server defaults to 20. */
  size?: number;
  level?: GenerationLevel;
  anchors?: string[];
  into_list?: string;
  budget_usd?: number;
};

export type GenerationStatus = {
  state: GenerationState;
  /** Counts per pass and verification state; shape is owned by the pipeline. */
  report: Record<string, unknown> | null;
  spend_usd: number;
  list_id: string | null;
};

// ---- Reading history ----------------------------------------------------------------------

export type HistoryEventKind =
  | "paper_opened" | "paper_left" | "view_changed" | "section_focused" | "figure_viewed"
  | "page_reached" | "search_run" | "question_asked" | "link_followed" | "digest_opened"
  | "graph_opened" | "work_inspected" | "list_opened" | "list_changed" | "list_generated"
  | "fetch_requested";

/** What a caller records; the queue adds the time, session and sequence number. */
export type HistoryEventDraft = {
  kind: HistoryEventKind;
  paper_id?: string;
  work_id?: string;
  list_id?: string;
  dwell_ms?: number;
  detail?: Record<string, unknown>;
};

/** One event as sent to POST /api/history/events. */
export type HistoryEventInput = HistoryEventDraft & {
  occurred_at: string;
  session_id: string;
  client_seq: number;
};

export type HistoryIngestResult = { accepted: number; duplicates: number; nudged: string[] };

/** One stored event as the timeline returns it. */
export type HistoryEventView = {
  id: number;
  occurred_at: string;
  session_id: string;
  kind: HistoryEventKind;
  paper_id: string | null;
  work_id: string | null;
  list_id: string | null;
  dwell_ms: number | null;
  detail: Record<string, unknown>;
  client_seq: number;
  work_title: string | null;
};

export type HistorySession = {
  session_id: string;
  started_at: string;
  ended_at: string;
  /** Newest first. */
  events: HistoryEventView[];
};

export type HistoryTimeline = { sessions: HistorySession[]; next_before: string | null };

export type HistoryQuery = { before?: string; kinds?: HistoryEventKind[]; q?: string; limit?: number };

export type ReadingPosition = {
  paper_id: string;
  view: string;
  page: number | null;
  offset_utf16: number | null;
  section_id: string | null;
  scroll_ratio: number | null;
  updated_at: string;
};

export type ReadingPositionInput = {
  view: string;
  page?: number | null;
  offset_utf16?: number | null;
  section_id?: string | null;
  scroll_ratio?: number | null;
};

export type SectionDwell = { section_id: string; visits: number; dwell_ms: number };

export type PaperHistorySummary = {
  paper_id: string;
  first_seen: string | null;
  last_seen: string | null;
  visits: number;
  dwell_ms: number;
  sections: SectionDwell[];
  barely_read: boolean;
  position: ReadingPosition | null;
};

export type InspectedWork = {
  work_id: string;
  title: string | null;
  inspects: number;
  last_inspected: string;
};

export type BarelyReadPaper = {
  paper_id: string;
  work_id: string | null;
  first_seen: string;
  last_seen: string;
  visits: number;
  dwell_ms: number;
};
