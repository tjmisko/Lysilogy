//! Domain types for the Lysilogy literature viewer.
//!
//! This crate holds pure data types and identifiers and performs no I/O. Database,
//! ingestion, and API concerns live in the sibling crates.

pub mod document;
pub mod entry;
pub mod id;
pub mod identifier;
pub mod names;
pub mod state;
pub mod title;
pub mod work;

pub use document::{Document, DocumentVersion, ScannedFile};
pub use entry::BibliographyEntry;
pub use id::{
    AssertionId, BlobId, DocumentId, DocumentVersionId, EntryId, IdError, IdentifierId, MentionId,
    SourceRecordId, WorkId, WorkVersionId, WorkspaceId,
};
pub use identifier::{DetectedIdentifier, extract as extract_identifiers, front_matter};
pub use names::first_author_family;
pub use state::{
    AssertionStatus, CitationStyle, Coverage, DecisionKind, DocumentFormat, FieldKind, FieldState,
    IdentifierScheme, IdentifierStatus, ParseStateError, RelationshipOrigin, ResolutionMethod,
    ResolutionStatus, ReviewState, UploadState, VersionKind, Visibility, WorkKind, WorkLinkMethod,
};
pub use title::{title_key, title_similarity};
pub use work::{Work, WorkIdentifier, WorkVersion};
