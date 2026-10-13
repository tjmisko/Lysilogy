//! State enumerations shared by domain types, the database, and the API.
//!
//! Every enum serializes as `snake_case`, matching the `text` + `CHECK` columns
//! in the migrations, so the same spelling is used in JSON, SQL, and URLs.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Error returned when a stored or submitted string is not a known enum value.
#[derive(Debug, thiserror::Error)]
#[error("invalid {type_name} value: {value:?}")]
pub struct ParseStateError {
    type_name: &'static str,
    value: String,
}

impl ParseStateError {
    #[must_use]
    pub fn new(type_name: &'static str, value: &str) -> Self {
        Self {
            type_name,
            value: value.to_owned(),
        }
    }
}

/// Generate a `snake_case` string enum with `as_str`, `FromStr`, and `Display`.
macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// The exact string stored in the database and sent over JSON.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl FromStr for $name {
            type Err = ParseStateError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                match text {
                    $($text => Ok(Self::$variant),)+
                    _ => Err(ParseStateError::new(stringify!($name), text)),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

string_enum!(
    /// The broad bibliographic class of a Work.
    WorkKind {
        Article => "article",
        Book => "book",
        Chapter => "chapter",
        Thesis => "thesis",
        Report => "report",
        Dataset => "dataset",
        Other => "other",
    }
);

string_enum!(
    /// Whether a record may be shown outside its owning workspace.
    Visibility {
        Private => "private",
        Public => "public",
    }
);

string_enum!(
    /// How in-text citation markers are printed in a paper.
    CitationStyle {
        Numeric => "numeric",
        Superscript => "superscript",
        AuthorYear => "author_year",
    }
);

string_enum!(
    /// How a bibliography entry was resolved to a Work.
    ResolutionMethod {
        Identifier => "identifier",
        TitleMatch => "title_match",
        LocalStub => "local_stub",
        Merge => "merge",
    }
);

string_enum!(
    /// Per-field provenance state for a claim.
    FieldState {
        Known => "known",
        Unknown => "unknown",
        NotChecked => "not_checked",
        Conflicting => "conflicting",
        NotApplicable => "not_applicable",
    }
);

string_enum!(
    /// Curation state of a claim or AI-generated relationship label.
    ReviewState {
        Unreviewed => "unreviewed",
        Accepted => "accepted",
        Rejected => "rejected",
        Disputed => "disputed",
    }
);

string_enum!(
    /// The namespace of a bibliographic identifier.
    IdentifierScheme {
        Doi => "doi",
        Arxiv => "arxiv",
        Openalex => "openalex",
        SemanticScholar => "semantic_scholar",
        Pmid => "pmid",
        Isbn => "isbn",
        Url => "url",
    }
);

string_enum!(
    /// Whether an identifier claim is trusted, disputed, or withdrawn.
    IdentifierStatus {
        Active => "active",
        Conflicting => "conflicting",
        Retracted => "retracted",
    }
);

string_enum!(
    /// The kind of manifestation a `WorkVersion` represents.
    VersionKind {
        Preprint => "preprint",
        Published => "published",
        Revision => "revision",
        Unknown => "unknown",
    }
);

string_enum!(
    /// How completely a source covers a population.
    Coverage {
        Unknown => "unknown",
        Partial => "partial",
        Complete => "complete",
    }
);

string_enum!(
    /// Lifecycle of a candidate linkage between an entry and a Work.
    ResolutionStatus {
        Candidate => "candidate",
        Accepted => "accepted",
        Rejected => "rejected",
        Ambiguous => "ambiguous",
        Withdrawn => "withdrawn",
    }
);

string_enum!(
    /// Lifecycle of one source's evidence for a citation edge.
    AssertionStatus {
        Asserted => "asserted",
        Accepted => "accepted",
        Rejected => "rejected",
        Withdrawn => "withdrawn",
    }
);

string_enum!(
    /// Who authored a relationship claim or correction.
    RelationshipOrigin {
        Ai => "ai",
        User => "user",
        Imported => "imported",
    }
);

string_enum!(
    /// The kind of recorded resolution decision.
    DecisionKind {
        Merge => "merge",
        Split => "split",
        Distinct => "distinct",
        Correction => "correction",
    }
);

string_enum!(
    /// The container format of a stored document.
    DocumentFormat {
        Pdf => "pdf",
        Epub => "epub",
        Other => "other",
    }
);

string_enum!(
    /// Upload and availability state of a document version's bytes.
    UploadState {
        Pending => "pending",
        Ready => "ready",
        Missing => "missing",
        Failed => "failed",
    }
);

string_enum!(
    /// How a document came to be linked to a Work.
    WorkLinkMethod {
        Identifier => "identifier",
        TitleMatch => "title_match",
        Manual => "manual",
        Acquisition => "acquisition",
    }
);

string_enum!(
    /// How far open-access acquisition of a Work has progressed.
    AcquisitionState {
        Unresolved => "unresolved",
        IdentifierFound => "identifier_found",
        LinkFound => "link_found",
        Downloaded => "downloaded",
        Mapped => "mapped",
        Unavailable => "unavailable",
    }
);

impl AcquisitionState {
    /// Stage order for "is past stage" checks: `Unresolved` is 0 through
    /// `Mapped` at 4; `Unavailable` is a terminal verdict and ranks 5.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Unresolved => 0,
            Self::IdentifierFound => 1,
            Self::LinkFound => 2,
            Self::Downloaded => 3,
            Self::Mapped => 4,
            Self::Unavailable => 5,
        }
    }
}

string_enum!(
    /// Why a Work has no obtainable open-access copy.
    UnavailableReason {
        NoIdentifier => "no_identifier",
        NoOaLocation => "no_oa_location",
        AllLinksFailed => "all_links_failed",
    }
);

string_enum!(
    /// Where an acquisition candidate link came from.
    CandidateOrigin {
        Arxiv => "arxiv",
        OpenalexBest => "openalex_best",
        OpenalexOaUrl => "openalex_oa_url",
        OpenalexLocation => "openalex_location",
        UnpaywallBest => "unpaywall_best",
        UnpaywallLocation => "unpaywall_location",
        OpenalexLanding => "openalex_landing",
        UnpaywallLanding => "unpaywall_landing",
        LandingMeta => "landing_meta",
        Model => "model",
    }
);

string_enum!(
    /// Which bibliographic field a claim populates.
    FieldKind {
        Title => "title",
        Year => "year",
        Author => "author",
        Venue => "venue",
        Type => "type",
        Abstract => "abstract",
        Identifier => "identifier",
        Other => "other",
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_and_as_str_agree() {
        let cases = [
            (
                serde_json::to_string(&FieldState::NotChecked).unwrap(),
                "not_checked",
            ),
            (
                serde_json::to_string(&IdentifierScheme::SemanticScholar).unwrap(),
                "semantic_scholar",
            ),
            (
                serde_json::to_string(&DecisionKind::Correction).unwrap(),
                "correction",
            ),
        ];
        for (json, text) in cases {
            assert_eq!(json, format!("\"{text}\""));
        }
    }

    #[test]
    fn round_trips_every_variant() {
        fn check<T>(variants: &[T])
        where
            T: Copy + fmt::Display + FromStr + PartialEq + fmt::Debug + serde::de::DeserializeOwned,
            T::Err: fmt::Debug,
        {
            for &variant in variants {
                let text = variant.to_string();
                let parsed: T = text.parse().expect("parse");
                assert_eq!(parsed, variant);
                assert_eq!(
                    serde_json::from_str::<T>(&format!("\"{text}\"")).expect("deserialize"),
                    variant
                );
            }
        }

        check(&[
            WorkKind::Article,
            WorkKind::Book,
            WorkKind::Chapter,
            WorkKind::Thesis,
            WorkKind::Report,
            WorkKind::Dataset,
            WorkKind::Other,
        ]);
        check(&[
            FieldState::Known,
            FieldState::Unknown,
            FieldState::NotChecked,
            FieldState::Conflicting,
            FieldState::NotApplicable,
        ]);
        check(&[
            ResolutionStatus::Candidate,
            ResolutionStatus::Accepted,
            ResolutionStatus::Rejected,
            ResolutionStatus::Ambiguous,
            ResolutionStatus::Withdrawn,
        ]);
        check(&[
            IdentifierScheme::Doi,
            IdentifierScheme::Arxiv,
            IdentifierScheme::Openalex,
            IdentifierScheme::SemanticScholar,
            IdentifierScheme::Pmid,
            IdentifierScheme::Isbn,
            IdentifierScheme::Url,
        ]);
    }

    #[test]
    fn should_round_trip_acquisition_state_when_parsing_strings() {
        let states = [
            AcquisitionState::Unresolved,
            AcquisitionState::IdentifierFound,
            AcquisitionState::LinkFound,
            AcquisitionState::Downloaded,
            AcquisitionState::Mapped,
            AcquisitionState::Unavailable,
        ];
        for state in states {
            assert_eq!(state.as_str().parse::<AcquisitionState>().unwrap(), state);
            assert_eq!(
                serde_json::from_str::<AcquisitionState>(&format!("\"{state}\"")).unwrap(),
                state
            );
        }
        for origin in [
            CandidateOrigin::Arxiv,
            CandidateOrigin::OpenalexBest,
            CandidateOrigin::OpenalexOaUrl,
            CandidateOrigin::OpenalexLocation,
            CandidateOrigin::UnpaywallBest,
            CandidateOrigin::UnpaywallLocation,
            CandidateOrigin::OpenalexLanding,
            CandidateOrigin::UnpaywallLanding,
            CandidateOrigin::LandingMeta,
            CandidateOrigin::Model,
        ] {
            assert_eq!(origin.as_str().parse::<CandidateOrigin>().unwrap(), origin);
        }
        assert_eq!(
            "acquisition".parse::<WorkLinkMethod>().unwrap(),
            WorkLinkMethod::Acquisition
        );
    }

    #[test]
    fn should_order_stage_ranks_when_comparing_states() {
        let stages = [
            AcquisitionState::Unresolved,
            AcquisitionState::IdentifierFound,
            AcquisitionState::LinkFound,
            AcquisitionState::Downloaded,
            AcquisitionState::Mapped,
        ];
        for (index, state) in stages.iter().enumerate() {
            assert_eq!(usize::from(state.rank()), index);
        }
        assert_eq!(AcquisitionState::Unavailable.rank(), 5);
    }

    #[test]
    fn should_reject_unknown_unavailable_reason_when_parsing() {
        let error = "paywalled"
            .parse::<UnavailableReason>()
            .expect_err("must fail");
        assert!(error.to_string().contains("UnavailableReason"));
        assert!("".parse::<UnavailableReason>().is_err());
        assert!("No_Identifier".parse::<UnavailableReason>().is_err());
        assert_eq!(
            "no_oa_location".parse::<UnavailableReason>().unwrap(),
            UnavailableReason::NoOaLocation
        );
    }

    #[test]
    fn unknown_values_are_rejected_with_the_type_name() {
        let error = "bogus".parse::<FieldState>().expect_err("must fail");
        assert!(error.to_string().contains("FieldState"));
        assert!(error.to_string().contains("bogus"));
    }
}
