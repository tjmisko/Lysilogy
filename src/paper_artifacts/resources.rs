//! Basic resource extraction: the only deterministic stage of the agent path.
//!
//! A PDF becomes page-marked text and one colour PNG per page, built from the
//! PDF alone so the model's input matches what the product has at runtime.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    Error, Result,
    domain::{ExtractedPage, ExtractedPaper, PaperMetadata},
    extract::PdfExtractor,
    source_index::{BuildPriority, bounded_command, extraction_command},
    store::write_atomic,
};

/// The default render resolution; equations stay legible at this size.
pub const DEFAULT_DPI: u32 = 120;

const DPI_MINIMUM: u32 = 36;
const DPI_MAXIMUM: u32 = 300;

/// A paper's page text and page images.
#[derive(Debug)]
pub struct PageResources {
    pub paper: ExtractedPaper,
    /// One image per page, in page order; `images[i]` is page `i + 1`.
    pub images: Vec<PathBuf>,
    pub dpi: u32,
}

#[derive(Serialize)]
struct Manifest<'a> {
    pdf: &'a Path,
    dpi: u32,
    pages: Vec<ManifestPage<'a>>,
}

#[derive(Serialize)]
struct ManifestPage<'a> {
    number: u32,
    text_chars: usize,
    image: &'a Path,
}

/// Extract text and render every page of `pdf` into `directory`.
///
/// Writes `source.md` (page-marked text), `page-NNN.png`, and `resources.json`.
///
/// # Errors
/// Fails for a DPI outside 36–300, a PDF with no text layer, or when
/// `pdftotext`/`pdftoppm` are missing or fail.
pub async fn build(
    extractor: &PdfExtractor,
    pdf: &Path,
    directory: &Path,
    dpi: u32,
) -> Result<PageResources> {
    if !(DPI_MINIMUM..=DPI_MAXIMUM).contains(&dpi) {
        return Err(Error::InvalidRequest(format!(
            "page render DPI must be {DPI_MINIMUM}–{DPI_MAXIMUM}, not {dpi}"
        )));
    }
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| Error::io(directory, error))?;
    let mut paper = extractor
        .extract_allowing_empty(pdf, &PaperMetadata::default())
        .await?;
    let mut images = Vec::with_capacity(paper.pages.len());
    for page in &paper.pages {
        images.push(render_page(pdf, page.number, dpi, directory).await?);
    }
    for (page, image) in paper.pages.iter_mut().zip(&images) {
        if page.text.trim().is_empty()
            && let Ok(text) = ocr_page(image).await
        {
            page.text = text;
        }
    }
    write_atomic(
        &directory.join("source.md"),
        page_marked_text(&paper.pages).as_bytes(),
    )
    .await?;
    let manifest = Manifest {
        pdf,
        dpi,
        pages: paper
            .pages
            .iter()
            .zip(&images)
            .map(|(page, image)| ManifestPage {
                number: page.number,
                text_chars: page.text.chars().count(),
                image,
            })
            .collect(),
    };
    write_atomic(
        &directory.join("resources.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )
    .await?;
    Ok(PageResources { paper, images, dpi })
}

const MAX_OCR_BYTES: usize = 4 * 1024 * 1024;

/// Text of a rendered page, via tesseract; an error leaves the page empty.
async fn ocr_page(image: &Path) -> Result<String> {
    let mut command = extraction_command("tesseract", BuildPriority::Background);
    command.arg(image).arg("stdout");
    let output = bounded_command(&mut command, "tesseract", MAX_OCR_BYTES).await?;
    Ok(String::from_utf8_lossy(&output).trim().to_owned())
}

/// Render one page of `pdf` to `directory/page-NNN.png`.
///
/// # Errors
/// Fails when `pdftoppm` is missing, the page does not exist, or no image is written.
pub async fn render_page(pdf: &Path, page: u32, dpi: u32, directory: &Path) -> Result<PathBuf> {
    let prefix = directory.join(format!("page-{page:03}"));
    let mut render = extraction_command("pdftoppm", BuildPriority::Interactive);
    render
        .args(["-f", &page.to_string(), "-l", &page.to_string()])
        .args(["-singlefile", "-png", "-r", &dpi.to_string()])
        .arg(pdf)
        .arg(&prefix);
    bounded_command(&mut render, "pdftoppm", 1024).await?;
    let image = prefix.with_extension("png");
    // pdftoppm exits zero for a page past the end and writes nothing.
    if !tokio::fs::try_exists(&image).await.unwrap_or(false) {
        return Err(Error::InvalidRequest(format!(
            "pdftoppm wrote no image for page {page} of {}",
            pdf.display()
        )));
    }
    Ok(image)
}

/// The page text as one document with an explicit marker before each page.
#[must_use]
pub fn page_marked_text(pages: &[ExtractedPage]) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for page in pages {
        // Writing to a String cannot fail.
        let _ = write!(text, "<<page {}>>\n{}\n\n", page.number, page.text.trim());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    async fn poppler_available() -> bool {
        for program in ["pdftotext", "pdftoppm", "pdfinfo"] {
            if tokio::process::Command::new(program)
                .arg("-v")
                .output()
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }

    fn page(number: u32, text: &str) -> ExtractedPage {
        ExtractedPage {
            number,
            text: text.to_owned(),
        }
    }

    #[test]
    fn should_mark_every_page_in_order_when_joining_page_text() {
        let text = page_marked_text(&[page(1, "  First page.\n"), page(2, "Second.")]);
        assert_eq!(text, "<<page 1>>\nFirst page.\n\n<<page 2>>\nSecond.\n\n");
    }

    #[test]
    fn should_keep_the_marker_when_a_page_has_no_text() {
        let text = page_marked_text(&[page(1, ""), page(2, "Body")]);
        assert!(text.starts_with("<<page 1>>\n\n\n<<page 2>>"), "{text:?}");
    }

    #[tokio::test]
    async fn should_write_one_png_and_marked_text_per_page_when_given_a_pdf() {
        if !poppler_available().await {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let resources = build(
            &PdfExtractor::default(),
            &fixture("paragraph-continuation.pdf"),
            directory.path(),
            DEFAULT_DPI,
        )
        .await
        .expect("resources");
        assert_eq!(resources.paper.pages.len(), 2);
        assert_eq!(resources.images.len(), 2);
        for (index, image) in resources.images.iter().enumerate() {
            assert!(image.ends_with(format!("page-{:03}.png", index + 1)));
            let bytes = std::fs::read(image).expect("image");
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        }
        let source = std::fs::read_to_string(directory.path().join("source.md")).expect("source");
        assert!(source.contains("<<page 1>>") && source.contains("<<page 2>>"));
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("resources.json")).expect("manifest"),
        )
        .expect("manifest json");
        assert_eq!(manifest["dpi"], DEFAULT_DPI);
        assert_eq!(manifest["pages"].as_array().map(Vec::len), Some(2));
    }

    #[tokio::test]
    async fn should_fail_when_the_page_is_past_the_end() {
        if !poppler_available().await {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let error = render_page(&fixture("cursor-objects.pdf"), 9, 72, directory.path())
            .await
            .expect_err("no page 9");
        assert!(
            matches!(
                error,
                Error::InvalidRequest(_) | Error::CommandFailed { .. }
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn should_fail_without_panicking_when_the_pdf_is_malformed() {
        if !poppler_available().await {
            return;
        }
        let directory = tempfile::tempdir().expect("temporary directory");
        let pdf = directory.path().join("broken.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\nnot really a pdf").expect("write");
        let result = build(
            &PdfExtractor::default(),
            &pdf,
            &directory.path().join("out"),
            DEFAULT_DPI,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn should_reject_a_dpi_outside_the_supported_range() {
        let directory = tempfile::tempdir().expect("temporary directory");
        for dpi in [0, 35, 301, 10_000] {
            let error = build(
                &PdfExtractor::default(),
                &fixture("cursor-objects.pdf"),
                directory.path(),
                dpi,
            )
            .await
            .expect_err("bad dpi");
            assert!(matches!(error, Error::InvalidRequest(_)), "{dpi}: {error}");
        }
    }
}
