//! Stable JSON contracts for the rebuildable knowledge base.
//!
//! IDs are allocated once by the store and retained in canonical records. They
//! must never be derived again from a mutable title, name, or file location.

use std::{collections::BTreeMap, collections::BTreeSet, fmt, str::FromStr};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub use crate::citation_graph::Identifier;
use crate::{
    citation_graph::Provider,
    domain::{AnalysisProvider, CitationStatus, PaperId, TextAnchor},
};

macro_rules! entity_id {
    ($name:ident, $prefix:literal) => {
        /// An opaque, permanently allocated entity ID, serialized as a string.
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let suffix = value.strip_prefix($prefix).ok_or_else(|| {
                    concat!(stringify!($name), " must start with ", $prefix).to_owned()
                })?;
                if suffix.is_empty()
                    || value.len() > 128
                    || !suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                {
                    return Err(concat!(stringify!($name), " has an invalid ID token").to_owned());
                }
                Ok(Self(value.to_owned()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

entity_id!(WorkId, "W");
entity_id!(PersonId, "P");

/// Bibliographic identifiers are shared with the provider adapters. Work-level
/// arXiv identifiers are versionless; version-specific IDs live on `WorkVersion`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Work {
    pub id: WorkId,
    pub identifiers: BTreeSet<Identifier>,
    pub title: Option<String>,
    pub title_key: Option<String>,
    pub year: Option<u16>,
    pub venue: Option<String>,
    pub kind: WorkType,
    pub versions: Vec<WorkVersion>,
    pub local_copies: Vec<LocalCopy>,
    pub acquisition: AcquisitionState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkType {
    JournalArticle,
    ConferencePaper,
    Preprint,
    Book,
    BookChapter,
    Thesis,
    Report,
    Dataset,
    Other,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkVersion {
    /// Stable within the Work; does not encode a mutable publication date.
    pub id: String,
    pub kind: VersionKind,
    pub label: Option<String>,
    pub identifiers: BTreeSet<Identifier>,
    pub date: Option<NaiveDate>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    Preprint,
    Published,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalCopy {
    pub paper_id: PaperId,
    /// SHA-256 of the PDF bytes, independent of the current paper path.
    pub content_hash: String,
    pub version_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub display_name: String,
    /// Unresolved name order leaves the parsed components empty.
    pub family_name: Option<String>,
    pub given_names: Vec<String>,
    pub particles: Vec<String>,
    pub suffix: Option<String>,
    pub name_variants: Vec<NameVariant>,
    pub identifiers: BTreeSet<PersonIdentifier>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NameVariant {
    pub name: String,
    pub count: u32,
}

/// Author identifiers cannot be confused with a provider's Work identifiers.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "scheme", content = "value", rename_all = "snake_case")]
pub enum PersonIdentifier {
    Orcid(String),
    OpenalexAuthor(String),
    SemanticScholarAuthor(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Authorship {
    pub work_id: WorkId,
    pub person_id: PersonId,
    /// Zero-based position in the printed author list.
    pub position: u32,
    pub raw_name: String,
}

/// A directed bibliographic edge; it never establishes influence or support.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Citation {
    pub citing: WorkId,
    pub cited: WorkId,
    /// One record per local entry or provider source; sources stay distinct.
    pub evidence: Vec<CitationEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum CitationEvidence {
    Local {
        paper_id: PaperId,
        bibliography_entry_id: String,
        mentions: Vec<CitationMention>,
    },
    Provider {
        provider: Provider,
        retrieved_at: DateTime<Utc>,
        provider_edge_id: Option<String>,
        /// Unverified provider passages, never promoted to local source anchors.
        passages: Vec<String>,
        /// Provider-generated labels, not ground truth about the edge.
        intents: Vec<String>,
        is_influential: Option<bool>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CitationMention {
    pub sentence: String,
    pub anchor: Option<CitationAnchor>,
    pub validation: CitationStatus,
}

/// Layout tokens and reading-index offsets are different coordinate systems.
/// Consumers must resolve the selected artifact without casting between them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CitationAnchor {
    Layout {
        anchor: TextAnchor,
    },
    ReadingIndex {
        /// Fingerprint of the reading-index generation that supplied the range.
        generation: String,
        page: u32,
        /// Half-open UTF-16 code-unit range in the reading index's complete text.
        start: usize,
        end: usize,
    },
}

/// Raw observed data is retained independently of resolved entity projections.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub source: ObservationSource,
    pub retrieved_at: DateTime<Utc>,
    pub payload: ObservationPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservationSource {
    Bibliography {
        paper_id: PaperId,
        entry_id: String,
    },
    PdfMetadata {
        paper_id: PaperId,
    },
    Provider {
        provider: Provider,
        record_id: String,
    },
    AiProposal {
        provider: AnalysisProvider,
        model: String,
        generation_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ObservationPayload {
    Text(String),
    Json(Value),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub recorded_at: DateTime<Utc>,
    pub rationale: String,
    pub action: DecisionAction,
}

/// A decision cannot merge, split, or compare entities of different kinds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "entity", content = "decision", rename_all = "snake_case")]
pub enum DecisionAction {
    Work(EntityDecision<WorkId>),
    Person(EntityDecision<PersonId>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EntityDecision<I> {
    Merge {
        surviving: I,
        absorbed: I,
    },
    Split {
        original: I,
        created: I,
        observation_ids: Vec<String>,
    },
    Distinct {
        left: I,
        right: I,
    },
    AutomaticMerge {
        surviving: I,
        absorbed: I,
        score: f64,
        matcher_version: String,
    },
}

/// Each completed stage carries its provenance. The job layer retains this
/// state's history, including stages completed before an unavailable result.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AcquisitionState {
    #[default]
    Unresolved,
    IdentifierFound {
        identifiers: BTreeSet<Identifier>,
        provenance: AcquisitionProvenance,
    },
    LinkFound {
        url: String,
        provenance: AcquisitionProvenance,
    },
    Downloaded {
        local_copy: LocalCopy,
        provenance: AcquisitionProvenance,
    },
    Mapped {
        local_copy: LocalCopy,
        provenance: AcquisitionProvenance,
    },
    Unavailable {
        reason: String,
        checked_at: DateTime<Utc>,
        source: AcquisitionSource,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcquisitionProvenance {
    pub source: AcquisitionSource,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcquisitionSource {
    Local {
        paper_id: PaperId,
    },
    Provider {
        provider: Provider,
        record_id: String,
    },
    OpenAccess {
        locator: String,
        source_url: String,
    },
    Agent {
        provider: AnalysisProvider,
        model: String,
        generation_id: String,
    },
    User,
}

/// An acyclic, deterministically serialized alias projection for one entity kind.
/// Persistence and decision replay are owned by the store/decision-log layer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AliasMap<I: Ord> {
    links: BTreeMap<I, I>,
}

impl<I: Ord> Default for AliasMap<I> {
    fn default() -> Self {
        Self {
            links: BTreeMap::new(),
        }
    }
}

impl<I: Ord + Clone> AliasMap<I> {
    /// Unknown IDs resolve to themselves; entity existence is checked by the store.
    #[must_use]
    pub fn resolve<'a>(&'a self, id: &'a I) -> &'a I {
        let mut current = id;
        while let Some(next) = self.links.get(current) {
            current = next;
        }
        current
    }

    /// Link the absorbed entity's surviving ID to the requested survivor. Repeated
    /// merges within the same component are no-ops and cannot introduce cycles.
    pub fn merge(&mut self, absorbed: &I, surviving: &I) {
        let absorbed = self.resolve(absorbed).clone();
        let surviving = self.resolve(surviving).clone();
        if absorbed != surviving {
            self.links.insert(absorbed, surviving);
        }
    }

    #[must_use]
    pub const fn links(&self) -> &BTreeMap<I, I> {
        &self.links
    }
}

impl<'de, I: Ord + Clone + Deserialize<'de>> Deserialize<'de> for AliasMap<I> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let links = BTreeMap::<I, I>::deserialize(deserializer)?;
        let mut checked = BTreeSet::new();
        for id in links.keys() {
            let mut visiting = BTreeSet::new();
            let mut current = id;
            while !checked.contains(current) {
                let Some(next) = links.get(current) else {
                    break;
                };
                if !visiting.insert(current) {
                    return Err(serde::de::Error::custom("entity aliases contain a cycle"));
                }
                current = next;
            }
            checked.extend(visiting);
        }
        Ok(Self { links })
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use serde_json::json;

    use super::*;

    fn round_trip<T: DeserializeOwned + Serialize>(expected: &Value) {
        let value: T = serde_json::from_value(expected.clone()).expect("valid fixture");
        let encoded = serde_json::to_string(&value).expect("serialize fixture");
        assert_eq!(&serde_json::from_str::<Value>(&encoded).unwrap(), expected);
        let decoded: T = serde_json::from_str(&encoded).expect("round-trip fixture");
        assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded);
    }

    fn local_copy() -> Value {
        json!({
            "paper_id": "0123456789abcdef",
            "content_hash": "a".repeat(64),
            "version_id": "v1"
        })
    }

    fn provenance() -> Value {
        json!({"source": {"kind": "user"}, "recorded_at": "2026-09-12T12:00:00Z"})
    }

    #[test]
    fn should_round_trip_ids_when_tokens_are_valid() {
        round_trip::<WorkId>(&json!("W0123456789abcdef"));
        round_trip::<PersonId>(&json!("Pperson-123_abc"));
        let id: WorkId = "W1".parse().unwrap();
        assert_eq!(id.to_string(), "W1");
        assert_eq!(id.as_str(), "W1");
    }

    #[test]
    fn should_reject_ids_when_prefix_or_token_is_invalid() {
        for value in ["", "W", "P1", "w1", "W../1", "W one", "Wä", "W\n1"] {
            assert!(value.parse::<WorkId>().is_err(), "{value:?}");
            assert!(serde_json::from_value::<WorkId>(json!(value)).is_err());
        }
        for value in ["", "P", "W1", "p1", "P/a", "P\u{0000}1"] {
            assert!(value.parse::<PersonId>().is_err(), "{value:?}");
            assert!(serde_json::from_value::<PersonId>(json!(value)).is_err());
        }
        assert!(format!("W{}", "a".repeat(128)).parse::<WorkId>().is_err());
    }

    #[test]
    fn should_reuse_identifier_parsing_when_provider_schemes_are_present() {
        for input in [
            "https://doi.org/10.1234/example",
            "arxiv:2608.00001v2",
            "openalex:W123",
            "s2:abc123",
            "pmid:12345",
            "omid:br/123",
        ] {
            let identifier: crate::citation_graph::Identifier = Identifier::parse(input).unwrap();
            round_trip::<Identifier>(&serde_json::to_value(identifier).unwrap());
        }
    }

    #[test]
    fn should_round_trip_work_when_versions_and_local_copies_exist() {
        let version = json!({
            "id": "v1", "kind": "preprint", "label": "arXiv v2",
            "identifiers": [{"scheme": "arxiv", "value": "2608.00001v2"}],
            "date": "2026-08-02"
        });
        round_trip::<WorkVersion>(&version);
        round_trip::<LocalCopy>(&local_copy());
        round_trip::<Work>(&json!({
            "id": "W1", "identifiers": [{"scheme": "arxiv", "value": "2608.00001"}],
            "title": "Example paper", "title_key": "example paper", "year": 2026,
            "venue": null, "kind": "preprint", "versions": [version],
            "local_copies": [local_copy()], "acquisition": {"state": "unresolved"}
        }));
        for kind in [
            "journal_article",
            "conference_paper",
            "preprint",
            "book",
            "book_chapter",
            "thesis",
            "report",
            "dataset",
            "other",
            "unknown",
        ] {
            round_trip::<WorkType>(&json!(kind));
        }
        for kind in ["preprint", "published", "other"] {
            round_trip::<VersionKind>(&json!(kind));
        }
    }

    #[test]
    fn should_round_trip_person_when_variants_and_author_identifiers_exist() {
        let variant = json!({"name": "van der Waals, J. D.", "count": 3});
        round_trip::<NameVariant>(&variant);
        let identifiers = json!([
            {"scheme": "orcid", "value": "0000-0002-1825-0097"},
            {"scheme": "openalex_author", "value": "A123"},
            {"scheme": "semantic_scholar_author", "value": "456"}
        ]);
        for identifier in identifiers.as_array().unwrap() {
            round_trip::<PersonIdentifier>(identifier);
        }
        round_trip::<Person>(&json!({
            "id": "P1", "display_name": "J. D. van der Waals", "family_name": "Waals",
            "given_names": ["J.", "D."], "particles": ["van", "der"], "suffix": null,
            "name_variants": [variant], "identifiers": identifiers
        }));
        round_trip::<Authorship>(&json!({
            "work_id": "W1", "person_id": "P1", "position": 0,
            "raw_name": "J. D. van der Waals"
        }));
    }

    #[test]
    fn should_round_trip_citation_when_local_and_provider_evidence_coexist() {
        let mention = json!({
            "sentence": "See [1].", "validation": "exact",
            "anchor": {
                "kind": "layout", "anchor": {
                    "page": 1, "start_token": 3, "end_token": 5, "sentence_ids": ["p1s1"],
                    "rects": [{"x_min": 1.0, "y_min": 2.0, "x_max": 3.0, "y_max": 4.0}],
                    "exact_text": "See [1]."
                }
            }
        });
        round_trip::<CitationAnchor>(&mention["anchor"]);
        round_trip::<CitationAnchor>(&json!({
            "kind": "reading_index", "generation": "index-generation-1",
            "page": 1, "start": 10, "end": 18
        }));
        round_trip::<CitationMention>(&mention);
        let local = json!({
            "source": "local", "paper_id": "0123456789abcdef",
            "bibliography_entry_id": "ref-1", "mentions": [mention]
        });
        let provider = json!({
            "source": "provider", "provider": "semantic_scholar",
            "retrieved_at": "2026-09-12T12:00:00Z", "provider_edge_id": "edge-1",
            "passages": ["A provider-supplied passage."], "intents": ["background"],
            "is_influential": true
        });
        round_trip::<CitationEvidence>(&local);
        round_trip::<CitationEvidence>(&provider);
        round_trip::<Citation>(
            &json!({"citing": "W1", "cited": "W2", "evidence": [local, provider]}),
        );
        round_trip::<CitationMention>(&json!({
            "sentence": "A missing anchor.", "anchor": null, "validation": "missing"
        }));
    }

    #[test]
    fn should_round_trip_observations_when_sources_and_raw_payloads_differ() {
        let sources = [
            json!({"kind": "bibliography", "paper_id": "0123456789abcdef", "entry_id": "ref-1"}),
            json!({"kind": "pdf_metadata", "paper_id": "0123456789abcdef"}),
            json!({"kind": "provider", "provider": "crossref", "record_id": "10.1234/example"}),
            json!({"kind": "ai_proposal", "provider": "codex", "model": "fixture", "generation_id": "run-1"}),
        ];
        for source in sources {
            round_trip::<ObservationSource>(&source);
            for payload in [
                json!({"kind": "text", "value": "Untrusted raw reference [1]."}),
                json!({"kind": "json", "value": {"title": ["Raw title"], "year": 2026}}),
            ] {
                round_trip::<ObservationPayload>(&payload);
                round_trip::<Observation>(&json!({
                    "id": "observation-1", "source": source,
                    "retrieved_at": "2026-09-12T12:00:00Z", "payload": payload
                }));
            }
        }
    }

    #[test]
    fn should_round_trip_decisions_when_every_entity_and_action_is_recorded() {
        for (entity, prefix) in [("work", "W"), ("person", "P")] {
            let first = format!("{prefix}1");
            let second = format!("{prefix}2");
            for decision in [
                json!({"action": "merge", "surviving": first, "absorbed": second}),
                json!({"action": "split", "original": first, "created": second, "observation_ids": ["o1"]}),
                json!({"action": "distinct", "left": first, "right": second}),
                json!({"action": "automatic_merge", "surviving": first, "absorbed": second,
                    "score": 0.99, "matcher_version": "exact-v1"}),
            ] {
                if entity == "work" {
                    round_trip::<EntityDecision<WorkId>>(&decision);
                } else {
                    round_trip::<EntityDecision<PersonId>>(&decision);
                }
                let action = json!({"entity": entity, "decision": decision});
                round_trip::<DecisionAction>(&action);
                round_trip::<Decision>(&json!({
                    "id": "decision-1", "recorded_at": "2026-09-12T12:00:00Z",
                    "rationale": "Fixture evidence.", "action": action
                }));
            }
        }
    }

    #[test]
    fn should_reject_decision_when_entity_kinds_are_mixed() {
        assert!(serde_json::from_value::<DecisionAction>(json!({
            "entity": "work", "decision": {"action": "merge", "surviving": "W1", "absorbed": "P1"}
        })).is_err());
    }

    #[test]
    fn should_round_trip_acquisition_when_each_stage_has_provenance() {
        for source in [
            json!({"kind": "local", "paper_id": "0123456789abcdef"}),
            json!({"kind": "provider", "provider": "openalex", "record_id": "W123"}),
            json!({"kind": "open_access", "locator": "arxiv", "source_url": "https://arxiv.org/abs/2608.00001"}),
            json!({"kind": "agent", "provider": "claude", "model": "fixture", "generation_id": "run-1"}),
            json!({"kind": "user"}),
        ] {
            round_trip::<AcquisitionSource>(&source);
        }
        round_trip::<AcquisitionProvenance>(&provenance());
        for state in [
            json!({"state": "unresolved"}),
            json!({"state": "identifier_found", "identifiers": [{"scheme": "arxiv", "value": "2608.00001"}], "provenance": provenance()}),
            json!({"state": "link_found", "url": "https://arxiv.org/pdf/2608.00001", "provenance": provenance()}),
            json!({"state": "downloaded", "local_copy": local_copy(), "provenance": provenance()}),
            json!({"state": "mapped", "local_copy": local_copy(), "provenance": provenance()}),
            json!({"state": "unavailable", "reason": "No open-access copy.", "checked_at": "2026-09-12T12:00:00Z", "source": {"kind": "user"}}),
        ] {
            round_trip::<AcquisitionState>(&state);
        }
        assert!(
            serde_json::from_value::<AcquisitionState>(json!({"state": "downloaded"})).is_err()
        );
        assert!(serde_json::from_value::<AcquisitionState>(json!({"state": "invented"})).is_err());
    }

    #[test]
    fn should_resolve_alias_when_an_entity_is_merged_repeatedly() {
        let [first, second, third, unknown] =
            ["W1", "W2", "W3", "W4"].map(|id| id.parse::<WorkId>().unwrap());
        let mut aliases = AliasMap::default();
        aliases.merge(&first, &second);
        aliases.merge(&second, &third);
        assert_eq!(aliases.resolve(&first), &third);
        assert_eq!(aliases.resolve(&second), &third);
        assert_eq!(aliases.resolve(&third), &third);
        assert_eq!(aliases.resolve(&unknown), &unknown);
        let snapshot = serde_json::to_value(&aliases).unwrap();
        assert_eq!(snapshot, json!({"W1": "W2", "W2": "W3"}));
        round_trip::<AliasMap<WorkId>>(&snapshot);
        let restored: AliasMap<WorkId> = serde_json::from_value(snapshot).unwrap();
        assert_eq!(restored.resolve(&first), &third);
        aliases.merge(&third, &first);
        aliases.merge(&first, &first);
        assert_eq!(aliases, restored);

        let mut people = AliasMap::default();
        let original: PersonId = "P1".parse().unwrap();
        let survivor = "P2".parse().unwrap();
        people.merge(&original, &survivor);
        assert_eq!(people.resolve(&original), &survivor);
        round_trip::<AliasMap<PersonId>>(&json!({"P1": "P2"}));
    }

    #[test]
    fn should_reject_aliases_when_persisted_links_contain_cycles() {
        for aliases in [
            json!({"W1": "W1"}),
            json!({"W1": "W2", "W2": "W1"}),
            json!({"W0": "W1", "W1": "W2", "W2": "W3", "W3": "W1"}),
            json!({"W1": "P1"}),
        ] {
            assert!(serde_json::from_value::<AliasMap<WorkId>>(aliases).is_err());
        }
    }

    #[test]
    fn should_serialize_aliases_and_identifier_sets_stably_when_insert_order_differs() {
        let first: AliasMap<WorkId> =
            serde_json::from_value(json!({"W2": "W3", "W1": "W2"})).unwrap();
        let second: AliasMap<WorkId> =
            serde_json::from_value(json!({"W1": "W2", "W2": "W3"})).unwrap();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        let doi = Identifier::parse("doi:10.1234/example").unwrap();
        let arxiv = Identifier::parse("arxiv:2608.00001").unwrap();
        let first = BTreeSet::from([doi.clone(), arxiv.clone()]);
        let second = BTreeSet::from([arxiv, doi]);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
    }
}
