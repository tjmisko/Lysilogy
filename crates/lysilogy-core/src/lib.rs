//! Domain types for the Lysilogy literature viewer.
//!
//! This crate holds pure data types and identifiers and performs no I/O. Database,
//! ingestion, and API concerns live in the sibling crates.

pub mod document;
pub mod entry;
pub mod id;
pub mod identifier;
pub mod state;
pub mod work;

pub use document::{Document, DocumentVersion, ScannedFile};
pub use entry::BibliographyEntry;
pub use identifier::{DetectedIdentifier, extract as extract_identifiers};
pub use id::{
    AssertionId, BlobId, DocumentId, DocumentVersionId, EntryId, IdError, IdentifierId, MentionId,
    SourceRecordId, WorkId, WorkVersionId, WorkspaceId,
};
pub use state::{
    AssertionStatus, Coverage, DecisionKind, DocumentFormat, FieldKind, FieldState, IdentifierScheme,
    IdentifierStatus, ParseStateError, RelationshipOrigin, ResolutionStatus, ReviewState,
    UploadState, VersionKind, Visibility, WorkKind,
};
pub use work::{Work, WorkIdentifier, WorkVersion};
