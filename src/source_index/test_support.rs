use std::{fmt::Write, path::Path};

use super::{
    CachedIndex, Paragraph, Provenance, ReadingIndex, ReadingPage, ReadingToken, SourceStamp,
    TextRange, assemble,
};
use crate::domain::TextRect;

pub fn native_pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 50 700 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
    ];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        writeln!(pdf, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
    }
    let xref = pdf.len();
    writeln!(pdf, "xref\n0 {}\n0000000000 65535 f ", offsets.len()).unwrap();
    for offset in offsets.iter().skip(1) {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    writeln!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
        offsets.len()
    )
    .unwrap();
    pdf.into_bytes()
}

pub fn has_pdftotext() -> bool {
    std::process::Command::new("pdftotext")
        .arg("-v")
        .output()
        .is_ok_and(|output| output.status.success())
}

pub async fn cache_fixture(source: &Path, directory: &Path) {
    let metadata = tokio::fs::metadata(source).await.unwrap();
    let stamp = SourceStamp {
        bytes: metadata.len(),
        modified_nanos: metadata
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    };
    let mut index = assemble(&[]);
    index.text = "A permanently cached source index.".into();
    tokio::fs::create_dir_all(directory).await.unwrap();
    tokio::fs::write(
        directory.join("reading-index.json"),
        serde_json::to_vec(&CachedIndex {
            source: stamp,
            generation: "2020-01-01".into(),
            index,
        })
        .unwrap(),
    )
    .await
    .unwrap();
}

/// Like [`cache_fixture`], with the given `(page number, text)` pages laid out
/// the way `assemble` does: paragraphs separated by a blank line, page bounds
/// in UTF-16 units.
pub async fn cache_fixture_with_pages(source: &Path, directory: &Path, pages: &[(u32, &str)]) {
    cache_fixture(source, directory).await;
    let path = directory.join("reading-index.json");
    let mut cached: CachedIndex =
        serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
    let mut text = String::new();
    let mut offset = 0;
    for (number, page_text) in pages {
        if !text.is_empty() {
            text.push_str("\n\n");
            offset += 2;
        }
        let start = offset;
        text.push_str(page_text);
        offset += page_text.encode_utf16().count();
        cached.index.pages.push(ReadingPage {
            number: *number,
            width: 612.0,
            height: 792.0,
            start,
            end: offset,
            provenance: Provenance::Native,
            confidence: None,
        });
    }
    cached.index.text = text;
    tokio::fs::write(&path, serde_json::to_vec(&cached).unwrap())
        .await
        .unwrap();
}

/// An empty index, as produced for a document with no pages.
pub fn reading_index_fixture_empty() -> ReadingIndex {
    assemble(&[])
}

/// A small hand-built index with an astral character, two pages, a heading,
/// a paragraph with split member spans, and two sentences.
pub fn reading_index_fixture() -> ReadingIndex {
    let mut index = assemble(&[]);
    // The emoji takes two UTF-16 units, so the next word starts at 3.
    index.text = "😀 ab [1] cd.\n\nReferences x".into();
    let rect = |x: f32| TextRect {
        x_min: x,
        y_min: 10.0,
        x_max: x + 5.0,
        y_max: 22.0,
    };
    let token = |start, end, page, text: &str, x| ReadingToken {
        start,
        end,
        page,
        rects: vec![rect(x)],
        text: text.into(),
        provenance: Provenance::Native,
    };
    index.tokens = vec![
        token(0, 2, 1, "😀", 1.0),
        token(3, 5, 1, "ab", 7.0),
        token(6, 9, 1, "[1]", 13.0),
    ];
    index.pages = vec![page(1, 0, 13), page(2, 15, 25)];
    index.objects.paragraph = vec![
        Paragraph {
            start: 15,
            end: 25,
            kind: "heading".into(),
            spans: vec![],
        },
        Paragraph {
            start: 0,
            end: 25,
            kind: "body".into(),
            spans: vec![
                TextRange { start: 0, end: 13 },
                TextRange { start: 20, end: 25 },
            ],
        },
    ];
    index.objects.sentence = vec![
        TextRange { start: 0, end: 13 },
        TextRange { start: 15, end: 25 },
    ];
    index
}

fn page(number: u32, start: usize, end: usize) -> ReadingPage {
    ReadingPage {
        number,
        width: 612.0,
        height: 792.0,
        start,
        end,
        provenance: Provenance::Native,
        confidence: None,
    }
}
