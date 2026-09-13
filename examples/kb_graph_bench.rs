//! Exact-size offline graph benchmark. Synthetic rows are fixture setup only;
//! every timed query calls the production KbStore API and is checked by a separate BFS.
use clap::Parser;
use lysilogy::kb::{
    AcquisitionState, Work, WorkId, WorkType,
    store::{KbStore, NeighborhoodLimits},
};
use rusqlite::{Connection, params};
use serde::Serialize;
use std::{collections::BTreeSet, path::PathBuf, time::Instant};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    data: PathBuf,
    #[arg(long, default_value_t = 500_000)]
    works: usize,
    #[arg(long, default_value_t = 200)]
    queries: usize,
}
#[derive(Serialize)]
struct Sample {
    root: usize,
    stratum: &'static str,
    milliseconds: f64,
    nodes: usize,
    edges: usize,
    correct: bool,
    nodes_truncated: bool,
    edges_truncated: bool,
}
#[derive(Serialize)]
struct Report {
    schema_version: u32,
    works: usize,
    edges: usize,
    queries: usize,
    sqlite_version: String,
    fts5: bool,
    build_seconds: f64,
    samples: Vec<Sample>,
}
fn id(index: usize) -> String {
    format!("Wbench{index:06}")
}
fn destinations(source: usize, count: usize) -> Vec<usize> {
    let mut targets = Vec::new();
    let hubs = (count / 20).clamp(1, 100);
    for (slot, offset) in [1, 7, 31, 127, 509, 2039].into_iter().enumerate() {
        let mut target = if slot == 5 && source.is_multiple_of(5) {
            (source / 5) % hubs
        } else {
            (source + offset) % count
        };
        while target == source || targets.contains(&target) {
            target = (target + 1) % count;
        }
        targets.push(target);
    }
    targets
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.works < 100 || args.works > 500_000 || args.queries < 200 {
        return Err("expected 100..500000 Works and at least 200 queries".into());
    }
    if args.data.exists() {
        return Err("benchmark requires a fresh data directory".into());
    }
    if args.works == 500_000 {
        let cache = PathBuf::from(std::env::var("HOME")?).join(".cache/lysilogy");
        let parent = args
            .data
            .parent()
            .ok_or("benchmark data needs a parent")?
            .canonicalize()?;
        if !parent.starts_with(cache.canonicalize()?) {
            return Err("full benchmark must use ~/.cache/lysilogy".into());
        }
    }
    let started = Instant::now();
    let store = KbStore::open(&args.data)?;
    let mut connection = Connection::open(store.database_path())?;
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    let transaction = connection.transaction()?;
    {
        let mut allocation = transaction.prepare("INSERT INTO allocations VALUES(?1,'work',?1)")?;
        let mut insert = transaction.prepare("INSERT INTO works VALUES(?1,?2,?2,?3)")?;
        for index in 0..args.works {
            let id = id(index);
            let title = format!("Synthetic graph work {index:06}");
            let work = Work {
                id: id.parse()?,
                identifiers: Default::default(),
                title: Some(title.clone()),
                title_key: Some(title.clone()),
                year: Some(2020),
                venue: None,
                kind: WorkType::Preprint,
                versions: vec![],
                local_copies: vec![],
                acquisition: AcquisitionState::Unresolved,
            };
            allocation.execute([&id])?;
            insert.execute(params![id, title, serde_json::to_string(&work)?])?;
        }
    }
    let mut incoming = vec![Vec::new(); args.works];
    {
        let mut insert = transaction.prepare("INSERT INTO citations VALUES(?1,?2)")?;
        for source in 0..args.works {
            for destination in destinations(source, args.works) {
                insert.execute(params![id(source), id(destination)])?;
                incoming[destination].push(source);
            }
        }
    }
    transaction.commit()?;
    connection.execute_batch("ANALYZE; PRAGMA wal_checkpoint(PASSIVE);")?;
    let works: usize = connection.query_row("SELECT count(*) FROM works", [], |row| row.get(0))?;
    let edges: usize =
        connection.query_row("SELECT count(*) FROM citations", [], |row| row.get(0))?;
    if works != args.works || edges != args.works * 6 {
        return Err("graph inventory differs from fixture contract".into());
    }
    let fts5: bool = connection.query_row(
        "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
        [],
        |row| row.get(0),
    )?;
    let build_seconds = started.elapsed().as_secs_f64();
    eprintln!("built {works} Works and {edges} distinct directed edges in {build_seconds:.2}s");
    let hubs = (works / 20).clamp(1, 100);
    let mut samples = Vec::new();
    for query in 0..args.queries {
        let (root, stratum) = if query.is_multiple_of(5) {
            ((query / 5) % hubs, "hub")
        } else {
            ((query * 15485863 + 32452843) % works, "uniform")
        };
        let work_id: WorkId = id(root).parse()?;
        let started = Instant::now();
        let graph = store.two_hop(
            &work_id,
            NeighborhoodLimits {
                max_nodes: 500_000,
                max_edges: 3_000_000,
            },
        )?;
        let milliseconds = started.elapsed().as_secs_f64() * 1000.0;
        // Independent two-round adjacency BFS, outside the timing boundary.
        let mut expected = BTreeSet::from([root]);
        let mut frontier = BTreeSet::from([root]);
        for _ in 0..2 {
            let mut next = BTreeSet::new();
            for source in frontier {
                next.extend(destinations(source, works));
                next.extend(incoming[source].iter().copied());
            }
            next.retain(|node| !expected.contains(node));
            expected.extend(&next);
            frontier = next;
        }
        let expected_ids = expected
            .iter()
            .map(|index| id(*index))
            .collect::<BTreeSet<_>>();
        let actual_ids = graph
            .nodes
            .iter()
            .map(|node| node.id.to_string())
            .collect::<BTreeSet<_>>();
        let expected_edges = expected
            .iter()
            .flat_map(|source| {
                destinations(*source, works)
                    .into_iter()
                    .filter(|target| expected.contains(target))
                    .map(move |target| (id(*source), id(target)))
            })
            .collect::<BTreeSet<_>>();
        let actual_edges = graph
            .edges
            .iter()
            .map(|edge| (edge.citing.to_string(), edge.cited.to_string()))
            .collect::<BTreeSet<_>>();
        let correct = expected_ids == actual_ids && expected_edges == actual_edges;
        samples.push(Sample {
            root,
            stratum,
            milliseconds,
            nodes: graph.nodes.len(),
            edges: graph.edges.len(),
            correct,
            nodes_truncated: graph.nodes_truncated,
            edges_truncated: graph.edges_truncated,
        });
        if !correct || graph.nodes_truncated || graph.edges_truncated {
            return Err("production neighborhood differs from complete independent BFS".into());
        }
    }
    let report = Report {
        schema_version: 1,
        works,
        edges,
        queries: args.queries,
        sqlite_version: rusqlite::version().into(),
        fts5,
        build_seconds,
        samples,
    };
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn should_generate_six_distinct_nonself_edges_when_building_each_fixture_node() {
        for source in 0..1000 {
            let edges = destinations(source, 1000);
            assert_eq!(edges.iter().collect::<BTreeSet<_>>().len(), 6);
            assert!(!edges.contains(&source));
        }
    }
    #[test]
    fn should_include_hubs_when_generating_the_graph_fixture() {
        let mut counts = vec![0; 1000];
        for source in 0..1000 {
            for target in destinations(source, 1000) {
                counts[target] += 1;
            }
        }
        assert!(counts.iter().take(50).any(|count| *count > 8));
    }
}
