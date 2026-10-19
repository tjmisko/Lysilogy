//! Run every deterministic extraction stage on one PDF and write the result in
//! the shape `lysilogy extract` records, so the spike benchmark scores both
//! paths with the same code (handoff gate 4). Never calls a model.
//!
//! `cargo run --release --example deterministic_inventory -- <pdf> <out.json> <work-dir>`

use std::{path::PathBuf, time::Instant};

use lysilogy::{
    abstracts, domain::PaperMetadata, extract::PdfExtractor, literature::evidence, source_index,
};
use serde_json::{Value, json};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(pdf), Some(out), Some(work)) = (args.next(), args.next(), args.next()) else {
        return Err("usage: deterministic_inventory <pdf> <out.json> <work-dir>".into());
    };
    let (pdf, out, work) = (PathBuf::from(pdf), PathBuf::from(out), PathBuf::from(work));
    let started = Instant::now();

    let paper = PdfExtractor::default()
        .extract(&pdf, &PaperMetadata::default())
        .await?;
    let abstract_result = abstracts::extract(&paper);
    let index = source_index::load_or_build(&pdf, &work, true).await?;
    let bibliography =
        lysilogy_ingest::bibliography::extract(&evidence::source_text_from_index(&index));

    let objects = index
        .figures
        .iter()
        .map(|figure| {
            let kind = if figure.kind.to_ascii_lowercase().starts_with("tab")
                || figure.label.to_ascii_lowercase().starts_with("tab")
            {
                "table"
            } else {
                "figure"
            };
            json!({"kind": kind, "label": figure.label, "number": null, "page": figure.page, "caption": figure.caption})
        })
        .collect::<Vec<_>>();
    let references = bibliography
        .entries
        .iter()
        .map(|entry| {
            json!({
                "key": entry.fields.printed_key.value,
                "title": entry.fields.title.value,
                "authors": entry.fields.authors.value.clone().unwrap_or_default(),
                "year": entry.fields.year.value,
                "venue": entry.fields.venue.value,
                "doi": entry.fields.doi.value,
                "page": entry.page,
            })
        })
        .collect::<Vec<_>>();
    let artifacts = json!({
        "identity": {
            "title": paper.metadata.title,
            "authors": paper.metadata.authors.iter().map(|name| json!({"name": name})).collect::<Vec<_>>(),
        },
        "abstract": {"text": abstract_result.text, "page": abstract_result.start_page},
        "objects": objects,
        "references": references,
        "citations": bibliography.mentions.iter().map(|mention| json!({"page": mention.page})).collect::<Vec<_>>(),
    });
    let record = json!({
        "record_version": 2,
        "pdf": pdf,
        "model": "deterministic",
        "pages": paper.pages.len(),
        "abstract_status": format!("{:?}", abstract_result.status),
        "totals": {"cost_usd": 0.0, "wall_s": started.elapsed().as_secs_f64()},
        "calls": Value::Array(Vec::new()),
        "failures": Value::Array(Vec::new()),
        "pass1": artifacts,
        "artifacts": artifacts,
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, serde_json::to_vec_pretty(&record)?)?;
    Ok(())
}
