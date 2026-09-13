use super::{KbStore, Result, StoreError, projection};
use crate::kb::WorkId;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct NeighborhoodLimits {
    pub max_nodes: usize,
    pub max_edges: usize,
}
impl Default for NeighborhoodLimits {
    fn default() -> Self {
        Self {
            max_nodes: 500,
            max_edges: 5000,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Neighbor {
    pub id: WorkId,
    pub distance: u8,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NeighborhoodEdge {
    pub citing: WorkId,
    pub cited: WorkId,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Neighborhood {
    pub root: WorkId,
    pub nodes: Vec<Neighbor>,
    pub edges: Vec<NeighborhoodEdge>,
    pub nodes_truncated: bool,
    pub edges_truncated: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    pub ids: Vec<String>,
    pub truncated: bool,
}

impl KbStore {
    /// Two undirected hops over directed citations, followed by all directed
    /// edges induced by the returned node set. Output bounds are explicit;
    /// neither traversal depth nor hub expansion has a hidden LIMIT.
    pub fn two_hop(&self, root: &WorkId, limits: NeighborhoodLimits) -> Result<Neighborhood> {
        if limits.max_nodes == 0 || limits.max_nodes > 500_000 || limits.max_edges > 3_000_000 {
            return Err(StoreError::Invalid(
                "neighborhood bounds: 1..=500000 nodes, 0..=3000000 edges".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let graph = neighborhood(&transaction, root, limits)?;
        transaction.commit()?;
        Ok(graph)
    }

    /// Literal trigram phrase search. Query syntax is quoted, so punctuation or
    /// user operators cannot become FTS expressions. Sub-trigram inputs are
    /// rejected explicitly rather than causing a surprise full-table scan.
    pub fn search_titles(&self, query: &str, limit: usize) -> Result<SearchResults> {
        self.search("work_search", "title", query, limit)
    }
    pub fn search_names(&self, query: &str, limit: usize) -> Result<SearchResults> {
        self.search("person_search", "name", query, limit)
    }
    fn search(
        &self,
        table: &str,
        column: &str,
        query: &str,
        limit: usize,
    ) -> Result<SearchResults> {
        if query.chars().count() < 3 || query.len() > 4096 || limit == 0 || limit > 1000 {
            return Err(StoreError::Invalid(
                "search requires at least 3 characters, at most 4096 bytes, and a 1..1000 result limit".into(),
            ));
        }
        let query = format!("\"{}\"", query.replace('"', "\"\""));
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT id FROM {table} WHERE {column} MATCH ?1 ORDER BY id LIMIT ?2"
        ))?;
        let mut ids = statement
            .query_map(params![query, limit + 1], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let truncated = ids.len() > limit;
        ids.truncate(limit);
        Ok(SearchResults { ids, truncated })
    }
}

fn neighborhood(
    connection: &rusqlite::Connection,
    root: &WorkId,
    limits: NeighborhoodLimits,
) -> Result<Neighborhood> {
    let root = projection::resolve(&connection, root.as_str())?;
    let exists: Option<String> = connection
        .query_row("SELECT id FROM works WHERE id=?1", [&root], |row| {
            row.get(0)
        })
        .optional()?;
    if exists.is_none() {
        return Err(StoreError::Invalid(
            "neighborhood root is not a Work".into(),
        ));
    }
    connection.execute_batch("CREATE TEMP TABLE IF NOT EXISTS neighborhood_nodes(id TEXT PRIMARY KEY,distance INTEGER NOT NULL) WITHOUT ROWID; DELETE FROM neighborhood_nodes;")?;
    // The PK of citations serves outgoing edges; citations_incoming serves
    // incoming edges. UNION removes reciprocal and duplicate frontier nodes.
    let sql="WITH first(id) AS (
        SELECT cited FROM citations WHERE citing=?1 UNION SELECT citing FROM citations WHERE cited=?1
    ), reached(id,distance) AS (
        SELECT ?1,0 UNION ALL SELECT id,1 FROM first
        UNION ALL SELECT c.cited,2 FROM first f JOIN citations c ON c.citing=f.id
        UNION ALL SELECT c.citing,2 FROM first f JOIN citations c ON c.cited=f.id
    ) INSERT INTO neighborhood_nodes SELECT id,min(distance) FROM reached GROUP BY id ORDER BY min(distance),id LIMIT ?2";
    connection.execute(sql, params![root, limits.max_nodes + 1])?;
    let mut statement =
        connection.prepare("SELECT id,distance FROM neighborhood_nodes ORDER BY distance,id")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u8>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let nodes_truncated = rows.len() > limits.max_nodes;
    if let Some((extra, _)) = rows.get(limits.max_nodes) {
        connection.execute("DELETE FROM neighborhood_nodes WHERE id=?1", [extra])?;
    }
    let nodes = rows
        .into_iter()
        .take(limits.max_nodes)
        .map(|(id, distance)| {
            Ok(Neighbor {
                id: id.parse().map_err(StoreError::Invalid)?,
                distance,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    #[cfg(test)]
    FRONTIER_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
    let mut statement=connection.prepare("SELECT c.citing,c.cited FROM neighborhood_nodes n CROSS JOIN citations c ON c.citing=n.id JOIN neighborhood_nodes other ON other.id=c.cited ORDER BY c.citing,c.cited LIMIT ?1")?;
    let rows = statement
        .query_map([limits.max_edges + 1], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let edges_truncated = rows.len() > limits.max_edges;
    let edges = rows
        .into_iter()
        .take(limits.max_edges)
        .map(|(citing, cited)| {
            Ok(NeighborhoodEdge {
                citing: citing.parse().map_err(StoreError::Invalid)?,
                cited: cited.parse().map_err(StoreError::Invalid)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Neighborhood {
        root: root.parse().map_err(StoreError::Invalid)?,
        nodes,
        edges,
        nodes_truncated,
        edges_truncated,
    })
}

#[cfg(test)]
thread_local! {
    pub(super) static FRONTIER_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
