//! Provider expansion: make the outgoing fanout of a Work known from its
//! `OpenAlex` record, for Works the reader does not hold locally.
//!
//! The local flow (`literature`) covers held papers; this covers every other
//! Work that has an identifier the provider understands.

use lysilogy_core::{
    DetectedIdentifier, ExpansionUnavailableReason, IdentifierScheme, IdentifierStatus,
    SourceRecordId, WorkId, WorkIdentifier,
};
use lysilogy_db::citation::CitationRepository;
use lysilogy_db::claim::ClaimRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;

use crate::Result;
use crate::ingest::{hydrate_stub_works, ingest_openalex_references};
use crate::literature::{is_soft, provider_key};
use crate::openalex::{JsonTransport, OpenAlexClient, fetch_work_cached_record};

/// The repositories and provider client expansion works through.
pub struct ExpandContext<'a, T: JsonTransport> {
    pub works: &'a WorkRepository,
    pub citations: &'a CitationRepository,
    pub claims: &'a ClaimRepository,
    pub records: &'a SourceRecordRepository,
    pub client: &'a OpenAlexClient<T>,
}

/// What one provider expansion did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderExpansion {
    Expanded {
        provider_key: String,
        source_record: SourceRecordId,
        /// Citation edges asserted from the record.
        references: usize,
        /// Works created by this run's reference ingest.
        new_works: usize,
        /// Stub Works given a title by this run.
        hydrated: usize,
        warnings: Vec<String>,
    },
    Unavailable {
        reason: ExpansionUnavailableReason,
        provider_key: Option<String>,
        warnings: Vec<String>,
    },
}

/// The provider lookup key for a Work: its `OpenAlex` ID, else `doi:<doi>`, else
/// the arXiv `DataCite` DOI. Only active identifiers count.
#[must_use]
pub fn provider_lookup_key(identifiers: &[WorkIdentifier]) -> Option<String> {
    [
        IdentifierScheme::Openalex,
        IdentifierScheme::Doi,
        IdentifierScheme::Arxiv,
    ]
    .into_iter()
    .find_map(|scheme| {
        let identifier = identifiers.iter().find(|identifier| {
            identifier.status == IdentifierStatus::Active && identifier.scheme == scheme
        })?;
        Some(provider_key(&DetectedIdentifier {
            scheme,
            value: identifier.value_normalized.clone(),
        }))
    })
}

/// Fetch the Work's provider record and assert its references.
///
/// A Work with no usable identifier, or one the provider does not know, is
/// reported `Unavailable` without error. A record whose `OpenAlex` ID already
/// belongs to a different Work is also `Unavailable`, and nothing is ingested:
/// merging is resolution's job. Any other provider failure is an error.
pub async fn expand_from_provider<T: JsonTransport>(
    context: &ExpandContext<'_, T>,
    work: WorkId,
) -> Result<ProviderExpansion> {
    let identifiers = context.works.active_identifiers(work).await?;
    let Some(key) = provider_lookup_key(&identifiers) else {
        return Ok(ProviderExpansion::Unavailable {
            reason: ExpansionUnavailableReason::NoProviderIdentifier,
            provider_key: None,
            warnings: Vec::new(),
        });
    };
    let (record, provider) =
        match fetch_work_cached_record(context.records, context.client, &key).await {
            Ok(found) => found,
            Err(error) if is_soft(&error) => {
                return Ok(ProviderExpansion::Unavailable {
                    reason: ExpansionUnavailableReason::ProviderNotFound,
                    provider_key: Some(key),
                    warnings: Vec::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
    let owner = context
        .works
        .find_by_identifier(IdentifierScheme::Openalex, &provider.openalex_id)
        .await?;
    if let Some(owner) = owner.filter(|owner| owner.id != work) {
        return Ok(ProviderExpansion::Unavailable {
            reason: ExpansionUnavailableReason::IdentifierConflict,
            provider_key: Some(key),
            warnings: vec![format!("openalex_id_on_other_work:{}", owner.id)],
        });
    }
    let report =
        ingest_openalex_references(context.works, context.citations, work, &provider, record)
            .await?;
    let outgoing = context.citations.outgoing(work).await?;
    let hydrated = hydrate_stub_works(
        context.works,
        context.claims,
        context.records,
        context.client,
        &outgoing,
    )
    .await?;
    Ok(ProviderExpansion::Expanded {
        provider_key: key,
        source_record: record,
        references: report.edges,
        new_works: report.new_works,
        hydrated,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use lysilogy_core::{IdentifierId, Visibility, WorkKind};
    use serde_json::{Value, json};

    use super::*;
    use crate::openalex::{ProviderError, ProviderFuture};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Alphanumeric only, so it survives URL percent-encoding unchanged.
    fn marker() -> String {
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        format!("x{}x{sequence}x{nanos}", std::process::id())
    }

    type Responder = dyn Fn(&str) -> std::result::Result<Value, ProviderError> + Send + Sync;

    #[derive(Clone)]
    struct ScriptedTransport {
        responder: Arc<Responder>,
        urls: Arc<Mutex<Vec<String>>>,
    }

    impl ScriptedTransport {
        fn new(
            responder: impl Fn(&str) -> std::result::Result<Value, ProviderError>
            + Send
            + Sync
            + 'static,
        ) -> Self {
            Self {
                responder: Arc::new(responder),
                urls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn urls(&self) -> Vec<String> {
            self.urls.lock().expect("lock").clone()
        }

        fn record_requests(&self) -> usize {
            self.urls()
                .iter()
                .filter(|url| url.contains("/works/"))
                .count()
        }
    }

    impl JsonTransport for ScriptedTransport {
        fn get_json<'a>(
            &'a self,
            url: &'a str,
        ) -> ProviderFuture<'a, std::result::Result<Value, ProviderError>> {
            Box::pin(async move {
                self.urls.lock().expect("lock").push(url.to_owned());
                (self.responder)(url)
            })
        }
    }

    /// Answers a summary batch with one titled result per requested ID.
    fn batch_reply(url: &str) -> Value {
        let ids = url
            .split("filter=openalex:")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap_or_default();
        let results: Vec<Value> = ids
            .split('|')
            .map(|id| {
                json!({"id": format!("https://openalex.org/{id}"),
                       "display_name": format!("Title of {id}"), "publication_year": 2010})
            })
            .collect();
        json!({ "results": results })
    }

    fn record_json(openalex_id: &str, references: &[String]) -> Value {
        let urls: Vec<String> = references
            .iter()
            .map(|id| format!("https://openalex.org/{id}"))
            .collect();
        json!({
            "id": format!("https://openalex.org/{openalex_id}"),
            "display_name": "Expanded Paper",
            "publication_year": 2015,
            "referenced_works": urls,
        })
    }

    struct Repos {
        works: WorkRepository,
        citations: CitationRepository,
        claims: ClaimRepository,
        records: SourceRecordRepository,
    }

    impl Repos {
        async fn connect() -> Option<Self> {
            let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
                .ok()
                .filter(|url| !url.is_empty())?;
            let pool = lysilogy_db::connect(&url).await.expect("connect");
            lysilogy_db::migrate(&pool).await.expect("migrate");
            Some(Self {
                works: WorkRepository::new(pool.clone()),
                citations: CitationRepository::new(pool.clone()),
                claims: ClaimRepository::new(pool.clone()),
                records: SourceRecordRepository::new(pool),
            })
        }

        async fn expand(
            &self,
            transport: &ScriptedTransport,
            work: WorkId,
        ) -> Result<ProviderExpansion> {
            let client = OpenAlexClient::new(transport.clone());
            let context = ExpandContext {
                works: &self.works,
                citations: &self.citations,
                claims: &self.claims,
                records: &self.records,
                client: &client,
            };
            expand_from_provider(&context, work).await
        }

        async fn stub(&self, scheme: IdentifierScheme, value: &str) -> WorkId {
            self.works
                .resolve_or_create(scheme, value, WorkKind::Article, None)
                .await
                .expect("stub")
                .0
                .id
        }
    }

    fn identifier(
        scheme: IdentifierScheme,
        value: &str,
        status: IdentifierStatus,
    ) -> WorkIdentifier {
        WorkIdentifier {
            id: IdentifierId::new(),
            work_id: WorkId::new(),
            scheme,
            value_normalized: value.to_owned(),
            source_record_id: None,
            status,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn should_prefer_openalex_then_doi_then_arxiv_when_choosing_lookup_key() {
        let active = IdentifierStatus::Active;
        let openalex = identifier(IdentifierScheme::Openalex, "W1", active);
        let doi = identifier(IdentifierScheme::Doi, "10.1/abc", active);
        let arxiv = identifier(IdentifierScheme::Arxiv, "1706.03762", active);
        let all = [arxiv.clone(), doi.clone(), openalex];
        assert_eq!(provider_lookup_key(&all).as_deref(), Some("W1"));
        assert_eq!(
            provider_lookup_key(&[arxiv.clone(), doi]).as_deref(),
            Some("doi:10.1/abc")
        );
        assert_eq!(
            provider_lookup_key(&[arxiv]).as_deref(),
            Some("doi:10.48550/arxiv.1706.03762")
        );
        assert_eq!(provider_lookup_key(&[]), None);

        let inactive = [
            identifier(
                IdentifierScheme::Openalex,
                "W1",
                IdentifierStatus::Retracted,
            ),
            identifier(
                IdentifierScheme::Doi,
                "10.1/abc",
                IdentifierStatus::Conflicting,
            ),
            identifier(IdentifierScheme::Pmid, "123", active),
            identifier(IdentifierScheme::Url, "https://x.test", active),
        ];
        assert_eq!(
            provider_lookup_key(&inactive),
            None,
            "inactive and unsupported identifiers are ignored"
        );
        let mixed = [
            inactive[0].clone(),
            identifier(IdentifierScheme::Doi, "10.2/z", active),
        ];
        assert_eq!(provider_lookup_key(&mixed).as_deref(), Some("doi:10.2/z"));
    }

    #[tokio::test]
    async fn should_ingest_references_and_title_stubs_when_expanding_stub() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let own = format!("W1{marker}");
        let cited = [
            format!("W2{marker}"),
            format!("W3{marker}"),
            format!("W4{marker}"),
        ];
        let work = repos.stub(IdentifierScheme::Openalex, &own).await;
        let record = record_json(&own, &cited);
        let transport = ScriptedTransport::new(move |url| {
            if url.contains("filter=openalex:") {
                return Ok(batch_reply(url));
            }
            Ok(record.clone())
        });

        let outcome = repos.expand(&transport, work).await.expect("expand");
        let ProviderExpansion::Expanded {
            provider_key,
            references,
            new_works,
            hydrated,
            warnings,
            ..
        } = outcome
        else {
            panic!("expected an expansion, got {outcome:?}");
        };
        assert_eq!(provider_key, own);
        assert_eq!((references, new_works, hydrated), (3, 3, 3));
        assert!(warnings.is_empty());
        let outgoing = repos.citations.outgoing(work).await.expect("outgoing");
        assert_eq!(outgoing.len(), 3);
        for stub in &outgoing {
            assert!(
                stub.display_title
                    .as_deref()
                    .is_some_and(|title| title.starts_with("Title of ")),
                "{stub:?}"
            );
        }
    }

    #[tokio::test]
    async fn should_report_no_provider_identifier_when_work_has_no_identifiers() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let work = repos
            .works
            .create(WorkKind::Article, Visibility::Private)
            .await
            .expect("work")
            .id;
        let transport = ScriptedTransport::new(|_| Ok(json!({})));
        let outcome = repos.expand(&transport, work).await.expect("expand");
        assert_eq!(
            outcome,
            ProviderExpansion::Unavailable {
                reason: ExpansionUnavailableReason::NoProviderIdentifier,
                provider_key: None,
                warnings: Vec::new(),
            }
        );
        assert!(transport.urls().is_empty(), "no request without a key");
    }

    #[tokio::test]
    async fn should_report_provider_not_found_when_provider_returns_404() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let own = format!("W1{}", marker());
        let work = repos.stub(IdentifierScheme::Openalex, &own).await;
        let transport = ScriptedTransport::new(|_| Err(ProviderError::Status(404)));
        let outcome = repos.expand(&transport, work).await.expect("expand");
        assert_eq!(
            outcome,
            ProviderExpansion::Unavailable {
                reason: ExpansionUnavailableReason::ProviderNotFound,
                provider_key: Some(own),
                warnings: Vec::new(),
            }
        );

        let malformed = ScriptedTransport::new(|_| Ok(json!({"unexpected": true})));
        let other = format!("W1{}", marker());
        let other_work = repos.stub(IdentifierScheme::Openalex, &other).await;
        let outcome = repos.expand(&malformed, other_work).await.expect("expand");
        assert!(matches!(
            outcome,
            ProviderExpansion::Unavailable {
                reason: ExpansionUnavailableReason::ProviderNotFound,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn should_report_identifier_conflict_when_openalex_id_belongs_to_other_work() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let doi = format!("10.4444/{marker}");
        let other_openalex = format!("W5{marker}");
        let work = repos.stub(IdentifierScheme::Doi, &doi).await;
        let other = repos
            .stub(IdentifierScheme::Openalex, &other_openalex)
            .await;
        let record = record_json(&other_openalex, &[format!("W6{marker}")]);
        let transport = ScriptedTransport::new(move |_| Ok(record.clone()));

        let outcome = repos.expand(&transport, work).await.expect("expand");
        assert_eq!(
            outcome,
            ProviderExpansion::Unavailable {
                reason: ExpansionUnavailableReason::IdentifierConflict,
                provider_key: Some(format!("doi:{doi}")),
                warnings: vec![format!("openalex_id_on_other_work:{other}")],
            }
        );
        assert!(
            repos
                .citations
                .outgoing(work)
                .await
                .expect("outgoing")
                .is_empty(),
            "nothing is ingested on a conflict"
        );
        assert!(
            repos
                .citations
                .outgoing(other)
                .await
                .expect("outgoing")
                .is_empty(),
            "the other Work is left alone"
        );
    }

    #[tokio::test]
    async fn should_expand_with_zero_references_when_record_cites_nothing() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let own = format!("W1{}", marker());
        let work = repos.stub(IdentifierScheme::Openalex, &own).await;
        let record = record_json(&own, &[]);
        let transport = ScriptedTransport::new(move |_| Ok(record.clone()));
        let outcome = repos.expand(&transport, work).await.expect("expand");
        assert!(
            matches!(
                outcome,
                ProviderExpansion::Expanded {
                    references: 0,
                    new_works: 0,
                    hydrated: 0,
                    ..
                }
            ),
            "{outcome:?}"
        );
        assert_eq!(transport.urls().len(), 1, "no summary batch for no stubs");
    }

    #[tokio::test]
    async fn should_make_no_record_request_when_record_cached() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let marker = marker();
        let own = format!("W1{marker}");
        let work = repos.stub(IdentifierScheme::Openalex, &own).await;
        let record = record_json(&own, &[format!("W2{marker}"), format!("W3{marker}")]);
        let transport = ScriptedTransport::new(move |url| {
            if url.contains("filter=openalex:") {
                return Ok(batch_reply(url));
            }
            Ok(record.clone())
        });
        repos.expand(&transport, work).await.expect("first");
        assert_eq!(transport.record_requests(), 1);
        let requests = transport.urls().len();

        let second = repos.expand(&transport, work).await.expect("second");
        assert!(
            matches!(
                second,
                ProviderExpansion::Expanded {
                    references: 2,
                    new_works: 0,
                    hydrated: 0,
                    ..
                }
            ),
            "{second:?}"
        );
        assert_eq!(transport.record_requests(), 1, "the record came from cache");
        assert_eq!(
            transport.urls().len(),
            requests,
            "titled stubs need no batch"
        );
    }

    #[tokio::test]
    async fn should_return_error_when_transport_fails_hard() {
        let Some(repos) = Repos::connect().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let own = format!("W1{}", marker());
        let work = repos.stub(IdentifierScheme::Openalex, &own).await;
        for failure in [
            || ProviderError::Status(503),
            || ProviderError::Request("connection reset".to_owned()),
        ] {
            let transport = ScriptedTransport::new(move |_| Err(failure()));
            let outcome = repos.expand(&transport, work).await;
            assert!(
                matches!(outcome, Err(crate::IngestError::Provider(_))),
                "{outcome:?}"
            );
        }
    }
}
