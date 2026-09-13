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

pub const VERSION: u16 = 1;
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
}

pub struct PreparedGraphics {
    evidence: GraphicsEvidence,
    program: Option<PathBuf>,
}

pub struct GraphicsDocument {
    pub evidence: GraphicsEvidence,
    /// Optional external experiment evidence; never part of a production object JSON.
    pub traces: Vec<(u32, Vec<u8>)>,
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
}

pub async fn prepare(source: &Path, document: &IndexDocument) -> Result<PreparedGraphics> {
    let pdf_sha256 = file_hash(source, 512 * 1024 * 1024).await?;
    let program = program_path();
    let tool_sha256 = match &program {
        Some(path) => file_hash(path, 128 * 1024 * 1024).await.ok(),
        None => None,
    };
    let program = program.filter(|_| tool_sha256.is_some());
    let cache_key = digest(&serde_json::to_vec(&(
        VERSION,
        &document.etag,
        &pdf_sha256,
        &tool_sha256,
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
    let started = Instant::now();
    let mut total = 0;
    let mut traces = Vec::new();
    for (position, page) in pages.iter().enumerate() {
        let mut result = PageGraphics {
            page: *page,
            status: "unavailable".into(),
            trace_sha256: None,
            images: vec![],
            unsupported_images: 0,
        };
        if position >= MAX_PAGES
            || total >= TOTAL_BYTES
            || started.elapsed() > Duration::from_secs(30)
        {
            result.status = "resource_limit".into();
        } else if let Some(program) = &prepared.program {
            let mut command = trace_command(program, source, *page);
            let output = tokio::time::timeout(
                Duration::from_secs(5),
                super::bounded_command(&mut command, "mutool", PAGE_BYTES.min(TOTAL_BYTES - total)),
            )
            .await;
            if let Ok(Ok(raw)) = output {
                total += raw.len();
                result.trace_sha256 = Some(digest(&raw));
                if let Some(native) = document.index.pages.iter().find(|p| p.number == *page)
                    && let Ok((images, unsupported)) = parse_trace(&raw, native)
                {
                    result.images = images;
                    result.unsupported_images = unsupported;
                    result.status = if unsupported == 0 {
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
    if file_hash(source, 512 * 1024 * 1024).await? != prepared.evidence.pdf_sha256 {
        return Err(Error::InvalidRequest(
            "PDF changed while deriving graphics".into(),
        ));
    }
    if let Some(program) = &prepared.program
        && Some(file_hash(program, 128 * 1024 * 1024).await?) != prepared.evidence.tool_sha256
    {
        return Err(Error::InvalidRequest(
            "Graphics tool changed during derivation".into(),
        ));
    }
    prepared.evidence.generation = digest(&prepared.evidence.basis_json()?);
    Ok(GraphicsDocument {
        evidence: prepared.evidence,
        traces,
    })
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
    images: &mut Vec<TextRect>,
    unsupported: &mut usize,
) -> Result<()> {
    if images.len() + *unsupported >= 256 {
        return Err(invalid());
    }
    if visible && let Ok(rect) = image_rect(value, page) {
        images.push(rect);
    } else {
        *unsupported += 1;
    }
    Ok(())
}

/// Trace commands already carry page-space matrices. Unsupported drawing state
/// never becomes a claimed image rectangle; native text geometry is untouched.
pub fn parse_trace(raw: &[u8], page: &ReadingPage) -> Result<(Vec<TextRect>, usize)> {
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
    let mut stack = Vec::new();
    let mut images = Vec::new();
    let mut unsupported = 0;
    let mut pages = 0;
    let mut document_seen = false;
    let mut clips = 0_usize;
    let mut unsafe_state = false;
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
            && stack.is_empty()
        {
            declaration_seen = true;
            continue;
        }
        let value = tag(raw_tag)?;
        if value.close {
            if stack.pop() != Some(value.name) {
                return Err(invalid());
            }
            continue;
        }
        if value.name == "document" {
            if document_seen || !stack.is_empty() || value.empty {
                return Err(invalid());
            }
            document_seen = true;
        } else if value.name == "page" {
            if stack.as_slice() != ["document"] || pages != 0 || value.empty {
                return Err(invalid());
            }
            pages += 1;
            validate_page(&value, page)?;
        } else if !stack.contains(&"page") {
            return Err(invalid());
        }
        if !known_command(value.name) {
            unsafe_state = true;
        }
        if (value.name.starts_with("clip_") || value.name == "pop_clip")
            && (stack.as_slice() != ["document", "page"]
                || (value.name == "pop_clip" && !value.empty))
        {
            return Err(invalid());
        }
        if value.name.starts_with("clip_") {
            clips = clips.checked_add(1).ok_or_else(invalid)?;
        }
        if value.name == "pop_clip" {
            clips = clips.checked_sub(1).ok_or_else(invalid)?;
        }
        if value.name == "fill_image" {
            if stack.as_slice() != ["document", "page"] {
                unsafe_state = true;
            }
            record_image(
                &value,
                page,
                clips == 0 && !unsafe_state,
                &mut images,
                &mut unsupported,
            )?;
        }
        if !value.empty {
            stack.push(value.name);
        }
        if stack.len() > 128 || clips > 128 {
            return Err(invalid());
        }
    }
    if !rest.trim().is_empty() || !stack.is_empty() || pages != 1 || !document_seen || clips != 0 {
        return Err(invalid());
    }
    if unsafe_state {
        if unsupported + images.len() == 0 {
            return Err(invalid());
        }
        unsupported += images.len();
        images.clear();
    }
    Ok((images, unsupported))
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
            && !self
                .pages
                .iter()
                .any(|page| matches!(page.status.as_str(), "tool_failed" | "resource_limit"))
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
                case["rects"],
                "{}",
                case["name"]
            );
        }
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
