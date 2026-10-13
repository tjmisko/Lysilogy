//! Persistence for local evidence: bibliography entries parsed from a citing
//! document, their resolutions to Works, and in-text citation mentions.

use lysilogy_core::{
    AssertionId, CitationStyle, DocumentVersionId, EntryId, MentionId, ParseStateError,
    ResolutionMethod, ResolutionStatus, SourceRecordId, WorkId,
};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::Result;

/// One bibliography entry to store for a document version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEntry {
    pub entry_key: String,
    pub ordinal: i32,
    pub raw_text: String,
    pub printed_key: Option<String>,
    /// Serialized `BibliographicFields`.
    pub fields: Value,
    pub page: i32,
    pub text_start: i32,
    pub text_end: i32,
    /// `[{start, end}]` spans of the entry's member text runs.
    pub member_spans: Value,
    /// `{x_min, y_min, x_max, y_max}` on the page, when known.
    pub region: Option<Value>,
}

/// A stored bibliography entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEntry {
    pub id: EntryId,
    pub entry_key: String,
    pub ordinal: Option<i32>,
    pub raw_text: String,
    pub printed_key: Option<String>,
    pub fields: Value,
    pub page: Option<i32>,
    pub text_start: Option<i32>,
    pub text_end: Option<i32>,
    pub member_spans: Option<Value>,
    pub region: Option<Value>,
}

/// One in-text citation occurrence to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMention {
    pub assertion: AssertionId,
    pub version: DocumentVersionId,
    pub entry: EntryId,
    pub style: CitationStyle,
    pub marker_text: String,
    pub page: i32,
    pub marker_start: i32,
    pub marker_end: i32,
    pub passage_start: i32,
    pub passage_end: i32,
    /// Whitespace-collapsed passage text.
    pub passage: String,
    pub rects: Option<Value>,
}

/// A stored in-text citation occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMention {
    pub id: MentionId,
    pub assertion: AssertionId,
    pub entry: Option<EntryId>,
    pub style: Option<CitationStyle>,
    pub marker_text: Option<String>,
    pub page: Option<i32>,
    pub marker_start: Option<i32>,
    pub marker_end: Option<i32>,
    pub passage_start: Option<i32>,
    pub passage_end: Option<i32>,
    pub passage: String,
    pub rects: Option<Value>,
}

const ENTRY_COLUMNS: &str = "id, entry_key, ordinal, raw_text, printed_key, fields, page, \
     text_start, text_end, member_spans, region";

const MENTION_COLUMNS: &str = "m.id, m.assertion_id, m.entry_id, m.style, m.marker_text, m.page, \
     m.marker_start, m.marker_end, m.passage_start, m.passage_end, m.quote_or_anchor, m.rects";

/// Reads and writes bibliography entries, their resolutions, and mentions.
#[derive(Clone)]
pub struct EvidenceRepository {
    pool: PgPool,
}

impl EvidenceRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Store entries for a document version, skipping any whose key already
    /// exists, and return the IDs in ordinal order.
    pub async fn insert_entries(
        &self,
        version: DocumentVersionId,
        record: SourceRecordId,
        entries: &[NewEntry],
    ) -> Result<Vec<EntryId>> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let mut transaction = self.pool.begin().await?;
        for entry in entries {
            sqlx::query(
                "insert into source.bibliography_entry \
                     (citing_version_id, entry_key, raw_text, page_or_locator, source_record_id, \
                      ordinal, printed_key, fields, page, text_start, text_end, member_spans, region) \
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
                 on conflict (citing_version_id, entry_key) do nothing",
            )
            .bind(version.as_uuid())
            .bind(&entry.entry_key)
            .bind(&entry.raw_text)
            .bind(format!("p.{}", entry.page))
            .bind(record.as_uuid())
            .bind(entry.ordinal)
            .bind(&entry.printed_key)
            .bind(&entry.fields)
            .bind(entry.page)
            .bind(entry.text_start)
            .bind(entry.text_end)
            .bind(&entry.member_spans)
            .bind(&entry.region)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;

        let keys: Vec<&str> = entries
            .iter()
            .map(|entry| entry.entry_key.as_str())
            .collect();
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "select id from source.bibliography_entry \
             where citing_version_id = $1 and entry_key = any($2) \
             order by ordinal, entry_key",
        )
        .bind(version.as_uuid())
        .bind(&keys)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(EntryId::from_uuid).collect())
    }

    /// One stored entry by ID.
    pub async fn entry(&self, entry: EntryId) -> Result<Option<StoredEntry>> {
        let row = sqlx::query(&format!(
            "select {ENTRY_COLUMNS} from source.bibliography_entry where id = $1"
        ))
        .bind(entry.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(entry_from_row).transpose()
    }

    /// Entries extracted by one source record, in ordinal order.
    pub async fn entries_for_record(&self, record: SourceRecordId) -> Result<Vec<StoredEntry>> {
        let rows = sqlx::query(&format!(
            "select {ENTRY_COLUMNS} from source.bibliography_entry \
             where source_record_id = $1 order by ordinal, entry_key"
        ))
        .bind(record.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(entry_from_row).collect()
    }

    /// The Work an entry is currently accepted as, with how it was resolved.
    pub async fn accepted_resolution(
        &self,
        entry: EntryId,
    ) -> Result<Option<(WorkId, ResolutionMethod)>> {
        let row = sqlx::query(
            "select candidate_work_id, method from graph.entry_resolution \
             where entry_id = $1 and status = 'accepted'",
        )
        .bind(entry.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let method: String = row.try_get("method")?;
        Ok(Some((
            WorkId::from_uuid(row.try_get("candidate_work_id")?),
            parse_state(&method)?,
        )))
    }

    /// Record a resolution candidate for an entry. Re-recording the same
    /// `(entry, work)` pair updates its status, method, and confidence.
    pub async fn record_resolution(
        &self,
        entry: EntryId,
        work: WorkId,
        status: ResolutionStatus,
        method: ResolutionMethod,
        confidence: Option<f64>,
    ) -> Result<()> {
        sqlx::query(
            "insert into graph.entry_resolution \
                 (entry_id, candidate_work_id, status, method, confidence) \
             values ($1, $2, $3, $4, $5) \
             on conflict (entry_id, candidate_work_id) do update \
                 set status = excluded.status, method = excluded.method, \
                     confidence = excluded.confidence, updated_at = now()",
        )
        .bind(entry.as_uuid())
        .bind(work.as_uuid())
        .bind(status.as_str())
        .bind(method.as_str())
        .bind(confidence)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Withdraw the accepted resolution of an entry, if any.
    pub async fn withdraw_accepted(&self, entry: EntryId) -> Result<()> {
        sqlx::query(
            "update graph.entry_resolution set status = 'withdrawn', updated_at = now() \
             where entry_id = $1 and status = 'accepted'",
        )
        .bind(entry.as_uuid())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store one mention. Returns `None` when the same occurrence is already stored.
    pub async fn insert_mention(&self, mention: &NewMention) -> Result<Option<MentionId>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "insert into graph.citation_mention \
                 (assertion_id, document_version_id, entry_id, style, marker_text, page, \
                  marker_start, marker_end, passage_start, passage_end, rects, \
                  page_or_locator, quote_or_anchor, text_hash) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                     encode(sha256(convert_to($13, 'UTF8')), 'hex')) \
             on conflict do nothing \
             returning id",
        )
        .bind(mention.assertion.as_uuid())
        .bind(mention.version.as_uuid())
        .bind(mention.entry.as_uuid())
        .bind(mention.style.as_str())
        .bind(&mention.marker_text)
        .bind(mention.page)
        .bind(mention.marker_start)
        .bind(mention.marker_end)
        .bind(mention.passage_start)
        .bind(mention.passage_end)
        .bind(&mention.rects)
        .bind(format!("p.{}", mention.page))
        .bind(&mention.passage)
        .fetch_optional(&self.pool)
        .await?;
        Ok(id.map(MentionId::from_uuid))
    }

    /// Mentions of `cited` by `citing` inside one document version, in text order.
    pub async fn mentions_for_edge(
        &self,
        citing: WorkId,
        cited: WorkId,
        version: DocumentVersionId,
    ) -> Result<Vec<StoredMention>> {
        let rows = sqlx::query(&format!(
            "select {MENTION_COLUMNS} from graph.citation_mention m \
             join graph.citation_assertion a on a.id = m.assertion_id \
             where a.citing_work_id = $1 and a.cited_work_id = $2 \
               and a.status in ('asserted', 'accepted') \
               and m.document_version_id = $3 \
             order by m.passage_start, m.marker_start, m.id"
        ))
        .bind(citing.as_uuid())
        .bind(cited.as_uuid())
        .bind(version.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(mention_from_row).collect()
    }

    /// Edges that `citing` asserts from the local bibliography of `version`,
    /// each with the mentions stored for it, in text order.
    pub async fn local_edges(
        &self,
        citing: WorkId,
        version: DocumentVersionId,
    ) -> Result<Vec<(WorkId, AssertionId, Vec<StoredMention>)>> {
        let assertions = sqlx::query(
            "select a.id, a.cited_work_id from graph.citation_assertion a \
             join source.bibliography_entry e on e.id = a.entry_id \
             where a.citing_work_id = $1 and e.citing_version_id = $2 \
               and a.status in ('asserted', 'accepted') \
             order by e.ordinal, a.id",
        )
        .bind(citing.as_uuid())
        .bind(version.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        let mut edges = Vec::with_capacity(assertions.len());
        for row in &assertions {
            let assertion = AssertionId::from_uuid(row.try_get("id")?);
            let cited = WorkId::from_uuid(row.try_get("cited_work_id")?);
            let mention_rows = sqlx::query(&format!(
                "select {MENTION_COLUMNS} from graph.citation_mention m \
                 where m.assertion_id = $1 and m.document_version_id = $2 \
                 order by m.passage_start, m.marker_start, m.id"
            ))
            .bind(assertion.as_uuid())
            .bind(version.as_uuid())
            .fetch_all(&self.pool)
            .await?;
            let mentions = mention_rows
                .iter()
                .map(mention_from_row)
                .collect::<Result<Vec<_>>>()?;
            edges.push((cited, assertion, mentions));
        }
        Ok(edges)
    }
}

fn parse_state<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr<Err = ParseStateError>,
{
    Ok(value.parse()?)
}

fn entry_from_row(row: &PgRow) -> Result<StoredEntry> {
    Ok(StoredEntry {
        id: EntryId::from_uuid(row.try_get("id")?),
        entry_key: row.try_get("entry_key")?,
        ordinal: row.try_get("ordinal")?,
        raw_text: row.try_get("raw_text")?,
        printed_key: row.try_get("printed_key")?,
        fields: row.try_get("fields")?,
        page: row.try_get("page")?,
        text_start: row.try_get("text_start")?,
        text_end: row.try_get("text_end")?,
        member_spans: row.try_get("member_spans")?,
        region: row.try_get("region")?,
    })
}

fn mention_from_row(row: &PgRow) -> Result<StoredMention> {
    Ok(StoredMention {
        id: MentionId::from_uuid(row.try_get("id")?),
        assertion: AssertionId::from_uuid(row.try_get("assertion_id")?),
        entry: row
            .try_get::<Option<Uuid>, _>("entry_id")?
            .map(EntryId::from_uuid),
        style: row
            .try_get::<Option<String>, _>("style")?
            .map(|style| parse_state(&style))
            .transpose()?,
        marker_text: row.try_get("marker_text")?,
        page: row.try_get("page")?,
        marker_start: row.try_get("marker_start")?,
        marker_end: row.try_get("marker_end")?,
        passage_start: row.try_get("passage_start")?,
        passage_end: row.try_get("passage_end")?,
        passage: row
            .try_get::<Option<String>, _>("quote_or_anchor")?
            .unwrap_or_default(),
        rects: row.try_get("rects")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citation::CitationRepository;
    use crate::test_support::{self, new_entries, new_record, new_version, new_work, sample_entry};

    fn sample_mention(
        assertion: AssertionId,
        version: DocumentVersionId,
        entry: EntryId,
        marker_start: i32,
    ) -> NewMention {
        NewMention {
            assertion,
            version,
            entry,
            style: CitationStyle::Numeric,
            marker_text: "[1]".into(),
            page: 2,
            marker_start,
            marker_end: marker_start + 3,
            passage_start: marker_start - 20,
            passage_end: marker_start + 40,
            passage: "We build on prior work [1] for this.".into(),
            rects: Some(serde_json::json!([{"x": 0.1}])),
        }
    }

    #[tokio::test]
    async fn should_not_duplicate_entries_when_inserted_twice() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = EvidenceRepository::new(pool.clone());
        let version = new_version(&pool).await;
        let record = new_record(&pool).await;
        let entries: Vec<NewEntry> = (0..3)
            .map(|ordinal| sample_entry("bib-v1", ordinal))
            .collect();

        let first = repo
            .insert_entries(version, record, &entries)
            .await
            .expect("first");
        let second = repo
            .insert_entries(version, record, &entries)
            .await
            .expect("second");
        assert_eq!(first.len(), 3);
        assert_eq!(first, second, "second insert returns the same IDs in order");

        let stored = repo.entries_for_record(record).await.expect("stored");
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[1].entry_key, "bib-v1:2");
        assert_eq!(stored[1].ordinal, Some(1));
        assert_eq!(stored[1].printed_key.as_deref(), Some("2"));
        assert_eq!(stored[1].fields["title"], "A title");
        assert_eq!(stored[1].page, Some(3));
        assert!(stored[1].region.is_some());
    }

    #[tokio::test]
    async fn should_return_empty_when_no_entries_given() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = EvidenceRepository::new(pool.clone());
        let version = new_version(&pool).await;
        let record = new_record(&pool).await;
        let ids = repo
            .insert_entries(version, record, &[])
            .await
            .expect("insert");
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn should_keep_one_accepted_resolution_when_entry_resolved_twice() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = EvidenceRepository::new(pool.clone());
        let version = new_version(&pool).await;
        let entry = new_entries(&pool, version, 1).await[0];
        let first = new_work(&pool).await;
        let second = new_work(&pool).await;

        assert_eq!(repo.accepted_resolution(entry).await.expect("none"), None);
        repo.record_resolution(
            entry,
            first,
            ResolutionStatus::Accepted,
            ResolutionMethod::LocalStub,
            None,
        )
        .await
        .expect("first");
        repo.record_resolution(
            entry,
            first,
            ResolutionStatus::Accepted,
            ResolutionMethod::Identifier,
            Some(1.0),
        )
        .await
        .expect("same work again upserts");
        assert_eq!(
            repo.accepted_resolution(entry).await.expect("accepted"),
            Some((first, ResolutionMethod::Identifier))
        );

        repo.withdraw_accepted(entry).await.expect("withdraw");
        assert_eq!(
            repo.accepted_resolution(entry).await.expect("withdrawn"),
            None
        );
        repo.record_resolution(
            entry,
            second,
            ResolutionStatus::Accepted,
            ResolutionMethod::TitleMatch,
            Some(0.9),
        )
        .await
        .expect("second");
        assert_eq!(
            repo.accepted_resolution(entry).await.expect("accepted"),
            Some((second, ResolutionMethod::TitleMatch))
        );
        let accepted: i64 = sqlx::query_scalar(
            "select count(*) from graph.entry_resolution where entry_id = $1 and status = 'accepted'",
        )
        .bind(entry.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(accepted, 1);
    }

    #[tokio::test]
    async fn should_store_mention_once_when_same_occurrence_inserted_twice() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = EvidenceRepository::new(pool.clone());
        let citations = CitationRepository::new(pool.clone());
        let version = new_version(&pool).await;
        let entry = new_entries(&pool, version, 1).await[0];
        let (citing, cited) = (new_work(&pool).await, new_work(&pool).await);
        let assertion = citations
            .add_entry_assertion(citing, cited, entry)
            .await
            .expect("assertion");

        let mention = sample_mention(assertion, version, entry, 500);
        let first = repo.insert_mention(&mention).await.expect("first");
        let second = repo.insert_mention(&mention).await.expect("second");
        assert!(first.is_some());
        assert!(second.is_none(), "duplicate occurrence is skipped");

        let other = repo
            .insert_mention(&sample_mention(assertion, version, entry, 900))
            .await
            .expect("other occurrence");
        assert!(other.is_some());

        let stored = repo
            .mentions_for_edge(citing, cited, version)
            .await
            .expect("mentions");
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].marker_start, Some(500));
        assert_eq!(stored[0].style, Some(CitationStyle::Numeric));
        assert_eq!(stored[0].passage, "We build on prior work [1] for this.");
        let row = sqlx::query(
            "select text_hash, page_or_locator from graph.citation_mention where id = $1",
        )
        .bind(first.expect("id").as_uuid())
        .fetch_one(&pool)
        .await
        .expect("row");
        let hash: String = row.try_get("text_hash").expect("hash");
        let locator: String = row.try_get("page_or_locator").expect("locator");
        assert_eq!(hash.len(), 64);
        assert_eq!(locator, "p.2");
    }

    #[tokio::test]
    async fn should_order_mentions_by_passage_start_when_listing_edge() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repo = EvidenceRepository::new(pool.clone());
        let citations = CitationRepository::new(pool.clone());
        let version = new_version(&pool).await;
        let entry = new_entries(&pool, version, 1).await[0];
        let (citing, cited) = (new_work(&pool).await, new_work(&pool).await);
        let assertion = citations
            .add_entry_assertion(citing, cited, entry)
            .await
            .expect("assertion");
        for start in [900, 100, 500] {
            repo.insert_mention(&sample_mention(assertion, version, entry, start))
                .await
                .expect("insert");
        }
        let edges = repo.local_edges(citing, version).await.expect("edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, cited);
        assert_eq!(edges[0].1, assertion);
        let starts: Vec<_> = edges[0].2.iter().map(|m| m.marker_start).collect();
        assert_eq!(starts, vec![Some(100), Some(500), Some(900)]);

        let other_version = new_version(&pool).await;
        assert!(
            repo.local_edges(citing, other_version)
                .await
                .expect("none")
                .is_empty()
        );
    }
}
