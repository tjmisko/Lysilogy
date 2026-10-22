//! Structured records from printed reference strings and fuzzy candidate
//! matching.
//!
//! A printed reference is the unit of evidence. Its parsed fields are fallible
//! interpretations, so matching carries a per-signal score and an explicit
//! decision rather than a single boolean. Nothing here performs I/O: registry
//! adapters produce [`CandidateWork`] values and callers apply [`decide`].

mod adapters;
mod candidate;
mod record;
mod registry;
mod score;

pub use adapters::{
    CrossrefRegistry, OpenAlexRegistry, SemanticScholarRegistry, crossref_search_url,
    parse_crossref_candidates, parse_openalex_candidates, parse_semantic_scholar_candidates,
    semantic_scholar_search_url,
};
pub use candidate::CandidateRecord;
pub use record::{IdentifierLevel, ReferenceIdentifier, ReferenceRecord, identifier_level};
pub use registry::{
    ReferenceRegistry, RegistryCapabilities, RegistryError, RegistryFuture, RegistrySearch,
    Resolution, dedup_versions, resolve,
};
pub use score::{
    ACCEPT_MARGIN, ACCEPT_SCORE, CANDIDATE_SCORE, IdentifierAgreement, MatchDecision, MatchScore,
    RejectionReason, ScoredCandidate, TITLE_FLOOR, decide, rank, score_candidate,
};
