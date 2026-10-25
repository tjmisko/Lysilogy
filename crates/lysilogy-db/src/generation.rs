//! Persistence for agent generation runs (`ops.list_generation`) and the one transaction
//! that admits a run's nodes and edges to a list.
//!
//! Admission reuses the list repository's insert helpers, so a generated node obeys the
//! same limits, uniqueness and edge validation as one added by hand.

use chrono::{DateTime, Utc};
use lysilogy_core::{
    EdgeKind, EdgeRejection, EdgeSource, GenerationId, GenerationState, ListId, ListReview, NodeId,
    WorkId,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::Result;
use crate::list::{
    ListNode, NewEdge, NewNode, insert_edge, insert_new_node, lock_list, touch_list,
};

const GENERATION_COLUMNS: &str = "id, list_id, goal, options, state, report, \
    spend_usd::float8 as spend_usd, model_key, error, created_at, finished_at";

/// One stored generation run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GenerationRow {
    pub id: GenerationId,
    pub list_id: Option<ListId>,
    pub goal: String,
    pub options: Value,
    pub state: GenerationState,
    pub report: Option<Value>,
    pub spend_usd: f64,
    pub model_key: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// How an edge of an admission names its endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeRef {
    /// A node already in the list.
    Existing(NodeId),
    /// The node at this index of [`Admission::nodes`].
    New(usize),
}

/// An edge to add once the nodes exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionEdge {
    pub from: NodeRef,
    pub to: NodeRef,
    pub kind: EdgeKind,
}

/// Everything one run adds to a list.
#[derive(Clone, Debug)]
pub struct Admission {
    /// Appended in order; put topics before the nodes that depend on them.
    pub nodes: Vec<NewNode>,
    /// Added in order, as agent edges. An edge that closes a cycle is skipped, so the
    /// earlier edges win.
    pub edges: Vec<AdmissionEdge>,
    /// `proposed` when extending an existing list.
    pub edge_review: ListReview,
}

/// An edge an admission skipped, with why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedEdge {
    /// Index into [`Admission::edges`].
    pub index: usize,
    pub rejection: EdgeRejection,
}

/// What an admission wrote.
#[derive(Clone, Debug)]
pub struct Admitted {
    /// Parallel to [`Admission::nodes`].
    pub nodes: Vec<ListNode>,
    pub edges_added: usize,
    pub edges_skipped: Vec<SkippedEdge>,
}

/// Reads and writes generation runs.
#[derive(Clone)]
pub struct GenerationRepository {
    pool: PgPool,
}

impl GenerationRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record a run that has started.
    pub async fn create(
        &self,
        goal: &str,
        options: &Value,
        list: Option<ListId>,
        model_key: &str,
    ) -> Result<GenerationRow> {
        let row = sqlx::query(&format!(
            "insert into ops.list_generation (goal, options, state, list_id, model_key) \
             values ($1, $2, 'running', $3, $4) returning {GENERATION_COLUMNS}"
        ))
        .bind(goal)
        .bind(options)
        .bind(list.map(ListId::as_uuid))
        .bind(model_key)
        .fetch_one(&self.pool)
        .await?;
        generation_from_row(&row)
    }

    /// A run by ID.
    pub async fn get(&self, id: GenerationId) -> Result<Option<GenerationRow>> {
        let row = sqlx::query(&format!(
            "select {GENERATION_COLUMNS} from ops.list_generation where id = $1"
        ))
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(generation_from_row).transpose()
    }

    /// Replace the report of a run that is still going, so a poll sees its stage.
    pub async fn set_progress(
        &self,
        id: GenerationId,
        report: &Value,
        spend_usd: f64,
    ) -> Result<()> {
        sqlx::query(
            "update ops.list_generation set report = $2, spend_usd = $3::float8 \
             where id = $1 and state = 'running'",
        )
        .bind(id.as_uuid())
        .bind(report)
        .bind(spend_usd)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Close a run. `list` is the list the run wrote to, if it wrote one.
    pub async fn finish(
        &self,
        id: GenerationId,
        state: GenerationState,
        report: &Value,
        spend_usd: f64,
        list: Option<ListId>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "update ops.list_generation set state = $2, report = $3, spend_usd = $4::float8, \
                 list_id = coalesce($5, list_id), error = $6, finished_at = now() \
             where id = $1",
        )
        .bind(id.as_uuid())
        .bind(state.as_str())
        .bind(report)
        .bind(spend_usd)
        .bind(list.map(ListId::as_uuid))
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Works whose title mentions any of `keywords` (lower-case, letters and digits only),
    /// the ones matching most keywords first, then newest. Redirected Works are skipped.
    pub async fn works_matching_title(
        &self,
        keywords: &[String],
        limit: usize,
    ) -> Result<Vec<WorkId>> {
        if keywords.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let patterns: Vec<String> = keywords.iter().map(|word| format!("%{word}%")).collect();
        let rows = sqlx::query(
            "select w.id from graph.work w \
             where w.title_key like any($1) \
               and not exists (select 1 from graph.work_redirect r where r.old_work_id = w.id) \
             order by (select count(*) from unnest($1::text[]) p where w.title_key like p) desc, \
                      w.display_year desc nulls last, w.id \
             limit $2",
        )
        .bind(&patterns)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok(WorkId::from_uuid(row.try_get::<Uuid, _>("id")?)))
            .collect()
    }

    /// Write a run's nodes and edges to `list` in one transaction: either all of it lands
    /// or none. An edge that would close a cycle, or does not fit its endpoints, is skipped
    /// and reported; every other refusal (a full list, a duplicate Work) undoes the lot.
    pub async fn admit(&self, list: ListId, admission: Admission) -> Result<Admitted> {
        let mut transaction = self.pool.begin().await?;
        lock_list(&mut transaction, list).await?;
        let mut nodes = Vec::with_capacity(admission.nodes.len());
        for new in admission.nodes {
            nodes.push(insert_new_node(&mut transaction, list, new).await?);
        }
        let resolve = |reference: NodeRef| match reference {
            NodeRef::Existing(id) => Some(id),
            NodeRef::New(index) => nodes.get(index).map(|node| node.id),
        };
        let mut edges_added = 0;
        let mut edges_skipped = Vec::new();
        for (index, edge) in admission.edges.iter().enumerate() {
            let (Some(from), Some(to)) = (resolve(edge.from), resolve(edge.to)) else {
                edges_skipped.push(SkippedEdge {
                    index,
                    rejection: EdgeRejection::UnknownNode,
                });
                continue;
            };
            let new = NewEdge {
                from,
                to,
                kind: edge.kind,
                source: EdgeSource::Agent,
                review: admission.edge_review,
            };
            match insert_edge(&mut transaction, list, new).await? {
                Ok(_) => edges_added += 1,
                Err(rejection) => edges_skipped.push(SkippedEdge { index, rejection }),
            }
        }
        touch_list(&mut transaction, list).await?;
        transaction.commit().await?;
        Ok(Admitted {
            nodes,
            edges_added,
            edges_skipped,
        })
    }
}

fn generation_from_row(row: &PgRow) -> Result<GenerationRow> {
    let state: String = row.try_get("state")?;
    Ok(GenerationRow {
        id: GenerationId::from_uuid(row.try_get("id")?),
        list_id: row
            .try_get::<Option<Uuid>, _>("list_id")?
            .map(ListId::from_uuid),
        goal: row.try_get("goal")?,
        options: row.try_get("options")?,
        state: state.parse()?,
        report: row.try_get("report")?,
        spend_usd: row.try_get("spend_usd")?,
        model_key: row.try_get("model_key")?,
        error: row.try_get("error")?,
        created_at: row.try_get("created_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

#[cfg(test)]
mod tests {
    use lysilogy_core::{
        IdentifierScheme, ListOrigin, ListTitle, TopicLabel, WorkKind, WorkspaceId,
    };
    use serde_json::json;

    use super::*;
    use crate::list::{ListRepository, NewList};
    use crate::work::WorkRepository;

    async fn pool() -> Option<PgPool> {
        let url = std::env::var("LYSILOGY_TEST_DATABASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let pool = crate::connect(&url).await.expect("connect");
        crate::migrate(&pool).await.expect("migrate");
        Some(pool)
    }

    fn marker() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("x{}x{nanos}", std::process::id())
    }

    async fn workspace(pool: &PgPool) -> WorkspaceId {
        crate::document::DocumentRepository::new(pool.clone())
            .ensure_default_workspace()
            .await
            .expect("workspace")
    }

    async fn new_list(pool: &PgPool) -> ListId {
        let workspace = workspace(pool).await;
        let mut new = NewList::manual(ListTitle::parse("Generated").expect("title"));
        new.origin = ListOrigin::Generated;
        ListRepository::new(pool.clone())
            .create(workspace, new)
            .await
            .expect("list")
            .id
    }

    fn topic(label: &str) -> NewNode {
        NewNode::topic(TopicLabel::parse(label).expect("label"))
    }

    async fn work(pool: &PgPool, tag: &str) -> WorkId {
        WorkRepository::new(pool.clone())
            .resolve_or_create(
                IdentifierScheme::Doi,
                &format!("10.9999/{tag}"),
                WorkKind::Article,
                None,
            )
            .await
            .expect("work")
            .0
            .id
    }

    #[tokio::test]
    async fn should_round_trip_a_run_when_created_progressed_and_finished() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let repository = GenerationRepository::new(pool);
        let created = repository
            .create("learn rlhf", &json!({"size": 20}), None, "fake:model")
            .await
            .expect("create");
        assert_eq!(created.state, GenerationState::Running);
        assert!(created.spend_usd.abs() < f64::EPSILON);
        repository
            .set_progress(created.id, &json!({"stage": "verify"}), 0.0123)
            .await
            .expect("progress");
        let progressed = repository.get(created.id).await.expect("get").expect("row");
        assert_eq!(progressed.report, Some(json!({"stage": "verify"})));
        assert!((progressed.spend_usd - 0.0123).abs() < 1e-9);
        repository
            .finish(
                created.id,
                GenerationState::OverBudget,
                &json!({"done": true}),
                0.05,
                None,
                None,
            )
            .await
            .expect("finish");
        let finished = repository.get(created.id).await.expect("get").expect("row");
        assert_eq!(finished.state, GenerationState::OverBudget);
        assert!(finished.finished_at.is_some());
        // A late progress write does not reopen a finished run's report.
        repository
            .set_progress(created.id, &json!({"stage": "late"}), 9.0)
            .await
            .expect("progress");
        let after = repository.get(created.id).await.expect("get").expect("row");
        assert_eq!(after.report, Some(json!({"done": true})));
        assert_eq!(after.model_key.as_deref(), Some("fake:model"));
    }

    #[tokio::test]
    async fn should_find_nothing_when_the_generation_does_not_exist() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let found = GenerationRepository::new(pool)
            .get(GenerationId::new())
            .await
            .expect("get");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn should_skip_cycle_edges_and_keep_the_rest_when_admitting() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = new_list(&pool).await;
        let tag = marker();
        let paper = work(&pool, &format!("{tag}-a")).await;
        let admission = Admission {
            nodes: vec![
                topic("Alpha"),
                topic("Beta"),
                topic("Gamma"),
                NewNode::work(paper),
            ],
            edges: vec![
                AdmissionEdge {
                    from: NodeRef::New(0),
                    to: NodeRef::New(1),
                    kind: EdgeKind::Prerequisite,
                },
                AdmissionEdge {
                    from: NodeRef::New(1),
                    to: NodeRef::New(2),
                    kind: EdgeKind::Prerequisite,
                },
                AdmissionEdge {
                    from: NodeRef::New(2),
                    to: NodeRef::New(0),
                    kind: EdgeKind::Prerequisite,
                },
                AdmissionEdge {
                    from: NodeRef::New(0),
                    to: NodeRef::New(3),
                    kind: EdgeKind::Covers,
                },
                AdmissionEdge {
                    from: NodeRef::New(3),
                    to: NodeRef::New(0),
                    kind: EdgeKind::Covers,
                },
                AdmissionEdge {
                    from: NodeRef::New(9),
                    to: NodeRef::New(0),
                    kind: EdgeKind::Subtopic,
                },
            ],
            edge_review: ListReview::Proposed,
        };
        let admitted = GenerationRepository::new(pool.clone())
            .admit(list, admission)
            .await
            .expect("admit");
        assert_eq!(admitted.nodes.len(), 4);
        assert_eq!(admitted.edges_added, 3);
        let skipped: Vec<(usize, bool)> = admitted
            .edges_skipped
            .iter()
            .map(|skip| {
                (
                    skip.index,
                    matches!(skip.rejection, EdgeRejection::Cycle(_)),
                )
            })
            .collect();
        assert_eq!(skipped, vec![(2, true), (4, false), (5, false)]);
        let stored = ListRepository::new(pool).edges(list).await.expect("edges");
        assert_eq!(stored.len(), 3);
        assert!(
            stored
                .iter()
                .all(|edge| edge.review == ListReview::Proposed && edge.source == EdgeSource::Agent)
        );
    }

    #[tokio::test]
    async fn should_write_nothing_when_a_node_is_refused() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let list = new_list(&pool).await;
        let paper = work(&pool, &format!("{}-dup", marker())).await;
        let admission = Admission {
            nodes: vec![topic("Kept?"), NewNode::work(paper), NewNode::work(paper)],
            edges: Vec::new(),
            edge_review: ListReview::Accepted,
        };
        let error = GenerationRepository::new(pool.clone())
            .admit(list, admission)
            .await
            .expect_err("duplicate Work");
        assert!(error.to_string().contains("item_exists"), "{error}");
        let nodes = ListRepository::new(pool).nodes(list).await.expect("nodes");
        assert!(nodes.is_empty(), "the transaction rolled back");
    }

    #[tokio::test]
    async fn should_rank_by_keywords_matched_when_matching_titles() {
        let Some(pool) = pool().await else {
            eprintln!("skipping: LYSILOGY_TEST_DATABASE_URL not set");
            return;
        };
        let tag = marker().to_lowercase();
        let works = WorkRepository::new(pool.clone());
        let both = work(&pool, &format!("{tag}-both")).await;
        let one = work(&pool, &format!("{tag}-one")).await;
        works
            .set_display_title(both, &format!("Zorblax quuxing {tag}"))
            .await
            .expect("title");
        works
            .set_display_title(one, &format!("Zorblax only {tag}"))
            .await
            .expect("title");
        let repository = GenerationRepository::new(pool);
        let keywords = vec!["zorblax".to_owned(), "quuxing".to_owned(), tag.clone()];
        let found = repository
            .works_matching_title(&keywords, 5)
            .await
            .expect("search");
        let position = |id: WorkId| found.iter().position(|found| *found == id);
        assert!(
            position(both).is_some() && position(one).is_some(),
            "{found:?}"
        );
        assert!(position(both) < position(one), "most keywords first");
        assert!(
            repository
                .works_matching_title(&[], 5)
                .await
                .expect("search")
                .is_empty()
        );
    }
}
