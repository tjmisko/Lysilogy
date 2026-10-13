//! Domain types for the Lysilogy literature viewer.
//!
//! This crate holds pure data types and identifiers and performs no I/O. Database,
//! ingestion, and API concerns live in the sibling crates.

pub mod curation;
pub mod document;
pub mod entry;
pub mod id;
pub mod identifier;
pub mod names;
pub mod prominence;
pub mod state;
pub mod title;
pub mod work;

pub use curation::{CurationError, TAG_PALETTE, TagColor, TagName, parse_mark_note};
pub use document::{Document, DocumentVersion, ScannedFile};
pub use entry::BibliographyEntry;
pub use id::{
    AssertionId, BlobId, DocumentId, DocumentVersionId, EntryId, IdError, IdentifierId, MentionId,
    RelationshipClaimId, SourceRecordId, TagId, WorkId, WorkVersionId, WorkspaceId,
};
pub use identifier::{DetectedIdentifier, extract as extract_identifiers, front_matter};
pub use names::first_author_family;
pub use prominence::{
    PROMINENCE_SIGNALS_VERSION, ProminenceComponents, ProminenceSignals, prominence_components,
    prominence_score,
};
pub use state::{
    AcquisitionState, AssertionStatus, CandidateOrigin, CitationStyle, Coverage, DecisionKind,
    DocumentFormat, FieldKind, FieldState, IdentifierScheme, IdentifierStatus, ParseStateError,
    RelationshipOrigin, ResolutionMethod, ResolutionStatus, ReviewState, TagKind,
    UnavailableReason, UploadState, VersionKind, Visibility, WorkKind, WorkLinkMethod, WorkMark,
};
pub use title::{title_key, title_similarity};
pub use work::{Work, WorkIdentifier, WorkVersion};
