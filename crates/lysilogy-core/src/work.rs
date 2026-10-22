//! Bibliographic entities: `Work`, its identifiers, and its versions.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::id::{IdentifierId, SourceRecordId, WorkId, WorkVersionId};
use crate::state::{
    FieldState, IdentifierScheme, IdentifierStatus, VersionKind, Visibility, WorkKind,
};

/// A bibliographic thing that can be cited.
///
/// A Work may exist from a bare reference string and need not correspond to a
/// document Lysilogy holds. `display_title` and `display_year` are the selected,
/// curated values; their provenance lives in `field_claim` rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub id: WorkId,
    pub kind: WorkKind,
    pub visibility: Visibility,
    pub display_title: Option<String>,
    pub title_state: FieldState,
    pub display_year: Option<i32>,
    pub year_state: FieldState,
    /// Selected printed author strings in order; see `provenance::select_field`.
    pub display_authors: Option<Vec<String>>,
    pub authors_state: FieldState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One scheme-specific identifier claimed for a Work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkIdentifier {
    pub id: IdentifierId,
    pub work_id: WorkId,
    pub scheme: IdentifierScheme,
    pub value_normalized: String,
    pub source_record_id: Option<SourceRecordId>,
    pub status: IdentifierStatus,
    pub created_at: DateTime<Utc>,
}

/// One manifestation of a Work: a preprint, the published edition, a revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkVersion {
    pub id: WorkVersionId,
    pub work_id: WorkId,
    pub version_kind: VersionKind,
    pub version_label: Option<String>,
    pub published_date: Option<NaiveDate>,
    pub date_state: FieldState,
    pub created_at: DateTime<Utc>,
}

/// One scheme-specific identifier claimed for a Work version.
///
/// DOI, arXiv, PMID, ISBN and URL name a manifestation, so they live here
/// rather than on the Work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkVersionIdentifier {
    pub id: IdentifierId,
    pub version_id: WorkVersionId,
    pub scheme: IdentifierScheme,
    pub value_normalized: String,
    pub source_record_id: Option<SourceRecordId>,
    pub status: IdentifierStatus,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::WorkId;

    #[test]
    fn work_serializes_ids_and_enums_as_strings() {
        let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp");
        let work = Work {
            id: WorkId::new(),
            kind: WorkKind::Article,
            visibility: Visibility::Private,
            display_title: Some("Attention Is All You Need".to_owned()),
            title_state: FieldState::Known,
            display_year: Some(2017),
            year_state: FieldState::Known,
            display_authors: Some(vec!["Ashish Vaswani".to_owned()]),
            authors_state: FieldState::Known,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_value(&work).expect("serialize");
        assert_eq!(json["kind"], "article");
        assert_eq!(json["visibility"], "private");
        assert_eq!(json["title_state"], "known");
        assert_eq!(json["display_year"], 2017);
        assert!(
            json["id"]
                .as_str()
                .expect("id is a string")
                .starts_with('w')
        );

        let back: Work = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, work);
    }

    #[test]
    fn unknown_fields_are_optional_nulls() {
        let now = DateTime::<Utc>::from_timestamp(0, 0).expect("timestamp");
        let work = Work {
            id: WorkId::new(),
            kind: WorkKind::Other,
            visibility: Visibility::Private,
            display_title: None,
            title_state: FieldState::Unknown,
            display_year: None,
            year_state: FieldState::Unknown,
            display_authors: None,
            authors_state: FieldState::Unknown,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_value(&work).expect("serialize");
        assert!(json["display_title"].is_null());
        assert!(json["display_year"].is_null());
        let back: Work = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, work);
    }
}
