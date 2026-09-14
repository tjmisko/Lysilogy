//! Optional, bounded PDF image placements. Native text and anchors never change.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{io::AsyncReadExt, process::Command, sync::Semaphore};

use super::{IndexDocument, ReadingPage};
use crate::{Error, Result, domain::TextRect};

mod masks;
pub(super) mod vectors;
pub use masks::{MaskPageEvidence, MaskRuntime};
pub use vectors::VectorPageEvidence;

pub const VERSION: u16 = 5;
const PAGE_BYTES: usize = 16 * 1024 * 1024;
const TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAGES: usize = 64;
static GRAPHICS_JOB: Semaphore = Semaphore::const_new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphicsEvidence {
    pub version: u16,
    pub native_generation: String,
    pub pdf_sha256: String,
    pub tool_sha256: Option<String>,
    pub tool_path: Option<PathBuf>,
    #[serde(default)]
    pub mask_runtime: Option<MaskRuntime>,
    pub cache_key: String,
    pub generation: String,
    pub pages: Vec<PageGraphics>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageGraphics {
    pub page: u32,
    pub status: String,
    pub trace_sha256: Option<String>,
    pub images: Vec<TextRect>,
    pub unsupported_images: usize,
    #[serde(default)]
    pub mask: Option<MaskPageEvidence>,
    #[serde(default)]
    pub vectors: Option<VectorPageEvidence>,
}

pub struct PreparedGraphics {
    evidence: GraphicsEvidence,
    program: Option<PathBuf>,
}

pub struct GraphicsDocument {
    pub evidence: GraphicsEvidence,
    /// Optional external experiment evidence; never part of a production object JSON.
    pub traces: Vec<(u32, Vec<u8>)>,
    pub mask_receipts: Vec<(u32, Vec<u8>)>,
    pub vector_rasters: Vec<(u32, Vec<u8>)>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

async fn file_hash(path: &Path, limit: u64) -> Result<String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| Error::io(path, e))?;
    if file.metadata().await.map_err(|e| Error::io(path, e))?.len() > limit {
        return Err(Error::InvalidRequest(
            "Graphics input exceeds its byte bound".into(),
        ));
    }
    let mut file = file.take(limit + 1);
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 65536];
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|e| Error::io(path, e))?;
        if count == 0 {
            break;
        }
        total += u64::try_from(count).unwrap_or(u64::MAX);
        if total > limit {
            return Err(Error::InvalidRequest(
                "Graphics input grew beyond its bound".into(),
            ));
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn program_path() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|p| p.join("mutool"))
        .find(|path| path.is_file())
        .and_then(|path| path.canonicalize().ok())
}

impl PreparedGraphics {
    #[must_use]
    pub fn cache_key(&self) -> &str {
        &self.evidence.cache_key
    }
    async fn verify_unchanged(&self, source: &Path) -> Result<()> {
        if file_hash(source, 512 * 1024 * 1024).await? != self.evidence.pdf_sha256 {
            return Err(Error::InvalidRequest(
                "PDF changed while deriving graphics".into(),
            ));
        }
        if let Some(program) = &self.program
            && Some(file_hash(program, 128 * 1024 * 1024).await?) != self.evidence.tool_sha256
        {
            return Err(Error::InvalidRequest(
                "Graphics tool changed during derivation".into(),
            ));
        }
        if let Some(runtime) = &self.evidence.mask_runtime
            && !runtime.unchanged().await
        {
            return Err(Error::InvalidRequest(
                "Mask resource wrapper changed during derivation".into(),
            ));
        }
        Ok(())
    }
}

pub async fn prepare(source: &Path, document: &IndexDocument) -> Result<PreparedGraphics> {
    let pdf_sha256 = file_hash(source, 512 * 1024 * 1024).await?;
    let program = program_path();
    let tool_sha256 = match &program {
        Some(path) => file_hash(path, 128 * 1024 * 1024).await.ok(),
        None => None,
    };
    let program = program.filter(|_| tool_sha256.is_some());
    let mask_runtime = MaskRuntime::prepare().await;
    let cache_key = digest(&serde_json::to_vec(&(
        VERSION,
        &document.etag,
        &pdf_sha256,
        &tool_sha256,
        mask_runtime.as_ref().map(MaskRuntime::key),
    ))?);
    let tool_path = program.clone();
    Ok(PreparedGraphics {
        program,
        evidence: GraphicsEvidence {
            version: VERSION,
            native_generation: document.etag.clone(),
            pdf_sha256,
            tool_sha256,
            tool_path,
            mask_runtime,
            cache_key,
            generation: String::new(),
            pages: vec![],
        },
    })
}

pub async fn collect(
    source: &Path,
    document: &IndexDocument,
    pages: &[u32],
    mut prepared: PreparedGraphics,
) -> Result<GraphicsDocument> {
    if pages.windows(2).any(|pair| pair[0] >= pair[1])
        || pages.iter().any(|page| {
            document
                .index
                .pages
                .iter()
                .filter(|p| p.number == *page)
                .count()
                != 1
        })
    {
        return Err(Error::InvalidRequest(
            "Invalid graphics page inventory".into(),
        ));
    }
    let _permit = GRAPHICS_JOB
        .acquire()
        .await
        .map_err(|_| Error::Task("Graphics worker unavailable".into()))?;
    let mut budget = MaskBudget::new();
    let mut traces = Vec::new();
    for (position, page) in pages.iter().enumerate() {
        let mut result = PageGraphics {
            page: *page,
            status: "unavailable".into(),
            trace_sha256: None,
            images: vec![],
            unsupported_images: 0,
            mask: None,
            vectors: None,
        };
        if position >= MAX_PAGES
            || budget.total >= TOTAL_BYTES
            || budget.started.elapsed() > Duration::from_secs(30)
        {
            result.status = "resource_limit".into();
        } else if let Some(program) = &prepared.program {
            let mut command = trace_command(program, source, *page);
            let output = tokio::time::timeout(
                Duration::from_secs(5),
                super::bounded_command(
                    &mut command,
                    "mutool",
                    PAGE_BYTES.min(TOTAL_BYTES - budget.total),
                ),
            )
            .await;
            if let Ok(Ok(raw)) = output {
                budget.total += raw.len();
                result.trace_sha256 = Some(digest(&raw));
                if let Some(native) = document.index.pages.iter().find(|p| p.number == *page)
                    && let Ok((images, unsupported)) = parse_trace(&raw, native)
                {
                    result.images = images;
                    result.unsupported_images = unsupported;
                    if unsupported > 0 && raw.windows(15).any(|part| part == b"clip_image_mask") {
                        supplement_mask(
                            prepared.evidence.mask_runtime.as_ref(),
                            program,
                            source,
                            native,
                            &raw,
                            &mut result,
                            &mut budget,
                        )
                        .await;
                    }
                    result.status = if result.unsupported_images == 0 {
                        "complete"
                    } else {
                        "partial"
                    }
                    .into();
                } else {
                    result.status = "unsupported_trace".into();
                }
                traces.push((*page, raw));
            } else {
                result.status = "tool_failed".into();
            }
        } else {
            result.status = "tool_unavailable".into();
        }
        prepared.evidence.pages.push(result);
    }
    supplement_vector_pages(source, document, &mut prepared, &traces, &mut budget).await;
    prepared.verify_unchanged(source).await?;
    prepared.evidence.generation = digest(&prepared.evidence.basis_json()?);
    Ok(GraphicsDocument {
        evidence: prepared.evidence,
        traces,
        mask_receipts: budget.receipts,
        vector_rasters: budget.rasters,
    })
}

async fn supplement_vector_pages(
    source: &Path,
    document: &IndexDocument,
    prepared: &mut PreparedGraphics,
    traces: &[(u32, Vec<u8>)],
    budget: &mut MaskBudget,
) {
    // Complete all established image/mask work before spending the remaining
    // paper budget on optional vectors. New rasters cannot starve later images.
    if let Some(program) = &prepared.program {
        for result in &mut prepared.evidence.pages {
            if let Some((_, raw)) = traces.iter().find(|(page, _)| *page == result.page)
                && let Some(native) = document
                    .index
                    .pages
                    .iter()
                    .find(|p| p.number == result.page)
            {
                supplement_vectors(
                    prepared.evidence.mask_runtime.as_ref(),
                    program,
                    source,
                    document,
                    native,
                    raw,
                    result,
                    budget,
                )
                .await;
            }
        }
    }
}

struct MaskBudget {
    started: Instant,
    total: usize,
    receipts: Vec<(u32, Vec<u8>)>,
    rasters: Vec<(u32, Vec<u8>)>,
}

impl MaskBudget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            total: 0,
            receipts: Vec::new(),
            rasters: Vec::new(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn supplement_vectors(
    runtime: Option<&MaskRuntime>,
    program: &Path,
    source: &Path,
    document: &IndexDocument,
    native: &ReadingPage,
    raw: &[u8],
    result: &mut PageGraphics,
    budget: &mut MaskBudget,
) {
    let mut evidence = VectorPageEvidence::unavailable("unsupported_renderer_trace");
    if native.provenance != super::Provenance::Native
        || document
            .index
            .gaps
            .iter()
            .any(|gap| gap.page == native.number)
    {
        evidence.status = "native_gap_or_unavailable".into();
    } else if !parse_renderer_trace(raw, native).unwrap_or(false) {
        // Strict image parsing and its unsupported states remain unchanged.
        // This separate finite mode only permits original rendering of curved,
        // no-image pages under the explicitly tested compositing boundary.
    } else if vectors::dimensions(native).is_none() {
        evidence.status = "pixel_limit".into();
    } else if let Some(runtime) = runtime {
        let deadline = budget.started + Duration::from_secs(30);
        let remaining = deadline.saturating_duration_since(Instant::now());
        let limit = PAGE_BYTES.min(TOTAL_BYTES.saturating_sub(budget.total));
        if remaining.is_zero() || limit == 0 {
            evidence.status = "resource_limit".into();
        } else {
            let output = tokio::time::timeout(
                remaining.min(Duration::from_secs(5)),
                vectors::render(runtime, program, source, native.number, limit),
            )
            .await;
            match output {
                Ok(Ok(pixels)) => {
                    budget.total += pixels.len();
                    evidence =
                        vectors::decode(&pixels, native, deadline).unwrap_or_else(|reason| {
                            let mut failed = VectorPageEvidence::unavailable(reason);
                            failed.raster_sha256 = Some(digest(&pixels));
                            failed
                        });
                    budget.rasters.push((native.number, pixels));
                }
                Ok(Err(error)) => {
                    evidence.status = "tool_failed".into();
                    evidence.diagnostic = Some(error.to_string().chars().take(1024).collect());
                }
                _ => evidence.status = "resource_limit".into(),
            }
        }
    } else {
        evidence.status = "resource_wrapper_unavailable".into();
    }
    result.vectors = Some(evidence);
}

async fn supplement_mask(
    runtime: Option<&MaskRuntime>,
    program: &Path,
    source: &Path,
    native: &ReadingPage,
    raw: &[u8],
    result: &mut PageGraphics,
    budget: &mut MaskBudget,
) {
    let mut mask = MaskPageEvidence {
        status: "tool_unavailable".into(),
        receipt_sha256: None,
        supported_images: 0,
        empty_images: 0,
    };
    if budget.total >= TOTAL_BYTES || budget.started.elapsed() >= Duration::from_secs(30) {
        mask.status = "resource_limit".into();
    } else if let Some(runtime) = runtime {
        let remaining = Duration::from_secs(30).saturating_sub(budget.started.elapsed());
        let output = tokio::time::timeout(
            Duration::from_secs(10).min(remaining),
            masks::collect(
                runtime,
                program,
                source,
                native.number,
                PAGE_BYTES.min(TOTAL_BYTES - budget.total),
            ),
        )
        .await;
        if let Ok(Ok(mask_raw)) = output {
            budget.total += mask_raw.len();
            mask.receipt_sha256 = Some(digest(&mask_raw));
            mask.status = "unsupported_receipt".into();
            if let Ok(receipt) = masks::Receipt::parse(&mask_raw, native)
                && let Ok(parsed) = parse_trace_with_masks(raw, native, Some(&receipt))
            {
                mask.status = "evaluated".into();
                mask.supported_images = parsed.mask_supported;
                mask.empty_images = parsed.mask_empty;
                result.images = parsed.images;
                result.unsupported_images = parsed.unsupported;
            }
            budget.receipts.push((native.number, mask_raw));
        } else {
            mask.status = "tool_failed".into();
        }
    }
    result.mask = Some(mask);
}

fn trace_command(program: &Path, source: &Path, page: u32) -> Command {
    let mut command = Command::new(program);
    command
        .args([
            "draw",
            "-F",
            "trace",
            "-r",
            "72",
            "-N",
            "-L",
            "-m",
            "134217728",
            "-q",
            "-o",
            "-",
        ])
        .arg(source)
        .arg(page.to_string());
    command
}

struct Tag<'a> {
    name: &'a str,
    attrs: BTreeMap<&'a str, &'a str>,
    close: bool,
    empty: bool,
}

fn invalid() -> Error {
    Error::InvalidLayout("Unsupported or ambiguous graphics trace".into())
}

fn tag(raw: &str) -> Result<Tag<'_>> {
    let close = raw.starts_with('/');
    let empty = raw.ends_with('/');
    let raw = raw.strip_prefix('/').unwrap_or(raw);
    let raw = raw.strip_suffix('/').unwrap_or(raw).trim();
    let end = raw.find(char::is_whitespace).unwrap_or(raw.len());
    let name = &raw[..end];
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(invalid());
    }
    let mut rest = raw[end..].trim();
    let mut attrs = BTreeMap::new();
    while !rest.is_empty() {
        let split = rest.find('=').ok_or_else(invalid)?;
        let key = rest[..split].trim();
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(invalid());
        }
        rest = rest[split + 1..].trim_start();
        let quote = rest.chars().next().ok_or_else(invalid)?;
        if quote != '\'' && quote != '"' {
            return Err(invalid());
        }
        rest = &rest[1..];
        let end = rest.find(quote).ok_or_else(invalid)?;
        if attrs.insert(key, &rest[..end]).is_some() {
            return Err(invalid());
        }
        let tail = &rest[end + 1..];
        if tail.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
            return Err(invalid());
        }
        rest = tail.trim_start();
    }
    if close && (empty || !attrs.is_empty()) {
        return Err(invalid());
    }
    Ok(Tag {
        name,
        attrs,
        close,
        empty,
    })
}

fn numbers<const N: usize>(value: &str) -> Result<[f32; N]> {
    let values = value
        .split_whitespace()
        .map(str::parse::<f32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    if values.len() != N || values.iter().any(|x| !x.is_finite()) {
        return Err(invalid());
    }
    values.try_into().map_err(|_| invalid())
}

fn image_rect(tag: &Tag<'_>, page: &ReadingPage) -> Result<TextRect> {
    if !tag.empty || tag.attrs.get("alpha") != Some(&"1") {
        return Err(invalid());
    }
    let [xx, yx, xy, yy, tx, ty] = numbers::<6>(tag.attrs.get("transform").ok_or_else(invalid)?)?;
    if !((yx == 0.0 && xy == 0.0 && xx != 0.0 && yy != 0.0)
        || (xx == 0.0 && yy == 0.0 && yx != 0.0 && xy != 0.0))
    {
        return Err(invalid());
    }
    for key in ["width", "height"] {
        if tag
            .attrs
            .get(key)
            .ok_or_else(invalid)?
            .parse::<u32>()
            .map_err(|_| invalid())?
            == 0
        {
            return Err(invalid());
        }
    }
    let points = [
        (tx, ty),
        (xx + tx, yx + ty),
        (xy + tx, yy + ty),
        (xx + xy + tx, yx + yy + ty),
    ];
    let rect = TextRect {
        x_min: points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min),
        y_min: points.iter().map(|p| p.1).fold(f32::INFINITY, f32::min),
        x_max: points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max),
        y_max: points.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max),
    };
    if !rect.x_max.is_finite()
        || !rect.y_max.is_finite()
        || rect.x_min < 0.0
        || rect.y_min < 0.0
        || rect.x_max > page.width
        || rect.y_max > page.height
        || rect.x_min >= rect.x_max
        || rect.y_min >= rect.y_max
    {
        return Err(invalid());
    }
    Ok(rect)
}

fn next_tag(rest: &str) -> Result<(&str, &str)> {
    let mut quote = None;
    let mut end = None;
    for (position, ch) in rest.char_indices() {
        if position > 65536 {
            return Err(invalid());
        }
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(q), _) if q == ch => quote = None,
            (None, '>') => {
                end = Some(position);
                break;
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(invalid)?;
    let raw_tag = &rest[..end];
    Ok((raw_tag, &rest[end + 1..]))
}

fn known_command(name: &str) -> bool {
    matches!(
        name,
        "document"
            | "page"
            | "fill_image"
            | "fill_text"
            | "stroke_text"
            | "ignore_text"
            | "span"
            | "g"
            | "fill_path"
            | "stroke_path"
            | "moveto"
            | "lineto"
            | "curveto"
            | "closepath"
            | "rect"
            | "set_default_colorspaces"
            | "clip_path"
            | "clip_stroke_path"
            | "clip_text"
            | "clip_stroke_text"
            | "clip_image_mask"
            | "pop_clip"
    )
}

fn validate_page(value: &Tag<'_>, page: &ReadingPage) -> Result<()> {
    if value
        .attrs
        .get("number")
        .ok_or_else(invalid)?
        .parse::<u32>()
        .map_err(|_| invalid())?
        != page.number
    {
        return Err(invalid());
    }
    let [x, y, w, h] = numbers::<4>(value.attrs.get("mediabox").ok_or_else(invalid)?)?;
    if x != 0.0 || y != 0.0 || (w - page.width).abs() > 0.01 || (h - page.height).abs() > 0.01 {
        return Err(invalid());
    }
    Ok(())
}

fn record_image(
    value: &Tag<'_>,
    page: &ReadingPage,
    visible: bool,
    clips: &[Clip],
    images: &mut Vec<TextRect>,
    unsupported: &mut usize,
) -> Result<()> {
    if images.len() + *unsupported >= 256 {
        return Err(invalid());
    }
    if visible && let Ok(mut rect) = image_rect(value, page) {
        for clip in clips {
            match clip {
                Clip::Unknown | Clip::Mask(_) => {
                    *unsupported += 1;
                    return Ok(());
                }
                Clip::Rectangle(bounds) => {
                    let Some(cropped) = intersection(rect, *bounds) else {
                        // A supported empty intersection paints no part of this image.
                        return Ok(());
                    };
                    rect = cropped;
                }
            }
        }
        images.push(rect);
    } else {
        *unsupported += 1;
    }
    Ok(())
}

enum Clip {
    Unknown,
    Mask(usize),
    Rectangle(TextRect),
}

fn intersection(a: TextRect, b: TextRect) -> Option<TextRect> {
    let rect = TextRect {
        x_min: a.x_min.max(b.x_min),
        y_min: a.y_min.max(b.y_min),
        x_max: a.x_max.min(b.x_max),
        y_max: a.y_max.min(b.y_max),
    };
    (rect.x_min < rect.x_max && rect.y_min < rect.y_max).then_some(rect)
}

// MuPDF's trace matrices already map path coordinates into page space. Only
// one explicitly closed, axis-aligned rectangular subpath is understood here;
// arbitrary path and image masks still withhold images until their pop_clip.
struct RectanglePath {
    transform: [f32; 6],
    points: Vec<(f32, f32)>,
    closed: bool,
}

impl RectanglePath {
    fn new(value: &Tag<'_>) -> Option<Self> {
        if !matches!(value.attrs.get("winding"), Some(&"nonzero" | &"eofill"))
            || value
                .attrs
                .keys()
                .any(|key| !matches!(*key, "winding" | "transform"))
        {
            return None;
        }
        Some(Self {
            transform: numbers::<6>(value.attrs.get("transform")?).ok()?,
            points: Vec::new(),
            closed: false,
        })
    }

    fn push(&mut self, value: &Tag<'_>) -> Option<()> {
        if !value.empty || self.closed {
            return None;
        }
        if value.name == "closepath" && value.attrs.is_empty() {
            self.closed = true;
            return Some(());
        }
        if (value.name != "moveto" || !self.points.is_empty())
            && (value.name != "lineto" || self.points.is_empty())
        {
            return None;
        }
        if self.points.len() >= 5 || value.attrs.len() != 2 {
            return None;
        }
        let [x] = numbers::<1>(value.attrs.get("x")?).ok()?;
        let [y] = numbers::<1>(value.attrs.get("y")?).ok()?;
        self.points.push((x, y));
        Some(())
    }

    fn finish(mut self) -> Option<TextRect> {
        let same = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0) == 0.0 && (a.1 - b.1) == 0.0;
        if self.points.len() == 5 && same(self.points[0], self.points[4]) {
            self.points.pop();
        }
        if !self.closed || self.points.len() != 4 {
            return None;
        }
        let [xx, yx, xy, yy, tx, ty] = self.transform;
        if !((yx == 0.0 && xy == 0.0 && xx != 0.0 && yy != 0.0)
            || (xx == 0.0 && yy == 0.0 && yx != 0.0 && xy != 0.0))
        {
            return None;
        }
        for position in 0..4 {
            let a = self.points[position];
            let b = self.points[(position + 1) % 4];
            // Exactly one coordinate changes on each edge; crossed/degenerate
            // quadrilaterals are not rectangular clipping paths.
            if ((a.0 - b.0) == 0.0) == ((a.1 - b.1) == 0.0)
                || same(a, self.points[(position + 2) % 4])
            {
                return None;
            }
        }
        let points = self
            .points
            .iter()
            .map(|&(x, y)| {
                (
                    xx.mul_add(x, xy.mul_add(y, tx)),
                    yx.mul_add(x, yy.mul_add(y, ty)),
                )
            })
            .collect::<Vec<_>>();
        if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return None;
        }
        let rect = TextRect {
            x_min: points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min),
            y_min: points.iter().map(|p| p.1).fold(f32::INFINITY, f32::min),
            x_max: points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max),
            y_max: points.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max),
        };
        (rect.x_min < rect.x_max && rect.y_min < rect.y_max).then_some(rect)
    }
}

fn drawing_scope(stack: &[&str]) -> bool {
    stack.starts_with(&["document", "page"]) && stack[2..].iter().all(|name| *name == "group")
}

fn supported_group(value: &Tag<'_>, page: &ReadingPage) -> bool {
    value.attrs.len() == 5
        && value.attrs.get("isolated") == Some(&"1")
        && value.attrs.get("knockout") == Some(&"0")
        && value.attrs.get("blendmode") == Some(&"Normal")
        && value.attrs.get("alpha") == Some(&"1")
        && value.attrs.get("bbox").is_some_and(|bbox| {
            numbers::<4>(bbox).is_ok_and(|[x0, y0, x1, y1]| {
                x0 == 0.0
                    && y0 == 0.0
                    && (x1 - page.width).abs() <= 0.01
                    && (y1 - page.height).abs() <= 0.01
            })
        })
}

fn renderer_group(value: &Tag<'_>) -> bool {
    value.attrs.len() == 5
        && matches!(value.attrs.get("isolated"), Some(&"0" | &"1"))
        && matches!(value.attrs.get("knockout"), Some(&"0" | &"1"))
        && value.attrs.get("blendmode") == Some(&"Normal")
        && value.attrs.get("alpha").is_some_and(|alpha| {
            numbers::<1>(alpha).is_ok_and(|[alpha]| (0.0..=1.0).contains(&alpha))
        })
        && value.attrs.get("bbox").is_some_and(|bbox| {
            numbers::<4>(bbox).is_ok_and(|[x0, y0, x1, y1]| x0 <= x1 && y0 <= y1)
        })
}

fn renderer_command(value: &Tag<'_>, stack: &[&str]) -> bool {
    if value.name == "set_default_colorspaces" {
        return value.empty
            && stack == ["document", "page"]
            && value.attrs.len() == 4
            && value.attrs.get("gray") == Some(&"DeviceGray")
            && value.attrs.get("rgb") == Some(&"DeviceRGB")
            && value.attrs.get("cmyk") == Some(&"DeviceCMYK")
            && value.attrs.get("oi") == Some(&"None");
    }
    if value.name == "clip_path" && value.empty {
        return false;
    }
    if matches!(
        value.name,
        "fill_image"
            | "clip_image_mask"
            | "clip_text"
            | "clip_stroke_text"
            | "clip_stroke_path"
            | "rect"
    ) || (!known_command(value.name) && !matches!(value.name, "group" | "metatext"))
    {
        return false;
    }
    if value.attrs.get("transform").is_some_and(|matrix| {
        !numbers::<6>(matrix).is_ok_and(|[a, b, c, d, _, _]| {
            let determinant = a.mul_add(d, -(b * c));
            determinant.is_finite() && determinant != 0.0
        })
    }) || value
        .attrs
        .get("alpha")
        .is_some_and(|alpha| !numbers::<1>(alpha).is_ok_and(|[alpha]| (0.0..=1.0).contains(&alpha)))
    {
        return false;
    }
    if matches!(value.name, "fill_path" | "stroke_path") && !drawing_scope(stack) {
        return false;
    }
    if matches!(value.name, "moveto" | "lineto" | "curveto" | "closepath") {
        if !matches!(
            stack.last(),
            Some(&"fill_path" | &"stroke_path" | &"clip_path" | &"clip_stroke_path")
        ) || !value.empty
        {
            return false;
        }
        let keys: &[&str] = match value.name {
            "moveto" | "lineto" => &["x", "y"],
            "curveto" => &["x1", "y1", "x2", "y2", "x3", "y3"],
            _ => &[],
        };
        return value.attrs.len() == keys.len()
            && keys.iter().all(|key| {
                value
                    .attrs
                    .get(key)
                    .is_some_and(|value| numbers::<1>(value).is_ok())
            });
    }
    true
}

/// Trace commands already carry page-space matrices. Unsupported drawing state
/// never becomes a claimed image rectangle; native text geometry is untouched.
#[derive(Default)]
struct TraceState<'a> {
    renderer: bool,
    renderer_page_command_seen: bool,
    curved_paint: bool,
    receipt: Option<&'a masks::Receipt>,
    operations: usize,
    mask_supported: usize,
    mask_empty: usize,
    stack: Vec<&'a str>,
    clips: Vec<Clip>,
    clip_floors: Vec<usize>,
    clip_path: Option<RectanglePath>,
    observed_images: usize,
    unsafe_state: bool,
    images: Vec<TextRect>,
    unsupported: usize,
}

impl<'a> TraceState<'a> {
    fn close(&mut self, name: &str) -> Result<()> {
        if self.stack.pop() != Some(name) {
            return Err(invalid());
        }
        let floor = self.clip_floors.pop().ok_or_else(invalid)?;
        if matches!(name, "group" | "page") && self.clips.len() != floor {
            return Err(invalid());
        }
        if name == "clip_path" {
            if let Some(rect) = self.clip_path.take().and_then(RectanglePath::finish) {
                *self.clips.last_mut().ok_or_else(invalid)? = Clip::Rectangle(rect);
            } else if self.renderer {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn observe(&mut self, value: &Tag<'a>, page: &ReadingPage) -> Result<()> {
        if self.renderer {
            if !renderer_command(value, &self.stack)
                || (value.name == "set_default_colorspaces" && self.renderer_page_command_seen)
            {
                return Err(invalid());
            }
            self.renderer_page_command_seen |= !matches!(value.name, "document" | "page");
        }
        if value.name == "curveto"
            && matches!(self.stack.last(), Some(&"fill_path" | &"stroke_path"))
        {
            self.curved_paint = true;
        }
        if value.name == "group" {
            if !drawing_scope(&self.stack) || value.empty {
                return Err(invalid());
            }
            self.unsafe_state |= if self.renderer {
                !renderer_group(value)
            } else {
                !supported_group(value, page)
            };
        } else if value.name == "metatext" {
            if !drawing_scope(&self.stack) || value.empty {
                return Err(invalid());
            }
            self.unsafe_state |= value.attrs.len() != 2
                || value.attrs.get("type") != Some(&"actualtext")
                || !value.attrs.contains_key("txt");
        } else if !known_command(value.name) {
            self.unsafe_state = true;
        }
        if self.stack.last() == Some(&"clip_path")
            && self
                .clip_path
                .as_mut()
                .is_some_and(|path| path.push(value).is_none())
        {
            self.clip_path = None;
        }
        if (value.name.starts_with("clip_") || value.name == "pop_clip")
            && (!drawing_scope(&self.stack) || (value.name == "pop_clip" && !value.empty))
        {
            return Err(invalid());
        }
        let position = self.operations;
        if matches!(value.name, "clip_image_mask" | "fill_image") {
            if let Some(receipt) = self.receipt {
                receipt.binds(position, value)?;
            }
            self.operations += 1;
        }
        if value.name.starts_with("clip_") {
            self.clips
                .push(if value.name == "clip_image_mask" && value.empty {
                    Clip::Mask(position)
                } else {
                    Clip::Unknown
                });
            if value.name == "clip_path" && !value.empty {
                self.clip_path = RectanglePath::new(value);
            }
        }
        if value.name == "pop_clip" {
            let floor = self
                .stack
                .iter()
                .zip(&self.clip_floors)
                .rev()
                .find(|(name, _)| matches!(**name, "group" | "page"))
                .map_or(0, |(_, floor)| *floor);
            if self.clips.len() <= floor {
                return Err(invalid());
            }
            self.clips.pop();
        }
        if value.name == "fill_image" {
            self.observe_image(value, page, position)?;
        }
        if !value.empty {
            self.stack.push(value.name);
            self.clip_floors.push(self.clips.len());
        }
        if self.stack.len() > 128 || self.clips.len() > 128 {
            return Err(invalid());
        }
        Ok(())
    }
    fn observe_image(
        &mut self,
        value: &Tag<'a>,
        page: &ReadingPage,
        position: usize,
    ) -> Result<()> {
        self.observed_images += 1;
        if self.observed_images > 256 {
            return Err(invalid());
        }
        if !drawing_scope(&self.stack) {
            self.unsafe_state = true;
        }
        let supported_mask = if !self.unsafe_state
            && image_rect(value, page).is_ok()
            && let [Clip::Mask(mask_position)] = self.clips.as_slice()
            && let Some(receipt) = self.receipt
        {
            receipt.region(*mask_position, position, page).ok()
        } else {
            None
        };
        if let Some(rect) = supported_mask {
            if let Some(rect) = rect {
                self.images.push(rect);
                self.mask_supported += 1;
            } else {
                self.mask_empty += 1;
            }
        } else {
            record_image(
                value,
                page,
                !self.unsafe_state,
                &self.clips,
                &mut self.images,
                &mut self.unsupported,
            )?;
        }
        Ok(())
    }
}

pub fn parse_trace(raw: &[u8], page: &ReadingPage) -> Result<(Vec<TextRect>, usize)> {
    let parsed = parse_trace_with_masks(raw, page, None)?;
    Ok((parsed.images, parsed.unsupported))
}

struct ParsedTrace {
    images: Vec<TextRect>,
    unsupported: usize,
    mask_supported: usize,
    mask_empty: usize,
    curved_paint: bool,
    observed_images: usize,
}

fn parse_trace_with_masks(
    raw: &[u8],
    page: &ReadingPage,
    receipt: Option<&masks::Receipt>,
) -> Result<ParsedTrace> {
    parse_trace_mode(raw, page, receipt, false)
}

fn parse_renderer_trace(raw: &[u8], page: &ReadingPage) -> Result<bool> {
    let parsed = parse_trace_mode(raw, page, None, true)?;
    Ok(parsed.curved_paint && parsed.observed_images == 0)
}

fn parse_trace_mode(
    raw: &[u8],
    page: &ReadingPage,
    receipt: Option<&masks::Receipt>,
    renderer: bool,
) -> Result<ParsedTrace> {
    if raw.len() > PAGE_BYTES
        || !page.width.is_finite()
        || !page.height.is_finite()
        || page.width <= 0.0
        || page.height <= 0.0
    {
        return Err(invalid());
    }
    let text = std::str::from_utf8(raw).map_err(|_| invalid())?;
    let mut rest = text;
    let mut state = TraceState {
        renderer,
        receipt,
        ..TraceState::default()
    };
    let mut pages = 0;
    let mut document_seen = false;
    let mut declaration_seen = false;
    while let Some(start) = rest.find('<') {
        if !rest[..start].trim().is_empty() {
            return Err(invalid());
        }
        rest = &rest[start + 1..];
        let (raw_tag, tail) = next_tag(rest)?;
        rest = tail;
        if raw_tag.starts_with("?xml ")
            && raw_tag.ends_with('?')
            && !document_seen
            && !declaration_seen
            && state.stack.is_empty()
        {
            declaration_seen = true;
            continue;
        }
        let value = tag(raw_tag)?;
        if value.close {
            state.close(value.name)?;
            continue;
        }
        if value.name == "document" {
            if document_seen || !state.stack.is_empty() || value.empty {
                return Err(invalid());
            }
            document_seen = true;
        } else if value.name == "page" {
            if state.stack.as_slice() != ["document"] || pages != 0 || value.empty {
                return Err(invalid());
            }
            pages += 1;
            validate_page(&value, page)?;
        } else if !state.stack.contains(&"page") {
            return Err(invalid());
        }
        state.observe(&value, page)?;
    }
    if !rest.trim().is_empty()
        || !state.stack.is_empty()
        || pages != 1
        || !document_seen
        || !state.clips.is_empty()
        || receipt.is_some_and(|receipt| receipt.len() != state.operations)
    {
        return Err(invalid());
    }
    if state.unsafe_state {
        if state.unsupported + state.images.len() + state.mask_empty == 0 {
            return Err(invalid());
        }
        state.unsupported += state.images.len() + state.mask_empty;
        state.images.clear();
        state.mask_supported = 0;
        state.mask_empty = 0;
    }
    Ok(ParsedTrace {
        images: state.images,
        unsupported: state.unsupported,
        mask_supported: state.mask_supported,
        mask_empty: state.mask_empty,
        curved_paint: state.curved_paint,
        observed_images: state.observed_images,
    })
}

impl GraphicsEvidence {
    /// Exact serialized commitment retained by offline experiments.
    pub fn basis_json(&self) -> Result<Vec<u8>> {
        let mut basis = self.clone();
        basis.generation.clear();
        Ok(serde_json::to_vec(&basis)?)
    }

    #[must_use]
    pub fn reusable_for(&self, prepared: &PreparedGraphics) -> bool {
        self.version == VERSION
            && self.cache_key == prepared.cache_key()
            && self.native_generation == prepared.evidence.native_generation
            && self.pdf_sha256 == prepared.evidence.pdf_sha256
            && self.tool_sha256 == prepared.evidence.tool_sha256
            && self.tool_path == prepared.evidence.tool_path
            && self.mask_runtime == prepared.evidence.mask_runtime
            && !self.pages.iter().any(|page| {
                matches!(page.status.as_str(), "tool_failed" | "resource_limit")
                    || page.mask.as_ref().is_some_and(|mask| {
                        matches!(mask.status.as_str(), "tool_failed" | "resource_limit")
                    })
                    || page.vectors.as_ref().is_some_and(|vectors| {
                        matches!(vectors.status.as_str(), "tool_failed" | "resource_limit")
                    })
            })
            && self
                .basis_json()
                .is_ok_and(|bytes| digest(&bytes) == self.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> ReadingPage {
        ReadingPage {
            number: 3,
            width: 600.0,
            height: 800.0,
            start: 0,
            end: 1,
            provenance: super::super::Provenance::Native,
            confidence: None,
        }
    }
    fn trace(body: &str) -> Vec<u8> {
        format!("<?xml version=\"1.0\"?><document filename=\"fixture.pdf\"><page number=\"3\" mediabox=\"0 0 600 800\">{body}</page></document>").into_bytes()
    }
    fn image(transform: &str) -> String {
        format!("<fill_image alpha=\"1\" transform=\"{transform}\" width=\"20\" height=\"30\"/>")
    }
    fn curve() -> &'static str {
        "<fill_path winding=\"nonzero\" colorspace=\"DeviceRGB\" color=\"0 0 0\" alpha=\"1\" transform=\"1 0 0 1 0 0\"><moveto x=\"10\" y=\"10\"/><curveto x1=\"12\" y1=\"9\" x2=\"13\" y2=\"14\" x3=\"10\" y3=\"10\"/><closepath/></fill_path>"
    }

    const fn identity_defaults() -> &'static str {
        "<set_default_colorspaces gray=\"DeviceGray\" rgb=\"DeviceRGB\" cmyk=\"DeviceCMYK\" oi=\"None\"/>"
    }

    #[test]
    fn should_accept_identity_device_defaults_when_they_are_the_first_page_command() {
        let body = format!(
            "{}<group bbox=\"5 5 60 60\" isolated=\"0\" knockout=\"1\" blendmode=\"Normal\" alpha=\"1\">{}</group>",
            identity_defaults(),
            curve()
        );
        assert!(parse_renderer_trace(&trace(&body), &page()).unwrap());
        assert!(parse_trace(&trace(&body), &page()).is_err());
        let image_body = format!("{}{}", identity_defaults(), image("10 0 0 10 0 0"));
        assert_eq!(
            parse_trace(&trace(&image_body), &page()).unwrap().0.len(),
            1
        );
        assert!(parse_renderer_trace(&trace(&image_body), &page()).is_err());
    }

    #[test]
    fn should_withhold_renderer_support_when_device_defaults_are_not_exact_identities() {
        for defaults in [
            identity_defaults().replace("DeviceGray", "ICCBased"),
            identity_defaults().replace("DeviceRGB", "DeviceCMYK"),
            identity_defaults().replace("DeviceCMYK", "DefaultCMYK"),
            identity_defaults().replace("oi=\"None\"", "oi=\"DeviceRGB\""),
            identity_defaults().replace(" oi=\"None\"", ""),
            identity_defaults().replace("/>", " extra=\"0\"/>"),
            identity_defaults().replace("/>", " gray=\"DeviceGray\"/>"),
            identity_defaults().replace("/>", "></set_default_colorspaces>"),
        ] {
            assert!(
                parse_renderer_trace(&trace(&format!("{defaults}{}", curve())), &page()).is_err(),
                "{defaults}"
            );
        }
    }

    #[test]
    fn should_withhold_renderer_support_when_identity_defaults_have_late_or_nested_placement() {
        for body in [
            format!("{}{}{}", identity_defaults(), identity_defaults(), curve()),
            format!("{}{}", curve(), identity_defaults()),
            format!(
                "<group bbox=\"0 0 600 800\" isolated=\"1\" knockout=\"0\" blendmode=\"Normal\" alpha=\"1\">{}{}</group>",
                identity_defaults(),
                curve()
            ),
            format!("<fill_path>{}</fill_path>{}", identity_defaults(), curve()),
        ] {
            assert!(
                parse_renderer_trace(&trace(&body), &page()).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn should_allow_original_normal_compositing_when_only_the_renderer_mode_is_selected() {
        for (isolated, knockout) in [("0", "0"), ("0", "1"), ("1", "1")] {
            let body = format!(
                "<group bbox=\"5 5 60 60\" isolated=\"{isolated}\" knockout=\"{knockout}\" blendmode=\"Normal\" alpha=\"0.5\">{}</group>",
                curve()
            );
            assert!(parse_renderer_trace(&trace(&body), &page()).unwrap());
            // The established image parser must still reject these states.
            assert!(parse_trace(&trace(&body), &page()).is_err());
        }
        assert!(!parse_renderer_trace(&trace(""), &page()).unwrap());
    }

    #[test]
    fn should_withhold_render_support_when_trace_state_or_path_data_is_unsupported() {
        for body in [
            format!("{}{}", curve(), image("10 0 0 10 0 0")),
            format!("<fill_shade/>{}", curve()),
            format!(
                "<clip_path winding=\"nonzero\" transform=\"1 0 0 1 0 0\"/>{}<pop_clip/>",
                curve()
            ),
            format!(
                "<group bbox=\"0 0 600 800\" isolated=\"1\" knockout=\"0\" blendmode=\"Multiply\" alpha=\"1\">{}</group>",
                curve()
            ),
            curve().replace("x1=\"12\"", "x1=\"NaN\""),
            curve().replace("1 0 0 1 0 0", "0 0 0 0 0 0"),
            curve().replace("<moveto", "<future_path"),
            format!(
                "<clip_path winding=\"nonzero\" transform=\"1 0 0 1 0 0\"><moveto x=\"0\" y=\"0\"/><lineto x=\"10\" y=\"10\"/><closepath/></clip_path>{}<pop_clip/>",
                curve()
            ),
        ] {
            assert!(
                parse_renderer_trace(&trace(&body), &page()).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn should_preserve_clip_lifetimes_when_the_renderer_uses_a_closed_rectangular_clip() {
        let clip = "<clip_path winding=\"nonzero\" transform=\"1 0 0 1 0 0\"><moveto x=\"0\" y=\"0\"/><lineto x=\"40\" y=\"0\"/><lineto x=\"40\" y=\"40\"/><lineto x=\"0\" y=\"40\"/><closepath/></clip_path>";
        assert!(
            parse_renderer_trace(&trace(&format!("{clip}{}<pop_clip/>", curve())), &page())
                .unwrap()
        );
        assert!(parse_renderer_trace(&trace(&format!("{clip}{}", curve())), &page()).is_err());
    }

    #[test]
    fn should_match_independent_fixtures_when_trace_operations_cross_support_boundaries() {
        let packet: serde_json::Value =
            serde_json::from_str(include_str!("../../eval/fixtures/graphics-traces.json")).unwrap();
        let mut native = page();
        native.width = 612.0;
        native.height = 792.0;
        for case in packet["cases"].as_array().unwrap() {
            let result = parse_trace(case["trace"].as_str().unwrap().as_bytes(), &native);
            if case["disposition"] == "rejected" {
                assert!(result.is_err(), "{}", case["name"]);
                continue;
            }
            if case["disposition"] == "rejected_or_no_supported_placements" && result.is_err() {
                continue;
            }
            let (images, _) = result.unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
            assert_eq!(
                serde_json::to_value(images).unwrap(),
                *case
                    .get("graphics_v2_rects")
                    .unwrap_or_else(|| &case["rects"]),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn should_preserve_exact_scoped_bounds_when_groups_and_rectangular_clips_are_supported() {
        let packet: serde_json::Value =
            serde_json::from_str(include_str!("../../eval/fixtures/graphics-scopes.json")).unwrap();
        for case in packet["cases"].as_array().unwrap() {
            let raw = case["trace"].as_str().unwrap().as_bytes();
            assert_eq!(digest(raw), case["sha256"].as_str().unwrap());
            let result = parse_trace(raw, &page());
            if case["disposition"] == "rejected" {
                assert!(result.is_err(), "{}", case["name"]);
                continue;
            }
            if case["disposition"] == "rejected_or_unsupported" && result.is_err() {
                continue;
            }
            let (images, excluded) = result.unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
            assert_eq!(
                serde_json::to_value(images).unwrap(),
                case["rects"],
                "{}",
                case["name"]
            );
            assert_eq!(
                excluded,
                usize::try_from(case["unsupported_images"].as_u64().unwrap()).unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn should_reject_excess_images_when_supported_clips_hide_every_placement() {
        let clip = "<clip_path winding=\"nonzero\" transform=\"1 0 0 1 0 0\"><moveto x=\"300\" y=\"300\"/><lineto x=\"310\" y=\"300\"/><lineto x=\"310\" y=\"310\"/><lineto x=\"300\" y=\"310\"/><closepath/></clip_path>";
        let raw = trace(&format!(
            "{clip}{}<pop_clip/>",
            image("100 0 0 50 20 30").repeat(256)
        ));
        assert_eq!(parse_trace(&raw, &page()).unwrap(), (vec![], 0));
        let excessive = trace(&format!(
            "{clip}{}<pop_clip/>",
            image("100 0 0 50 20 30").repeat(257)
        ));
        assert!(parse_trace(&excessive, &page()).is_err());
    }

    #[test]
    fn should_preserve_page_coordinates_when_images_are_flipped_or_rotated() {
        for transform in [
            "100 0 0 50 20 30",
            "-100 0 0 -50 120 80",
            "0 50 -100 0 120 30",
        ] {
            let (images, excluded) = parse_trace(&trace(&image(transform)), &page()).unwrap();
            assert_eq!(excluded, 0);
            assert_eq!(images.len(), 1);
            let rect = images[0];
            assert_eq!(
                (rect.x_min, rect.x_max, rect.y_min, rect.y_max),
                (20.0, 120.0, 30.0, 80.0)
            );
        }
    }
    #[test]
    fn should_withhold_image_geometry_when_placement_is_unsupported() {
        for transform in [
            "1 1 0 1 20 30",
            "0 0 0 0 20 30",
            "100 0 0 50 -1 30",
            "100 0 0 50 550 30",
            "NaN 0 0 50 20 30",
            "inf 0 0 50 20 30",
        ] {
            let (images, excluded) = parse_trace(&trace(&image(transform)), &page()).unwrap();
            assert!(images.is_empty());
            assert_eq!(excluded, 1);
        }
        for changed in [
            image("100 0 0 50 20 30").replace("alpha=\"1\"", "alpha=\"0.5\""),
            image("100 0 0 50 20 30").replace("width=\"20\"", "width=\"0\""),
        ] {
            assert_eq!(parse_trace(&trace(&changed), &page()).unwrap().1, 1);
        }
    }
    #[test]
    fn should_preserve_unclipped_images_when_prior_clipping_has_ended() {
        let drawing = image("100 0 0 50 20 30");
        let raw = trace(&format!(
            "<clip_path><moveto x=\"0\" y=\"0\"/></clip_path>{drawing}<pop_clip/>{drawing}"
        ));
        let (images, excluded) = parse_trace(&raw, &page()).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(excluded, 1);
        assert!(parse_trace(&trace("<pop_clip/>"), &page()).is_err());
        assert!(parse_trace(&trace("<clip_path/>"), &page()).is_err());
    }
    #[test]
    fn should_withhold_all_placements_when_mask_group_or_unknown_state_is_present() {
        let drawing = image("100 0 0 50 20 30");
        for state in [
            "<begin_mask/>",
            "<begin_group/>",
            "<begin_tile/>",
            "<unknown_hook/>",
        ] {
            let (images, excluded) =
                parse_trace(&trace(&format!("{drawing}{state}{drawing}")), &page()).unwrap();
            assert!(images.is_empty());
            assert_eq!(excluded, 2);
        }
    }
    #[test]
    fn should_reject_ambiguous_framing_when_trace_pages_or_attributes_conflict() {
        let good = String::from_utf8(trace(&image("100 0 0 50 20 30"))).unwrap();
        for raw in [
            good.replace("number=\"3\"", "number=\"4\""),
            good.replace("600 800", "600 801"),
            good.replace("alpha=\"1\"", "alpha=\"1\" alpha=\"0\""),
            good.replace("alpha=\"1\" ", "alpha=\"1\""),
            good.replace("</page>", "</wrong>"),
            format!("{good}{good}"),
            good.replace("<page ", "<!DOCTYPE danger><page "),
        ] {
            assert!(parse_trace(raw.as_bytes(), &page()).is_err(), "{raw}");
        }
        let quoted =
            trace("<fill_text><span><g unicode=\"&lt;fill_image x='1'&gt;\"/></span></fill_text>");
        assert!(parse_trace(&quoted, &page()).unwrap().0.is_empty());
    }
    #[test]
    fn should_reject_nonfinite_native_pages_when_trace_geometry_is_finite() {
        for dimension in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            let mut invalid_page = page();
            invalid_page.width = dimension;
            assert!(parse_trace(&trace(&image("100 0 0 50 20 30")), &invalid_page).is_err());
            let mut invalid_page = page();
            invalid_page.height = dimension;
            assert!(parse_trace(&trace(&image("100 0 0 50 20 30")), &invalid_page).is_err());
        }
    }

    #[test]
    fn should_reject_nested_state_when_path_payloads_try_to_pop_or_create_clips() {
        for prefix in [
            "<clip_path><pop_clip/></clip_path>",
            "<fill_path><clip_path/></fill_path>",
            "<pop_clip></pop_clip>",
        ] {
            assert!(
                parse_trace(
                    &trace(&format!("{prefix}{}", image("100 0 0 50 20 30"))),
                    &page()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn should_withhold_completion_when_unknown_state_contains_no_observed_images() {
        assert!(parse_trace(&trace("<unknown_hook/>"), &page()).is_err());
    }

    #[test]
    fn should_enforce_resource_bounds_when_traces_exceed_supported_inventory() {
        assert!(parse_trace(&trace(&image("100 0 0 50 20 30").repeat(257)), &page()).is_err());
        assert!(parse_trace(&vec![b' '; PAGE_BYTES + 1], &page()).is_err());
        assert!(
            parse_trace(
                &trace(&format!("<fill_text value=\"{}\"/>", "x".repeat(65537))),
                &page()
            )
            .is_err()
        );
    }
}
