//! The core fields, each asked for on its own over a shared paper prefix.
//!
//! One call per field keeps each field's output budget to itself, and because
//! every call after the first carries the same paper bytes, a provider with
//! prefix caching can serve the repeated prefix from its cache.

use super::{
    ABSTRACT_SCHEMA, CARD_SCHEMA, CITATIONS_SCHEMA, IDENTITY_SCHEMA, QUESTION_SCHEMA,
    REFERENCES_SCHEMA, RELATIONS_SCHEMA, SECTIONS_SCHEMA, TYPE_SCHEMA,
};

/// One core field and the call that fills it.
pub struct CoreField {
    pub name: &'static str,
    pub stage: &'static str,
    pub schema: &'static str,
    pub instruction: &'static str,
    /// How many leading pages to send; `None` sends the whole paper.
    pub page_limit: Option<usize>,
}

/// The core fields, in the order they run.
pub const CORE_FIELDS: &[CoreField] = &[
    CoreField {
        name: "identity",
        stage: "extract.identity",
        schema: IDENTITY_SCHEMA,
        instruction: "Return the `identity` field: title and authors exactly as printed on the first page, authors in printed order, without affiliations or footnote marks. Year, venue, and identifiers only when the paper states them.",
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
        instruction: "Return the `abstract` field: the abstract copied verbatim, paragraphs separated by a blank line. Null only if these pages contain no abstract.",
        page_limit: Some(3),
    },
    CoreField {
        name: "card",
        stage: "extract.card",
        schema: CARD_SCHEMA,
        instruction: "Return the `card` field: the thesis in one plain sentence; the position (what it builds on, contrasts with, critiques, replicates), each with a one-line why.",
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
        name: "sections",
        stage: "extract.sections",
        schema: SECTIONS_SCHEMA,
        instruction: "Return the `sections` field: the reading map.\n{{sections}}",
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
