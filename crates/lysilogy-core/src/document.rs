//! Reader documents: stable identities and their immutable byte versions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::id::{BlobId, DocumentId, DocumentVersionId, WorkId, WorkVersionId, WorkspaceId};
use crate::state::{DocumentFormat, UploadState, WorkLinkMethod};

/// A workspace-owned document with a stable identity, independent of its path
/// or file contents. Its `work_id` link is optional: a document may be unlinked,
/// and a Work may have no document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub workspace_id: WorkspaceId,
    pub work_id: Option<WorkId>,
    /// The held manifestation, when one is known.
    pub work_version_id: Option<WorkVersionId>,
    pub work_link_method: Option<WorkLinkMethod>,
    pub work_link_detail: Option<String>,
    pub relative_path: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One immutable version of a document's bytes, keyed by content hash.
///
/// Replacing a file creates a new `DocumentVersion`; it never rewrites this one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentVersion {
    pub id: DocumentVersionId,
    pub document_id: DocumentId,
    pub blob_id: Option<BlobId>,
    pub format: DocumentFormat,
    pub sha256: String,
    pub bytes: Option<i64>,
    pub upload_state: UploadState,
    pub created_at: DateTime<Utc>,
}

/// One file discovered by a library scan, before it is reconciled with the
/// database. `relative_path` is relative to the scanned library root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedFile {
    pub relative_path: String,
    pub sha256: String,
    pub bytes: i64,
    pub format: DocumentFormat,
}
