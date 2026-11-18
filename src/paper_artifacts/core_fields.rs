//! The core fields, each asked for on its own over a shared paper prefix.
//!
//! One call per field keeps each field's output budget to itself, and because
//! every call after the first carries the same paper bytes, a provider with
//! prefix caching can serve the repeated prefix from its cache.

use super::{
    ABSTRACT_SCHEMA, CARD_SCHEMA, CITATIONS_SCHEMA, IDENTITY_SCHEMA, OUTLINE_SCHEMA,
    QUESTION_SCHEMA, REFERENCES_SCHEMA, RELATIONS_SCHEMA, SECTIONS_SCHEMA, TYPE_SCHEMA,
};

/// What the abstract is and is not. The reader shows it as the authors'
/// abstract, and a deterministic extraction cross-checks it, so a summary
/// in its place is worse than nothing.
pub const ABSTRACT_INSTRUCTION: &str = "Return the `abstract` field: the authors' own abstract, copied exactly as printed, paragraphs separated by a blank line. \
The reader is shown this as the authors' words, so it must be their text and only their text: start at the first sentence of the abstract and stop at its last, \
leaving out the heading, keywords, classification codes, a cite-as line, footnotes and the start of the introduction. Do not summarize, shorten, fix or paraphrase it. \
Papers that print no labelled abstract sometimes open with an unlabelled summary paragraph set apart from the main text; copy that if it is plainly the abstract. \
Null only if the paper has no abstract.";

/// The thesis is the first line a reader sees; it must carry the finding.
pub const CARD_INSTRUCTION: &str = "Return the `card` field. \
The thesis is the first sentence a reader sees about this paper, before the abstract, so it has to carry the finding: one plain sentence that says what the paper shows, \
claims or builds, specific enough that it could not describe a different paper. State the result, not the topic: \
for an illustrative study from medicine, \"We study hand-washing and maternal mortality\" is a topic, while \"Requiring doctors to wash their hands in chlorinated lime cut maternal deaths in the first clinic from about 10% to under 2%\" is a finding. \
Use the paper's own hedges (a paper that suggests should not be made to prove), avoid jargon an outsider would not know when a plain word will do, and leave out hype. \
Then the position: the works it builds on, contrasts with or critiques, and replicates, each with a one-line why.";

/// One core field and the call that fills it.
pub struct CoreField {
    pub name: &'static str,
    pub stage: &'static str,
    pub schema: &'static str,
    pub instruction: &'static str,
    /// How many leading pages to send; `None` sends the whole paper.
    pub page_limit: Option<usize>,
}

/// The core fields, in the order they run. The outline comes before the
/// sections because the logical units are grouped from it.
pub const CORE_FIELDS: &[CoreField] = &[
    CoreField {
        name: "identity",
        stage: "extract.identity",
        schema: IDENTITY_SCHEMA,
        instruction: "Return the `identity` field: title and authors exactly as printed on the first page, authors in printed order, without affiliations or footnote marks. Year, venue, and identifiers only when the paper states them. Use the paper's own title heading and byline; ignore a cite-as note, running heads, journal cover lines, and accepted-for-publication banners.",
        page_limit: None,
    },
    CoreField {
        name: "type",
        stage: "extract.type",
        schema: TYPE_SCHEMA,
        instruction: "Return the `type` field: the kind of paper, and a one-line reason.",
        page_limit: None,
    },
    CoreField {
        name: "abstract",
        stage: "extract.abstract",
        schema: ABSTRACT_SCHEMA,
        instruction: ABSTRACT_INSTRUCTION,
        page_limit: Some(3),
    },
    CoreField {
        name: "card",
        stage: "extract.card",
        schema: CARD_SCHEMA,
        instruction: CARD_INSTRUCTION,
        page_limit: None,
    },
    CoreField {
        name: "question",
        stage: "extract.question",
        schema: QUESTION_SCHEMA,
        instruction: "Return the `question` field: the research question or hypothesis.",
        page_limit: None,
    },
    CoreField {
        name: "outline",
        stage: "extract.outline",
        schema: OUTLINE_SCHEMA,
        instruction: "Return the `outline` field: the paper's printed headings.\n{{outline}}",
        page_limit: None,
    },
    CoreField {
        name: "sections",
        stage: "extract.sections",
        schema: SECTIONS_SCHEMA,
        instruction: "Return the `sections` field: the logical reading units.\n{{sections}}",
        page_limit: None,
    },
    CoreField {
        name: "references",
        stage: "extract.references",
        schema: REFERENCES_SCHEMA,
        instruction: "Return the `references` field: every bibliography entry, in order, with the in-text key the paper uses (`12`, or `Smith et al., 2020`). Fill fields only as printed.",
        page_limit: None,
    },
    CoreField {
        name: "citations",
        stage: "extract.citations",
        schema: CITATIONS_SCHEMA,
        instruction: "Return the `citations` field: in-text citations with the citing sentence verbatim.",
        page_limit: None,
    },
    CoreField {
        name: "relations",
        stage: "extract.relations",
        schema: RELATIONS_SCHEMA,
        instruction: "Return the `relations` field: for the cited works that matter to the argument, the relation from the given vocabulary, a plain-language why, and a verbatim quote.",
        page_limit: None,
    },
];
