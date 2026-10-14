//! Domain types for the Lysilogy literature viewer.
//!
//! This crate holds pure data types and identifiers and performs no I/O. Database,
//! ingestion, and API concerns live in the sibling crates.

pub mod curation;
pub mod document;
pub mod entry;
pub mod expansion;
pub mod id;
pub mod identifier;
pub mod names;
pub mod prominence;
pub mod provenance;
pub mod ramp;
pub mod state;
pub mod title;
pub mod work;

pub use curation::{CurationError, TAG_PALETTE, TagColor, TagName, parse_mark_note};
pub use document::{Document, DocumentVersion, ScannedFile};
pub use entry::BibliographyEntry;
pub use expansion::{
    WALK_DEPTH_MAX, WALK_EXPANSIONS_MAX, WALK_FANOUT_MAX, WalkBounds, WalkBoundsError,
};
pub use id::{
    AssertionId, BlobId, DocumentId, DocumentVersionId, EntryId, FieldClaimId, IdError,
    IdentifierId, MentionId, RampId, RelationshipClaimId, SourceRecordId, TagId, WorkId,
    WorkVersionId, WorkspaceId,
};
pub use identifier::{DetectedIdentifier, extract as extract_identifiers, front_matter};
pub use names::first_author_family;
pub use prominence::{
    PROMINENCE_SIGNALS_VERSION, ProminenceComponents, ProminenceSignals, prominence_components,
    prominence_score,
};
pub use ramp::{
    RAMP_ITEMS_MAX, RAMP_PATH_MAX, RAMP_TITLE_FALLBACK, RAMP_TITLE_MAX_CHARS, RampError,
    RampOrderItem, RampTitle, default_ramp_title, suggest_order,
};
pub use state::{
    AcquisitionState, AssertionStatus, CandidateOrigin, CitationStyle, ClaimConfidence,
    ClaimOrigin, Coverage, DecisionKind, DocumentFormat, ExpansionMethod, ExpansionState,
    ExpansionUnavailableReason, FieldKind, FieldState, IdentifierScheme, IdentifierStatus,
    ParseStateError, ProvenanceField, RampItemStatus, RelationshipOrigin, ResolutionMethod,
    ResolutionStatus, ReviewState, TagKind, UnavailableReason, UploadState, VersionKind,
    Visibility, WorkKind, WorkLinkMethod, WorkMark,
};
pub use title::{title_key, title_similarity};
pub use work::{Work, WorkIdentifier, WorkVersion};
