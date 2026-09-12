use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::{Provenance, SourcePage, SourceStamp, SourceWord, bounded_command};
use crate::{Error, Result, domain::TextRect, store::write_atomic};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize, Serialize)]
struct CachedOcr {
    source: SourceStamp,
    tsv: String,
}

struct TemporaryImage(std::path::PathBuf);
impl Drop for TemporaryImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) async fn cached_page(
    directory: &Path,
    page: &SourcePage,
    stamp: &SourceStamp,
) -> Option<SourcePage> {
    let cache_path = directory.join(format!("reading-ocr-{}.json", page.number));
    if let Ok(bytes) = tokio::fs::read(&cache_path).await
        && let Ok(cache) = serde_json::from_slice::<CachedOcr>(&bytes)
        && &cache.source == stamp
    {
        return parse_tsv(&cache.tsv, page).ok();
    }
    None
}

pub(super) async fn page(
    source: &Path,
    directory: &Path,
    page: &SourcePage,
    stamp: &SourceStamp,
) -> Result<SourcePage> {
    let cache_path = directory.join(format!("reading-ocr-{}.json", page.number));
    let prefix = directory.join(format!(
        "reading-ocr-temp-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let image = TemporaryImage(prefix.with_extension("pgm"));
    let mut render = Command::new("pdftoppm");
    render
        .args([
            "-f",
            &page.number.to_string(),
            "-l",
            &page.number.to_string(),
            "-singlefile",
            "-scale-to",
            "1800",
            "-gray",
        ])
        .arg(source)
        .arg(&prefix);
    bounded_command(&mut render, "pdftoppm", 1024).await?;
    let mut command = Command::new("tesseract");
    command
        .arg(&image.0)
        .args(["stdout", "-l", "eng", "--psm", "3", "tsv"])
        .env("OMP_THREAD_LIMIT", "1");
    let bytes = bounded_command(&mut command, "tesseract", 8 * 1024 * 1024).await?;
    let tsv = String::from_utf8_lossy(&bytes).into_owned();
    let result = parse_tsv(&tsv, page)?;
    write_atomic(
        &cache_path,
        &serde_json::to_vec(&CachedOcr {
            source: stamp.clone(),
            tsv,
        })?,
    )
    .await?;
    Ok(result)
}

pub(super) fn parse_tsv(tsv: &str, original: &SourcePage) -> Result<SourcePage> {
    let mut width = 0.0_f32;
    let mut height = 0.0_f32;
    let mut words = Vec::new();
    let mut confidence = 0.0_f32;
    let mut confidence_count = 0.0_f32;
    let mut old_line = String::new();
    let mut line = 0_u32;
    for row in tsv.lines().skip(1) {
        let columns = row.splitn(12, '\t').collect::<Vec<_>>();
        if columns.len() < 12 {
            continue;
        }
        let value = |index: usize| {
            columns[index]
                .parse::<f32>()
                .ok()
                .filter(|number| number.is_finite())
        };
        if columns[0] == "1" {
            width = value(8).unwrap_or(0.0);
            height = value(9).unwrap_or(0.0);
            continue;
        }
        if columns[0] != "5" || width <= 0.0 || height <= 0.0 || columns[11].trim().is_empty() {
            continue;
        }
        let Some(score) = value(10) else {
            continue;
        };
        // Very low OCR scores are excluded, not presented as reliable source words.
        if score < 25.0 {
            continue;
        }
        let (Some(x), Some(y), Some(w), Some(h)) = (value(6), value(7), value(8), value(9)) else {
            continue;
        };
        let identity = columns[2..5].join(":");
        if old_line != identity {
            line += 1;
            old_line = identity;
        }
        let block = columns[2]
            .parse::<u32>()
            .unwrap_or(0)
            .saturating_mul(10000)
            .saturating_add(columns[3].parse::<u32>().unwrap_or(0));
        words.push(SourceWord {
            text: columns[11].trim().to_owned(),
            line,
            block,
            rect: TextRect {
                x_min: (x / width * original.width).clamp(0.0, original.width),
                y_min: (y / height * original.height).clamp(0.0, original.height),
                x_max: ((x + w) / width * original.width).clamp(0.0, original.width),
                y_max: ((y + h) / height * original.height).clamp(0.0, original.height),
            },
        });
        confidence += score / 100.0;
        confidence_count += 1.0;
    }
    if width <= 0.0 || height <= 0.0 {
        return Err(Error::InvalidLayout(
            "OCR did not return page dimensions".into(),
        ));
    }
    Ok(SourcePage {
        number: original.number,
        width: original.width,
        height: original.height,
        words,
        provenance: Provenance::Ocr,
        confidence: (confidence_count > 0.0).then(|| confidence / confidence_count),
    })
}
