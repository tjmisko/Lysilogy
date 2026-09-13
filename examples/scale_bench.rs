//! Model-free measurements of the real catalog and extractor. Scheduling here is
//! experimental benchmark code; production `ingest` remains sequential.
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use clap::Parser;
use lysilogy::{
    Error, Result,
    extract::PdfExtractor,
    library::{CatalogEntry, LibraryCatalog},
    store::ArtifactStore,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    library: PathBuf,
    /// A new, empty directory inside an owned benchmark run.
    #[arg(long)]
    data: PathBuf,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    expected_count: u32,
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=20))]
    repeats: u8,
    #[arg(long, default_value_t = 10_000, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    extraction_limit: u32,
}

#[derive(Debug, Serialize)]
struct Batch {
    workers: u8,
    papers: usize,
    seconds: f64,
    papers_per_second: f64,
    output_sha256: String,
}

#[derive(Debug, Serialize)]
struct Observation {
    schema_version: u8,
    papers: usize,
    catalog_initial_seconds: f64,
    catalog_discovered_no_change_seconds: Vec<f64>,
    population_seconds: f64,
    populated_papers: usize,
    catalog_populated_initial_seconds: f64,
    catalog_populated_no_change_seconds: Vec<f64>,
    extraction: Vec<Batch>,
    production_ingest_workers: u8,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    println!("{}", serde_json::to_string(&measure(&args).await?)?);
    Ok(())
}

async fn measure(args: &Args) -> Result<Observation> {
    // Never reuse or recover arbitrary artifact data. Each invocation receives a
    // fresh directory from the external-vault orchestration script.
    tokio::fs::create_dir(&args.data)
        .await
        .map_err(|error| Error::io(&args.data, error))?;
    let store = ArtifactStore::new(&args.data);
    store.initialize().await?;
    let started = Instant::now();
    let mut catalog = LibraryCatalog::scan(&args.library, &store).await?;
    let catalog_initial_seconds = started.elapsed().as_secs_f64();
    let papers = catalog.overviews().len();
    if papers != args.expected_count as usize {
        return Err(Error::InvalidRequest(
            "benchmark catalog count differs from manifest".into(),
        ));
    }
    let catalog_discovered_no_change_seconds =
        rescan(&mut catalog, &store, papers, args.repeats).await?;
    let entries = catalog
        .overviews()
        .iter()
        .map(|paper| {
            catalog
                .get(&paper.id)
                .expect("overview belongs to catalog")
                .clone()
        })
        .collect::<Vec<_>>();
    let mut extraction = Vec::new();
    for repeat in 0..args.repeats {
        // Alternate order to expose, rather than hide, warm-cache effects.
        for workers in if repeat % 2 == 0 { [1, 4] } else { [4, 1] } {
            extraction.push(
                extract_batch(
                    &entries[..entries.len().min(args.extraction_limit as usize)],
                    workers,
                )
                .await?,
            );
        }
    }
    if extraction
        .iter()
        .any(|batch| batch.output_sha256 != extraction[0].output_sha256)
    {
        return Err(Error::InvalidRequest(
            "extraction outputs differ between trials".into(),
        ));
    }
    // Persist real extraction artifacts for the ENTIRE vault, independently of
    // the optional capacity sample. This setup is excluded from rescan timing.
    let started = Instant::now();
    for entry in &entries {
        let paper = PdfExtractor::default()
            .extract(&entry.source_path, &entry.overview.metadata)
            .await?;
        store.save_extraction(&entry.overview.id, &paper).await?;
    }
    let population_seconds = started.elapsed().as_secs_f64();
    let catalog_populated_initial_seconds = rescan(&mut catalog, &store, papers, 1).await?[0];
    let populated_papers = catalog
        .overviews()
        .iter()
        .filter(|paper| matches!(paper.status, lysilogy::domain::ProcessingStatus::Extracted))
        .count();
    if populated_papers != papers {
        return Err(Error::InvalidRequest(
            "not every generated paper has a valid extraction artifact".into(),
        ));
    }
    let catalog_populated_no_change_seconds =
        rescan(&mut catalog, &store, papers, args.repeats).await?;
    Ok(Observation {
        schema_version: 1,
        papers,
        catalog_initial_seconds,
        catalog_discovered_no_change_seconds,
        population_seconds,
        populated_papers,
        catalog_populated_initial_seconds,
        catalog_populated_no_change_seconds,
        extraction,
        production_ingest_workers: 1,
    })
}

async fn rescan(
    catalog: &mut LibraryCatalog,
    store: &ArtifactStore,
    papers: usize,
    repeats: u8,
) -> Result<Vec<f64>> {
    let mut seconds = Vec::new();
    for _ in 0..repeats {
        let started = Instant::now();
        let replacement = LibraryCatalog::scan(catalog.root(), store).await?;
        catalog.replace_with(replacement);
        seconds.push(started.elapsed().as_secs_f64());
        if catalog.overviews().len() != papers {
            return Err(Error::InvalidRequest(
                "benchmark rescan changed the catalog count".into(),
            ));
        }
    }
    Ok(seconds)
}

async fn extract_batch(entries: &[CatalogEntry], workers: u8) -> Result<Batch> {
    let started = Instant::now();
    let mut pending = JoinSet::new();
    let mut remaining = entries.iter();
    let mut outputs = BTreeMap::new();
    loop {
        while pending.len() < usize::from(workers) {
            let Some(entry) = remaining.next() else { break };
            let entry = entry.clone();
            pending.spawn(async move {
                let extracted = PdfExtractor::default()
                    .extract(&entry.source_path, &entry.overview.metadata)
                    .await?;
                let hash = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&(
                        &extracted.metadata,
                        &extracted.full_text(),
                        &extracted.layout,
                    ))?)
                );
                Ok::<_, Error>((entry.overview.relative_path, hash))
            });
        }
        let Some(result) = pending.join_next().await else {
            break;
        };
        let (path, hash) = result.map_err(|error| Error::Task(error.to_string()))??;
        outputs.insert(path, hash);
    }
    let seconds = started.elapsed().as_secs_f64();
    let count =
        u32::try_from(outputs.len()).map_err(|error| Error::InvalidRequest(error.to_string()))?;
    Ok(Batch {
        workers,
        papers: outputs.len(),
        seconds,
        papers_per_second: f64::from(count) / seconds,
        output_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(&outputs)?)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn should_report_each_phase_when_a_tiny_real_pdf_benchmark_completes() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("papers");
        std::fs::create_dir(&library).unwrap();
        std::fs::write(
            library.join("fixture.pdf"),
            include_bytes!("../tests/fixtures/cursor-objects.pdf"),
        )
        .unwrap();
        let observation = measure(&Args {
            library,
            data: root.path().join("data"),
            expected_count: 1,
            repeats: 2,
            extraction_limit: 1,
        })
        .await
        .unwrap();
        assert_eq!(observation.papers, 1);
        assert!(observation.catalog_initial_seconds > 0.0);
        assert_eq!(observation.catalog_discovered_no_change_seconds.len(), 2);
        assert!(
            observation
                .catalog_discovered_no_change_seconds
                .iter()
                .all(|value| *value > 0.0)
        );
        assert!(observation.population_seconds > 0.0);
        assert_eq!(observation.populated_papers, 1);
        assert!(observation.catalog_populated_initial_seconds > 0.0);
        assert_eq!(observation.catalog_populated_no_change_seconds.len(), 2);
        assert!(
            observation
                .catalog_populated_no_change_seconds
                .iter()
                .all(|value| *value > 0.0)
        );
        assert_eq!(
            observation
                .extraction
                .iter()
                .map(|batch| batch.workers)
                .collect::<Vec<_>>(),
            [1, 4, 4, 1]
        );
        assert!(
            observation
                .extraction
                .iter()
                .all(|batch| batch.papers == 1 && batch.seconds > 0.0)
        );
        assert_eq!(observation.production_ingest_workers, 1);
    }

    #[tokio::test]
    async fn should_preserve_existing_data_when_the_output_directory_is_reused() {
        let root = tempfile::tempdir().unwrap();
        let sentinel = root.path().join("keep");
        std::fs::write(&sentinel, b"existing").unwrap();
        assert!(
            measure(&Args {
                library: root.path().into(),
                data: root.path().into(),
                expected_count: 1,
                repeats: 1,
                extraction_limit: 1
            })
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"existing");
    }
}
