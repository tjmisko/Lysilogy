//! Bibliography entries: one raw reference as printed in a citing document.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::id::{DocumentVersionId, EntryId, SourceRecordId};

/// A single bibliography item, retained exactly as it appeared.
///
/// The raw text is preserved even after the entry is resolved to a Work, so a
/// reader can always inspect what the citing document actually said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibliographyEntry {
    pub id: EntryId,
    pub citing_version_id: DocumentVersionId,
    pub entry_key: String,
    pub raw_text: String,
    pub page_or_locator: Option<String>,
    pub source_record_id: Option<SourceRecordId>,
    pub created_at: DateTime<Utc>,
}
