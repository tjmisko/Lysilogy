//! Persistence for open-access acquisition state (`ops.acquisition`).
//!
//! One row per Work records how far acquisition has progressed, so a rerun can
//! resume at the stage it stopped. The row is operational state, not evidence.

use chrono::{DateTime, Utc};
use lysilogy_core::{
    AcquisitionState, BlobId, CandidateOrigin, DocumentId, DocumentVersionId, SourceRecordId,
    UnavailableReason, WorkId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::Result;

/// How many trials a row keeps, newest last.
pub const MAX_TRIALS: i64 = 20;

/// One candidate link for a Work's open-access copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateRecord {
    pub url: String,
    pub origin: CandidateOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_record_id: Option<SourceRecordId>,
}

/// The logged result of trying one candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialRecord {
    pub url: String,
    pub origin: CandidateOrigin,
    pub outcome: String,
    pub at: DateTime<Utc>,
}

/// A downloaded PDF held in the blob store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRecord {
    pub source: CandidateOrigin,
    pub pdf_url: String,
    pub final_url: String,
    pub sha256: String,
    pub bytes: i64,
    pub blob: BlobId,
}

/// A stored acquisition row with typed fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AcquisitionRow {
    pub work_id: WorkId,
    pub state: AcquisitionState,
    pub unavailable_reason: Option<UnavailableReason>,
    pub identifiers: Option<Value>,
    pub candidates: Vec<CandidateRecord>,
    pub trials: Vec<TrialRecord>,
    pub source: Option<CandidateOrigin>,
    pub pdf_url: Option<String>,
    pub final_url: Option<String>,
    pub sha256: Option<String>,
    pub bytes: Option<i64>,
    pub blob_id: Option<BlobId>,
    pub document_id: Option<DocumentId>,
    pub document_version_id: Option<DocumentVersionId>,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
    pub lease_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A Work cited by a paper, with whether acquisition would skip it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CitedTarget {
    pub work_id: WorkId,
    /// Already mapped, or marked unavailable inside the recheck window.
    pub skipped: bool,
}

/// How long an `unavailable` verdict is trusted, in days (mirrors the runner).
const RECHECK_DAYS: i32 = 30;

/// Reads and writes acquisition rows.
#[derive(Clone)]
pub struct AcquisitionRepository {
    pool: PgPool,
}

impl AcquisitionRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ensure a row exists, then take its 10-minute lease if it is free or
    /// expired. Returns `false` when another runner holds a live lease.
    pub async fn claim(&self, work: WorkId) -> Result<bool> {
        sqlx::query("insert into ops.acquisition (work_id) values ($1) on conflict do nothing")
            .bind(work.as_uuid())
            .execute(&self.pool)
            .await?;
        let claimed: Option<Uuid> = sqlx::query_scalar(
            "update ops.acquisition set lease_until = now() + interval '10 minutes', \
                 updated_at = now() \
             where work_id = $1 and (lease_until is null or lease_until < now()) \
             returning work_id",
        )
        .bind(work.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        Ok(claimed.is_some())
    }

    /// Clear the lease.
    pub async fn release(&self, work: WorkId) -> Result<()> {
        self.execute_for(
            work,
            "update ops.acquisition set lease_until = null, updated_at = now() where work_id = $1",
        )
        .await
    }

    /// The row for a Work, if one exists.
    pub async fn get(&self, work: WorkId) -> Result<Option<AcquisitionRow>> {
        let row = sqlx::query("select * from ops.acquisition where work_id = $1")
            .bind(work.as_uuid())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_from_pg).transpose()
    }

    /// State and reason for each of the given Works that has a row.
    pub async fn states_for(
        &self,
        works: &[WorkId],
    ) -> Result<Vec<(WorkId, AcquisitionState, Option<UnavailableReason>)>> {
        if works.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = works.iter().map(|id| id.as_uuid()).collect();
        let rows = sqlx::query(
            "select work_id, state, unavailable_reason from ops.acquisition \
             where work_id = any($1) order by work_id",
        )
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let reason: Option<String> = row.try_get("unavailable_reason")?;
                Ok((
                    WorkId::from_uuid(row.try_get("work_id")?),
                    row.try_get::<String, _>("state")?.parse()?,
                    reason.map(|text| text.parse()).transpose()?,
                ))
            })
            .collect()
    }

    /// Return to `unresolved`, clearing stage outputs and the reason but keeping
    /// `attempts` and `trials`.
    pub async fn reset_to_unresolved(&self, work: WorkId) -> Result<()> {
        self.execute_for(
            work,
            "update ops.acquisition set state = 'unresolved', unavailable_reason = null, \
                 identifiers = null, candidates = '[]', source = null, pdf_url = null, \
                 final_url = null, sha256 = null, bytes = null, blob_id = null, \
                 document_id = null, document_version_id = null, updated_at = now() \
             where work_id = $1",
        )
        .await
    }

    /// Store the Work's identifiers and advance to `identifier_found`.
    pub async fn record_identifiers(&self, work: WorkId, identifiers: &Value) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set identifiers = $2, state = 'identifier_found', \
                 unavailable_reason = null, updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(identifiers)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store the ordered candidate links and advance to `link_found`.
    pub async fn record_candidates(
        &self,
        work: WorkId,
        candidates: &[CandidateRecord],
    ) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set candidates = $2, state = 'link_found', \
                 unavailable_reason = null, updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(encode(candidates)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Append a trial, keeping only the newest [`MAX_TRIALS`].
    pub async fn push_trial(&self, work: WorkId, trial: TrialRecord) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set trials = coalesce(( \
                 select jsonb_agg(entry order by position) from ( \
                     select entry, position \
                     from jsonb_array_elements(trials || jsonb_build_array($2::jsonb)) \
                         with ordinality as item(entry, position) \
                     order by position desc limit $3 \
                 ) newest \
             ), '[]'::jsonb), updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(encode(&trial)?)
        .bind(MAX_TRIALS)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store a downloaded PDF and advance to `downloaded`.
    pub async fn record_download(&self, work: WorkId, download: DownloadRecord) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set state = 'downloaded', unavailable_reason = null, \
                 source = $2, pdf_url = $3, final_url = $4, sha256 = $5, bytes = $6, \
                 blob_id = $7, last_error = null, updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(download.source.as_str())
        .bind(download.pdf_url)
        .bind(download.final_url)
        .bind(download.sha256)
        .bind(download.bytes)
        .bind(download.blob.as_uuid())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record the mapped document and advance to `mapped`.
    pub async fn mark_mapped(
        &self,
        work: WorkId,
        document: DocumentId,
        version: Option<DocumentVersionId>,
    ) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set state = 'mapped', unavailable_reason = null, \
                 document_id = $2, document_version_id = $3, last_error = null, \
                 checked_at = now(), updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(document.as_uuid())
        .bind(version.map(DocumentVersionId::as_uuid))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record the verdict that no copy can be obtained, with `checked_at` now.
    pub async fn mark_unavailable(&self, work: WorkId, reason: UnavailableReason) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set state = 'unavailable', unavailable_reason = $2, \
                 checked_at = now(), updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(reason.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Count a failed run and keep its message; the state is unchanged.
    pub async fn record_failure(&self, work: WorkId, error: &str) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set attempts = attempts + 1, last_error = $2, \
                 checked_at = now(), updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fall back (or move) to a non-terminal state without touching other columns.
    pub async fn set_state(&self, work: WorkId, state: AcquisitionState) -> Result<()> {
        sqlx::query(
            "update ops.acquisition set state = $2, \
                 unavailable_reason = case when $2 = 'unavailable' \
                     then unavailable_reason else null end, \
                 updated_at = now() where work_id = $1",
        )
        .bind(work.as_uuid())
        .bind(state.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Count one run that performed network work.
    pub async fn bump_attempts(&self, work: WorkId) -> Result<()> {
        self.execute_for(
            work,
            "update ops.acquisition set attempts = attempts + 1, updated_at = now() \
             where work_id = $1",
        )
        .await
    }

    /// Works cited by `citing` that have no present document in the workspace,
    /// best candidates first: an arXiv identifier (direct or a `DataCite`
    /// `10.48550/arxiv.*` DOI), then any DOI, then newest year (unknown last),
    /// then title. `skipped` marks Works that are already mapped or, unless
    /// `retry_unavailable`, unavailable within the recheck window.
    pub async fn cited_targets(
        &self,
        citing: WorkId,
        workspace: WorkspaceId,
        retry_unavailable: bool,
    ) -> Result<Vec<CitedTarget>> {
        let rows = sqlx::query(
            "select w.id, \
                 coalesce(a.state = 'mapped' or (a.state = 'unavailable' and not $3 \
                     and a.checked_at > now() - make_interval(days => $4)), false) as skipped \
             from graph.work w \
             left join ops.acquisition a on a.work_id = w.id \
             where w.id in (select cited_work_id from graph.citation_assertion \
                            where citing_work_id = $1 and status in ('asserted', 'accepted')) \
               and w.id <> $1 \
               and not exists (select 1 from reader.document d \
                               where d.workspace_id = $2 and d.work_id = w.id \
                                 and d.relative_path is not null) \
             order by \
                 exists (select 1 from graph.work_identifier i \
                         where i.work_id = w.id and i.status = 'active' \
                           and (i.scheme = 'arxiv' or (i.scheme = 'doi' \
                                and i.value_normalized like '10.48550/arxiv.%'))) desc, \
                 exists (select 1 from graph.work_identifier i \
                         where i.work_id = w.id and i.status = 'active' \
                           and i.scheme = 'doi') desc, \
                 w.display_year desc nulls last, w.display_title nulls last, w.id",
        )
        .bind(citing.as_uuid())
        .bind(workspace.as_uuid())
        .bind(retry_unavailable)
        .bind(RECHECK_DAYS)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(CitedTarget {
                    work_id: WorkId::from_uuid(row.try_get("id")?),
                    skipped: row.try_get("skipped")?,
                })
            })
            .collect()
    }

    async fn execute_for(&self, work: WorkId, sql: &str) -> Result<()> {
        sqlx::query(sql)
            .bind(work.as_uuid())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| sqlx::Error::Encode(Box::new(error)).into())
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| sqlx::Error::Decode(Box::new(error)).into())
}

fn row_from_pg(row: &PgRow) -> Result<AcquisitionRow> {
    let reason: Option<String> = row.try_get("unavailable_reason")?;
    let source: Option<String> = row.try_get("source")?;
    Ok(AcquisitionRow {
        work_id: WorkId::from_uuid(row.try_get("work_id")?),
        state: row.try_get::<String, _>("state")?.parse()?,
        unavailable_reason: reason.map(|text| text.parse()).transpose()?,
        identifiers: row.try_get("identifiers")?,
        candidates: decode(row.try_get("candidates")?)?,
        trials: decode(row.try_get("trials")?)?,
        source: source.map(|text| text.parse()).transpose()?,
        pdf_url: row.try_get("pdf_url")?,
        final_url: row.try_get("final_url")?,
        sha256: row.try_get("sha256")?,
        bytes: row.try_get("bytes")?,
        blob_id: row
            .try_get::<Option<Uuid>, _>("blob_id")?
            .map(BlobId::from_uuid),
        document_id: row
            .try_get::<Option<Uuid>, _>("document_id")?
            .map(DocumentId::from_uuid),
        document_version_id: row
            .try_get::<Option<Uuid>, _>("document_version_id")?
            .map(DocumentVersionId::from_uuid),
        attempts: row.try_get("attempts")?,
        last_error: row.try_get("last_error")?,
        checked_at: row.try_get("checked_at")?,
        lease_until: row.try_get("lease_until")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentRepository;
    use crate::test_support;

    fn trial(index: usize) -> TrialRecord {
        TrialRecord {
            url: format!("https://example.org/{index}.pdf"),
            origin: CandidateOrigin::OpenalexBest,
            outcome: format!("http_503_{index}"),
            at: Utc::now(),
        }
    }

    macro_rules! db {
        () => {{
            let guard = test_support::lock().await;
            let Some(pool) = test_support::pool().await else {
                eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
                return;
            };
            (guard, pool)
        }};
    }

    #[tokio::test]
    async fn should_insert_row_and_grant_lease_when_claiming_first_time() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        assert!(repo.get(work).await.expect("get").is_none());

        assert!(repo.claim(work).await.expect("claim"));

        let row = repo.get(work).await.expect("get").expect("row");
        assert_eq!(row.state, AcquisitionState::Unresolved);
        assert_eq!(row.attempts, 0);
        assert!(row.candidates.is_empty());
        assert!(row.lease_until.expect("lease") > Utc::now());
        repo.release(work).await.expect("release");
        assert!(repo.get(work).await.unwrap().unwrap().lease_until.is_none());
    }

    #[tokio::test]
    async fn should_refuse_second_claim_when_lease_is_live() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        assert!(repo.claim(work).await.expect("first"));
        assert!(!repo.claim(work).await.expect("second"));
        repo.release(work).await.expect("release");
        assert!(repo.claim(work).await.expect("after release"));
    }

    #[tokio::test]
    async fn should_grant_claim_when_lease_expired() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        assert!(repo.claim(work).await.expect("first"));
        sqlx::query(
            "update ops.acquisition set lease_until = now() - interval '1 minute' \
             where work_id = $1",
        )
        .bind(work.as_uuid())
        .execute(&pool)
        .await
        .expect("expire");
        assert!(repo.claim(work).await.expect("reclaim"));
    }

    #[tokio::test]
    async fn should_store_reason_and_checked_at_when_marking_unavailable() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        repo.claim(work).await.expect("claim");

        repo.mark_unavailable(work, UnavailableReason::NoOaLocation)
            .await
            .expect("mark");

        let row = repo.get(work).await.unwrap().unwrap();
        assert_eq!(row.state, AcquisitionState::Unavailable);
        assert_eq!(
            row.unavailable_reason,
            Some(UnavailableReason::NoOaLocation)
        );
        assert!(row.checked_at.is_some());
        let states = repo.states_for(&[work]).await.expect("states");
        assert_eq!(
            states,
            vec![(
                work,
                AcquisitionState::Unavailable,
                Some(UnavailableReason::NoOaLocation)
            )]
        );
    }

    #[tokio::test]
    async fn should_violate_check_when_unavailable_without_reason() {
        let (_guard, pool) = db!();
        let work = test_support::new_work(&pool).await;
        let without_reason =
            sqlx::query("insert into ops.acquisition (work_id, state) values ($1, 'unavailable')")
                .bind(work.as_uuid())
                .execute(&pool)
                .await;
        assert!(without_reason.is_err(), "unavailable needs a reason");

        let reason_without_state = sqlx::query(
            "insert into ops.acquisition (work_id, state, unavailable_reason) \
             values ($1, 'mapped', 'no_identifier')",
        )
        .bind(work.as_uuid())
        .execute(&pool)
        .await;
        assert!(reason_without_state.is_err(), "a reason needs unavailable");

        let unknown_reason = sqlx::query(
            "insert into ops.acquisition (work_id, state, unavailable_reason) \
             values ($1, 'unavailable', 'paywalled')",
        )
        .bind(work.as_uuid())
        .execute(&pool)
        .await;
        assert!(unknown_reason.is_err(), "reason must be a known value");
    }

    #[tokio::test]
    async fn should_keep_last_twenty_trials_when_pushing_more() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        repo.claim(work).await.expect("claim");

        for index in 0..25 {
            repo.push_trial(work, trial(index)).await.expect("push");
        }

        let trials = repo.get(work).await.unwrap().unwrap().trials;
        assert_eq!(trials.len(), 20);
        assert_eq!(trials.first().unwrap().outcome, "http_503_5");
        assert_eq!(trials.last().unwrap().outcome, "http_503_24");
    }

    #[tokio::test]
    async fn should_increment_attempts_and_keep_state_when_recording_failure() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        repo.claim(work).await.expect("claim");
        repo.record_candidates(
            work,
            &[CandidateRecord {
                url: "https://arxiv.org/pdf/1606.03137".to_owned(),
                origin: CandidateOrigin::Arxiv,
                source_record_id: None,
            }],
        )
        .await
        .expect("candidates");

        repo.record_failure(work, "http 503").await.expect("first");
        repo.record_failure(work, "timeout").await.expect("second");

        let row = repo.get(work).await.unwrap().unwrap();
        assert_eq!(row.state, AcquisitionState::LinkFound);
        assert_eq!(row.attempts, 2);
        assert_eq!(row.last_error.as_deref(), Some("timeout"));
        assert_eq!(row.candidates.len(), 1);
        assert_eq!(row.candidates[0].origin, CandidateOrigin::Arxiv);
    }

    #[tokio::test]
    async fn should_clear_stage_outputs_and_keep_attempts_when_resetting() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let documents = DocumentRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let blob = documents
            .upsert_blob(
                &format!("sha256/reset/{}.pdf", Uuid::new_v4()),
                "reset-sha",
                10,
                "application/pdf",
            )
            .await
            .expect("blob");
        repo.claim(work).await.expect("claim");
        repo.record_identifiers(work, &serde_json::json!({"doi": "10.1/x"}))
            .await
            .expect("identifiers");
        repo.push_trial(work, trial(1)).await.expect("trial");
        repo.record_download(
            work,
            DownloadRecord {
                source: CandidateOrigin::Arxiv,
                pdf_url: "https://arxiv.org/pdf/1".to_owned(),
                final_url: "https://arxiv.org/pdf/1v2".to_owned(),
                sha256: "reset-sha".to_owned(),
                bytes: 10,
                blob,
            },
        )
        .await
        .expect("download");
        repo.record_failure(work, "boom").await.expect("failure");
        repo.mark_unavailable(work, UnavailableReason::AllLinksFailed)
            .await
            .expect("unavailable");

        repo.reset_to_unresolved(work).await.expect("reset");

        let row = repo.get(work).await.unwrap().unwrap();
        assert_eq!(row.state, AcquisitionState::Unresolved);
        assert_eq!(row.unavailable_reason, None);
        assert_eq!(row.identifiers, None);
        assert!(row.candidates.is_empty());
        assert_eq!(
            (
                row.source,
                row.pdf_url,
                row.final_url,
                row.sha256,
                row.bytes
            ),
            (None, None, None, None, None)
        );
        assert_eq!(row.blob_id, None);
        assert_eq!(row.document_id, None);
        assert_eq!(row.attempts, 1);
        assert_eq!(row.trials.len(), 1);
    }

    #[tokio::test]
    async fn should_return_states_only_for_works_with_rows() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let with_row = test_support::new_work(&pool).await;
        let without_row = test_support::new_work(&pool).await;
        repo.claim(with_row).await.expect("claim");

        let states = repo
            .states_for(&[with_row, without_row])
            .await
            .expect("states");

        assert_eq!(states, vec![(with_row, AcquisitionState::Unresolved, None)]);
        assert!(repo.states_for(&[]).await.expect("empty").is_empty());
    }

    #[tokio::test]
    async fn should_record_mapping_and_bump_attempts_when_mapped() {
        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let work = test_support::new_work(&pool).await;
        let version = test_support::new_version(&pool).await;
        let document: Uuid =
            sqlx::query_scalar("select document_id from reader.document_version where id = $1")
                .bind(version.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("document");
        let document = DocumentId::from_uuid(document);
        repo.claim(work).await.expect("claim");
        repo.bump_attempts(work).await.expect("bump");

        repo.mark_mapped(work, document, Some(version))
            .await
            .expect("mapped");

        let row = repo.get(work).await.unwrap().unwrap();
        assert_eq!(row.state, AcquisitionState::Mapped);
        assert_eq!(row.document_id, Some(document));
        assert_eq!(row.document_version_id, Some(version));
        assert_eq!(row.attempts, 1);
        repo.set_state(work, AcquisitionState::Downloaded)
            .await
            .expect("set state");
        assert_eq!(
            repo.get(work).await.unwrap().unwrap().state,
            AcquisitionState::Downloaded
        );
    }

    #[tokio::test]
    async fn should_order_cited_targets_and_flag_skips_when_listing_for_citing_work() {
        use crate::citation::CitationRepository;
        use crate::work::WorkRepository;
        use lysilogy_core::IdentifierScheme;

        let (_guard, pool) = db!();
        let repo = AcquisitionRepository::new(pool.clone());
        let works = WorkRepository::new(pool.clone());
        let citations = CitationRepository::new(pool.clone());
        let documents = DocumentRepository::new(pool.clone());
        let workspace = documents
            .create_workspace(&format!("cited-{}", Uuid::new_v4()))
            .await
            .expect("workspace");
        let citing = test_support::new_work(&pool).await;
        let marker = Uuid::new_v4().simple().to_string();

        let plain = test_support::new_work(&pool).await;
        let old_doi = test_support::new_work(&pool).await;
        let new_doi = test_support::new_work(&pool).await;
        let arxiv_doi = test_support::new_work(&pool).await;
        let mapped = test_support::new_work(&pool).await;
        let unavailable = test_support::new_work(&pool).await;
        for (work, year) in [(old_doi, 2010), (new_doi, 2020), (arxiv_doi, 2001)] {
            works.set_display_year(work, year).await.expect("year");
        }
        for (work, doi) in [
            (old_doi, format!("10.1000/old-{marker}")),
            (new_doi, format!("10.1000/new-{marker}")),
            (arxiv_doi, format!("10.48550/arxiv.{marker}")),
        ] {
            works
                .add_identifier(
                    work,
                    IdentifierScheme::Doi,
                    &doi,
                    None,
                    lysilogy_core::IdentifierStatus::Active,
                )
                .await
                .expect("doi");
        }
        for work in [plain, old_doi, new_doi, arxiv_doi, mapped, unavailable] {
            citations
                .add_assertion(citing, work, None)
                .await
                .expect("assertion");
        }
        repo.claim(mapped).await.expect("claim");
        let version = test_support::new_version(&pool).await;
        let document: Uuid =
            sqlx::query_scalar("select document_id from reader.document_version where id = $1")
                .bind(version.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("document");
        repo.mark_mapped(mapped, DocumentId::from_uuid(document), None)
            .await
            .expect("mapped");
        repo.claim(unavailable).await.expect("claim");
        repo.mark_unavailable(unavailable, UnavailableReason::NoOaLocation)
            .await
            .expect("unavailable");

        let targets = repo
            .cited_targets(citing, workspace, false)
            .await
            .expect("targets");
        let order: Vec<WorkId> = targets.iter().map(|target| target.work_id).collect();
        assert_eq!(order[..4], [arxiv_doi, new_doi, old_doi, plain]);
        let skipped: Vec<WorkId> = targets
            .iter()
            .filter(|target| target.skipped)
            .map(|target| target.work_id)
            .collect();
        assert_eq!(skipped.len(), 2);
        assert!(skipped.contains(&mapped) && skipped.contains(&unavailable));

        let retrying = repo
            .cited_targets(citing, workspace, true)
            .await
            .expect("targets");
        let still_skipped: Vec<WorkId> = retrying
            .iter()
            .filter(|target| target.skipped)
            .map(|target| target.work_id)
            .collect();
        assert_eq!(still_skipped, vec![mapped]);
    }
}
