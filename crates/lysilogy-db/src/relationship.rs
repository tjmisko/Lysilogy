//! Persistence for relationship claims: labels on citation edges.
//!
//! `label` holds the relation code and `explanation` holds the reason. Claims
//! are never destroyed: an edit inserts a new user claim and points the old
//! row's `superseded_by` at it, so only rows with a null `superseded_by` are
//! current.

use chrono::{DateTime, Utc};
use lysilogy_core::{
    AssertionId, MentionId, RelationshipClaimId, RelationshipOrigin, ReviewState, SourceRecordId,
    WorkId,
};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{DbError, Result};

/// A stored relationship claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredClaim {
    pub id: RelationshipClaimId,
    pub assertion: AssertionId,
    pub mention: Option<MentionId>,
    /// The relation code.
    pub relation: String,
    /// The reason the relation holds.
    pub reason: Option<String>,
    /// Verbatim text from the mention's passage.
    pub quote: Option<String>,
    pub model: Option<String>,
    pub origin: RelationshipOrigin,
    pub review_state: ReviewState,
    pub source_record: Option<SourceRecordId>,
    pub superseded_by: Option<RelationshipClaimId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An unreviewed AI claim to store.
#[derive(Clone, Copy, Debug)]
pub struct NewAiClaim<'a> {
    pub assertion: AssertionId,
    pub mention: Option<MentionId>,
    pub relation: &'a str,
    pub reason: &'a str,
    pub quote: &'a str,
    pub model: &'a str,
    /// The cached model output that produced this claim.
    pub record: SourceRecordId,
}

/// A claim with the passage and document location of its mention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredClaimWithMention {
    pub claim: StoredClaim,
    pub citing: WorkId,
    pub cited: WorkId,
    pub passage: Option<String>,
    pub passage_start: Option<i32>,
    pub passage_end: Option<i32>,
    pub marker_start: Option<i32>,
    pub marker_end: Option<i32>,
    pub page: Option<i32>,
    /// Library-relative path of the document holding the mention.
    pub document_path: Option<String>,
}

const CLAIM_COLUMNS: &str = "c.id, c.assertion_id, c.mention_id, c.label, c.explanation, c.quote, \
     c.model, c.origin, c.review_state, c.source_record_id, c.superseded_by, c.created_at, \
     c.updated_at";

/// Reads and writes relationship claims.
#[derive(Clone)]
pub struct RelationshipRepository {
    pool: PgPool,
}

impl RelationshipRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Store an unreviewed AI claim. Returns `None` when the assertion already
    /// has a claim from the same source record.
    pub async fn insert_ai_claim(
        &self,
        claim: &NewAiClaim<'_>,
    ) -> Result<Option<RelationshipClaimId>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "insert into graph.relationship_claim \
                 (assertion_id, mention_id, label, explanation, quote, model, source_record_id, \
                  origin, review_state) \
             values ($1, $2, $3, $4, $5, $6, $7, 'ai', 'unreviewed') \
             on conflict (assertion_id, source_record_id) \
                 where source_record_id is not null do nothing \
             returning id",
        )
        .bind(claim.assertion.as_uuid())
        .bind(claim.mention.map(MentionId::as_uuid))
        .bind(claim.relation)
        .bind(claim.reason)
        .bind(claim.quote)
        .bind(claim.model)
        .bind(claim.record.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        Ok(id.map(RelationshipClaimId::from_uuid))
    }

    /// Whether the assertion has a claim that has not been superseded.
    pub async fn has_current_claim(&self, assertion: AssertionId) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "select exists (select 1 from graph.relationship_claim \
             where assertion_id = $1 and superseded_by is null)",
        )
        .bind(assertion.as_uuid())
        .fetch_one(&self.pool)
        .await?;
        Ok(exists)
    }

    /// Set the review state of a current claim.
    ///
    /// # Errors
    /// `DbError::Conflict` when the claim was superseded, and a not-found
    /// database error when it does not exist.
    pub async fn set_review_state(
        &self,
        claim: RelationshipClaimId,
        state: ReviewState,
    ) -> Result<StoredClaim> {
        let row = sqlx::query(&format!(
            "update graph.relationship_claim c \
             set review_state = $2, updated_at = now() \
             where c.id = $1 and c.superseded_by is null \
             returning {CLAIM_COLUMNS}"
        ))
        .bind(claim.as_uuid())
        .bind(state.as_str())
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = row {
            return claim_from_row(&row);
        }
        Err(self.missing_or_superseded(claim).await)
    }

    /// Replace a current claim with a user-authored one on the same assertion
    /// and mention. The old row keeps its content and points at the new one.
    ///
    /// # Errors
    /// `DbError::Conflict` when the claim was superseded, and a not-found
    /// database error when it does not exist.
    pub async fn supersede_with_user_claim(
        &self,
        claim: RelationshipClaimId,
        relation: &str,
        reason: &str,
        quote: Option<&str>,
    ) -> Result<StoredClaim> {
        let mut transaction = self.pool.begin().await?;
        let old = sqlx::query(
            "select assertion_id, mention_id, superseded_by from graph.relationship_claim \
             where id = $1 for update",
        )
        .bind(claim.as_uuid())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(old) = old else {
            return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
        };
        let superseded_by: Option<Uuid> = old.try_get("superseded_by")?;
        if superseded_by.is_some() {
            return Err(DbError::Conflict(format!("claim {claim} was superseded")));
        }
        let assertion: Uuid = old.try_get("assertion_id")?;
        let mention: Option<Uuid> = old.try_get("mention_id")?;
        let row = sqlx::query(&format!(
            "insert into graph.relationship_claim \
                 (assertion_id, mention_id, label, explanation, quote, origin, review_state) \
             values ($1, $2, $3, $4, $5, 'user', 'accepted') \
             returning {}",
            CLAIM_COLUMNS.replace("c.", "")
        ))
        .bind(assertion)
        .bind(mention)
        .bind(relation)
        .bind(reason)
        .bind(quote)
        .fetch_one(&mut *transaction)
        .await?;
        let created = claim_from_row(&row)?;
        sqlx::query(
            "update graph.relationship_claim set superseded_by = $2, updated_at = now() \
             where id = $1",
        )
        .bind(claim.as_uuid())
        .bind(created.id.as_uuid())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(created)
    }

    /// Current claims on the given `(citing, cited)` edges, with the passage,
    /// offsets, page, and document path of each mention. Rejected claims are
    /// included so callers can show them dimmed.
    pub async fn current_claims_for_edges(
        &self,
        pairs: &[(WorkId, WorkId)],
    ) -> Result<Vec<StoredClaimWithMention>> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }
        let citing: Vec<Uuid> = pairs.iter().map(|pair| pair.0.as_uuid()).collect();
        let cited: Vec<Uuid> = pairs.iter().map(|pair| pair.1.as_uuid()).collect();
        let rows = sqlx::query(&format!(
            "select {CLAIM_COLUMNS}, a.citing_work_id, a.cited_work_id, \
                    m.quote_or_anchor, m.passage_start, m.passage_end, m.marker_start, \
                    m.marker_end, m.page, d.relative_path \
             from unnest($1::uuid[], $2::uuid[]) as pair(citing, cited) \
             join graph.citation_assertion a \
                 on a.citing_work_id = pair.citing and a.cited_work_id = pair.cited \
             join graph.relationship_claim c on c.assertion_id = a.id \
             left join graph.citation_mention m on m.id = c.mention_id \
             left join reader.document_version v on v.id = m.document_version_id \
             left join reader.document d on d.id = v.document_id \
             where c.superseded_by is null \
             order by a.citing_work_id, a.cited_work_id, c.created_at, c.id"
        ))
        .bind(&citing)
        .bind(&cited)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(with_mention_from_row).collect()
    }

    /// One claim (current or superseded) with its mention location.
    pub async fn get(&self, claim: RelationshipClaimId) -> Result<Option<StoredClaimWithMention>> {
        let row = sqlx::query(&format!(
            "select {CLAIM_COLUMNS}, a.citing_work_id, a.cited_work_id, \
                    m.quote_or_anchor, m.passage_start, m.passage_end, m.marker_start, \
                    m.marker_end, m.page, d.relative_path \
             from graph.relationship_claim c \
             join graph.citation_assertion a on a.id = c.assertion_id \
             left join graph.citation_mention m on m.id = c.mention_id \
             left join reader.document_version v on v.id = m.document_version_id \
             left join reader.document d on d.id = v.document_id \
             where c.id = $1"
        ))
        .bind(claim.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(with_mention_from_row).transpose()
    }

    async fn missing_or_superseded(&self, claim: RelationshipClaimId) -> DbError {
        let exists = sqlx::query_scalar::<_, bool>(
            "select exists (select 1 from graph.relationship_claim where id = $1)",
        )
        .bind(claim.as_uuid())
        .fetch_one(&self.pool)
        .await;
        match exists {
            Ok(true) => DbError::Conflict(format!("claim {claim} was superseded")),
            Ok(false) => DbError::Sqlx(sqlx::Error::RowNotFound),
            Err(error) => DbError::Sqlx(error),
        }
    }
}

fn claim_from_row(row: &PgRow) -> Result<StoredClaim> {
    let origin: String = row.try_get("origin")?;
    let review_state: String = row.try_get("review_state")?;
    Ok(StoredClaim {
        id: RelationshipClaimId::from_uuid(row.try_get("id")?),
        assertion: AssertionId::from_uuid(row.try_get("assertion_id")?),
        mention: row
            .try_get::<Option<Uuid>, _>("mention_id")?
            .map(MentionId::from_uuid),
        relation: row.try_get("label")?,
        reason: row.try_get("explanation")?,
        quote: row.try_get("quote")?,
        model: row.try_get("model")?,
        origin: origin.parse()?,
        review_state: review_state.parse()?,
        source_record: row
            .try_get::<Option<Uuid>, _>("source_record_id")?
            .map(SourceRecordId::from_uuid),
        superseded_by: row
            .try_get::<Option<Uuid>, _>("superseded_by")?
            .map(RelationshipClaimId::from_uuid),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn with_mention_from_row(row: &PgRow) -> Result<StoredClaimWithMention> {
    Ok(StoredClaimWithMention {
        claim: claim_from_row(row)?,
        citing: WorkId::from_uuid(row.try_get("citing_work_id")?),
        cited: WorkId::from_uuid(row.try_get("cited_work_id")?),
        passage: row.try_get("quote_or_anchor")?,
        passage_start: row.try_get("passage_start")?,
        passage_end: row.try_get("passage_end")?,
        marker_start: row.try_get("marker_start")?,
        marker_end: row.try_get("marker_end")?,
        page: row.try_get("page")?,
        document_path: row.try_get("relative_path")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citation::CitationRepository;
    use crate::evidence::{EvidenceRepository, NewMention};
    use crate::test_support::{self, new_entries, new_record, new_version, new_work};
    use lysilogy_core::CitationStyle;

    struct Fixture {
        repo: RelationshipRepository,
        citing: WorkId,
        cited: WorkId,
        assertion: AssertionId,
        mention: MentionId,
        record: SourceRecordId,
    }

    async fn fixture(pool: &PgPool) -> Fixture {
        let citations = CitationRepository::new(pool.clone());
        let evidence = EvidenceRepository::new(pool.clone());
        let version = new_version(pool).await;
        let entry = new_entries(pool, version, 1).await[0];
        let (citing, cited) = (new_work(pool).await, new_work(pool).await);
        let assertion = citations
            .add_entry_assertion(citing, cited, entry)
            .await
            .expect("assertion");
        let mention = evidence
            .insert_mention(&NewMention {
                assertion,
                version,
                entry,
                style: CitationStyle::Numeric,
                marker_text: "[1]".into(),
                page: 2,
                marker_start: 500,
                marker_end: 503,
                passage_start: 480,
                passage_end: 540,
                passage: "We build on prior work [1] for this.".into(),
                rects: None,
            })
            .await
            .expect("mention")
            .expect("new mention");
        Fixture {
            repo: RelationshipRepository::new(pool.clone()),
            citing,
            cited,
            assertion,
            mention,
            record: new_record(pool).await,
        }
    }

    async fn insert(fixture: &Fixture) -> RelationshipClaimId {
        fixture
            .repo
            .insert_ai_claim(&NewAiClaim {
                assertion: fixture.assertion,
                mention: Some(fixture.mention),
                relation: "extends",
                reason: "It builds on the cited work.",
                quote: "build on prior work",
                model: "claude:default",
                record: fixture.record,
            })
            .await
            .expect("insert")
            .expect("claim id")
    }

    #[tokio::test]
    async fn should_insert_once_when_same_generation_inserted_twice() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(&pool).await;
        assert!(
            !fixture
                .repo
                .has_current_claim(fixture.assertion)
                .await
                .unwrap()
        );
        let first = insert(&fixture).await;
        let again = fixture
            .repo
            .insert_ai_claim(&NewAiClaim {
                assertion: fixture.assertion,
                mention: Some(fixture.mention),
                relation: "extends",
                reason: "x",
                quote: "y",
                model: "claude:default",
                record: fixture.record,
            })
            .await
            .expect("second");
        assert!(again.is_none(), "same (assertion, record) is skipped");
        assert!(
            fixture
                .repo
                .has_current_claim(fixture.assertion)
                .await
                .unwrap()
        );

        let claim = fixture.repo.get(first).await.unwrap().expect("claim");
        assert_eq!(claim.claim.origin, RelationshipOrigin::Ai);
        assert_eq!(claim.claim.review_state, ReviewState::Unreviewed);
        assert_eq!(claim.claim.relation, "extends");
        assert_eq!(
            claim.passage.as_deref(),
            Some("We build on prior work [1] for this.")
        );
        assert_eq!(claim.marker_start, Some(500));
        assert_eq!((claim.citing, claim.cited), (fixture.citing, fixture.cited));
    }

    #[tokio::test]
    async fn should_return_current_claims_with_mention_when_edge_listed() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(&pool).await;
        let id = insert(&fixture).await;
        let other = (new_work(&pool).await, new_work(&pool).await);
        let claims = fixture
            .repo
            .current_claims_for_edges(&[(fixture.citing, fixture.cited), other])
            .await
            .unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim.id, id);
        assert_eq!(claims[0].passage_start, Some(480));
        assert!(
            fixture
                .repo
                .current_claims_for_edges(&[])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn should_supersede_claim_when_user_edits_label() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(&pool).await;
        let old = insert(&fixture).await;
        let new = fixture
            .repo
            .supersede_with_user_claim(old, "critiques", "It disputes it.", Some("prior work"))
            .await
            .expect("supersede");
        assert_eq!(new.origin, RelationshipOrigin::User);
        assert_eq!(new.review_state, ReviewState::Accepted);
        assert_eq!(new.assertion, fixture.assertion);
        assert_eq!(new.mention, Some(fixture.mention));

        let old_row = fixture.repo.get(old).await.unwrap().expect("old");
        assert_eq!(old_row.claim.superseded_by, Some(new.id));
        assert_eq!(old_row.claim.relation, "extends", "old content is kept");

        let current = fixture
            .repo
            .current_claims_for_edges(&[(fixture.citing, fixture.cited)])
            .await
            .unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].claim.id, new.id);

        let again = fixture
            .repo
            .supersede_with_user_claim(old, "other", "r", Some("q"))
            .await;
        assert!(matches!(again, Err(DbError::Conflict(_))));
    }

    #[tokio::test]
    async fn should_refuse_review_when_claim_superseded() {
        let _guard = test_support::lock().await;
        let Some(pool) = test_support::pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let fixture = fixture(&pool).await;
        let old = insert(&fixture).await;
        let accepted = fixture
            .repo
            .set_review_state(old, ReviewState::Accepted)
            .await
            .expect("accept");
        assert_eq!(accepted.review_state, ReviewState::Accepted);
        let rejected = fixture
            .repo
            .set_review_state(old, ReviewState::Rejected)
            .await
            .expect("reject");
        assert_eq!(rejected.review_state, ReviewState::Rejected);
        assert!(
            fixture
                .repo
                .has_current_claim(fixture.assertion)
                .await
                .unwrap(),
            "a rejected claim is still current"
        );

        fixture
            .repo
            .supersede_with_user_claim(old, "other", "reason", Some("quote"))
            .await
            .expect("supersede");
        let result = fixture
            .repo
            .set_review_state(old, ReviewState::Accepted)
            .await;
        assert!(matches!(result, Err(DbError::Conflict(_))), "{result:?}");

        let missing = fixture
            .repo
            .set_review_state(RelationshipClaimId::new(), ReviewState::Accepted)
            .await;
        assert!(matches!(
            missing,
            Err(DbError::Sqlx(sqlx::Error::RowNotFound))
        ));
    }
}
