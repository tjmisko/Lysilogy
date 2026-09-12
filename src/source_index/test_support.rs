use std::{fmt::Write, path::Path};

use super::{CachedIndex, SourceStamp, assemble};

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
