//! Decoded opacity evidence supplements, but never replaces, strict trace state.
use super::{ReadingPage, Tag, TextRect, digest, file_hash, invalid, numbers};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

pub const SCRIPT: &str = include_str!("masks.js");
const PIXELS: usize = 4_000_000;
const TOTAL_PIXELS: usize = 6_000_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaskRuntime {
    pub wrapper_path: PathBuf,
    pub wrapper_sha256: String,
    pub script_sha256: String,
}
impl MaskRuntime {
    pub async fn prepare() -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let path = PathBuf::from("/usr/bin/prlimit").canonicalize().ok()?;
        Some(Self {
            wrapper_sha256: file_hash(&path, 8 * 1024 * 1024).await.ok()?,
            wrapper_path: path,
            script_sha256: digest(SCRIPT.as_bytes()),
        })
    }
    pub fn key(&self) -> (&Path, &str, &str) {
        (
            &self.wrapper_path,
            &self.wrapper_sha256,
            &self.script_sha256,
        )
    }
    pub async fn unchanged(&self) -> bool {
        file_hash(&self.wrapper_path, 8 * 1024 * 1024)
            .await
            .is_ok_and(|hash| hash == self.wrapper_sha256)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskPageEvidence {
    pub status: String,
    pub receipt_sha256: Option<String>,
    pub supported_images: usize,
    pub empty_images: usize,
}

pub async fn collect(
    runtime: &MaskRuntime,
    program: &Path,
    source: &Path,
    page: u32,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut child = Command::new(&runtime.wrapper_path)
        .args([
            "--as=805306368:805306368",
            "--cpu=8:8",
            "--core=0:0",
            "--fsize=16777216:16777216",
            "--",
        ])
        .arg(program)
        .args(["run", "/dev/stdin"])
        .arg(source)
        .arg(page.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| Error::io(&runtime.wrapper_path, error))?;
    let mut input = child.stdin.take().ok_or_else(invalid)?;
    let stdout = child.stdout.take().ok_or_else(invalid)?;
    let stderr = child.stderr.take().ok_or_else(invalid)?;
    let send = async {
        input
            .write_all(SCRIPT.as_bytes())
            .await
            .map_err(|error| Error::io("mask script stdin", error))?;
        input
            .shutdown()
            .await
            .map_err(|error| Error::io("mask script stdin", error))?;
        drop(input);
        Ok::<_, Error>(())
    };
    let read = async {
        let mut bytes = Vec::new();
        stdout
            .take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| Error::io("mask stdout", error))?;
        if bytes.len() > limit {
            return Err(invalid());
        }
        Ok(bytes)
    };
    let errors = async {
        let mut bytes = Vec::new();
        stderr
            .take(65537)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| Error::io("mask stderr", error))?;
        if bytes.len() > 65536 {
            return Err(invalid());
        }
        Ok(bytes)
    };
    let ((), raw, errors) = tokio::try_join!(send, read, errors)?;
    let status = child
        .wait()
        .await
        .map_err(|error| Error::io(program, error))?;
    if !status.success() {
        return Err(Error::CommandFailed {
            program: "mutool mask opacity".into(),
            status: status.to_string(),
            stderr: String::from_utf8_lossy(&errors).trim().into(),
        });
    }
    Ok(raw)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    schema_version: u16,
    page: u32,
    bounds: [f64; 4],
    operations: Vec<Operation>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    event_index: usize,
    kind: String,
    matrix: [f64; 6],
    alpha: Option<f64>,
    image: Image,
    mask: Option<Opacity>,
    base: Option<Pixmap>,
    error: Option<String>,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Image {
    width: u32,
    height: u32,
    components: u32,
    bits: u32,
    image_mask: bool,
    interpolate: bool,
    color_key: Option<Vec<f64>>,
    decode: Option<Vec<f64>>,
    orientation: u32,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Pixmap {
    width: u32,
    height: u32,
    components: u32,
    alpha: bool,
    bounds: [u32; 4],
    stride: u32,
    colorspace_components: Option<u32>,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Opacity {
    image: Image,
    pixmap: Pixmap,
    sample_hex_rows: Vec<String>,
}

impl Image {
    fn pixels(&self) -> Option<usize> {
        let n = usize::try_from(self.width)
            .ok()?
            .checked_mul(usize::try_from(self.height).ok()?)?;
        (self.width > 0
            && self.height > 0
            && self.width <= 4096
            && self.height <= 4096
            && n <= PIXELS)
            .then_some(n)
    }
    fn plain(&self) -> bool {
        self.pixels().is_some()
            && self.bits == 8
            && !self.image_mask
            && !self.interpolate
            && self.color_key.is_none()
            && self.decode.is_none()
            && self.orientation == 0
    }
}
impl Pixmap {
    fn matches(&self, image: &Image) -> bool {
        self.width == image.width
            && self.height == image.height
            && self.bounds == [0, 0, image.width, image.height]
    }
}
impl Opacity {
    fn support(&self) -> Result<Option<[u32; 4]>> {
        if !self.image.plain()
            || self.image.components != 1
            || !self.pixmap.matches(&self.image)
            || self.pixmap.components != 1
            || !self.pixmap.alpha
            || self.pixmap.colorspace_components.is_some()
            || self.pixmap.stride != self.image.width
            || self.sample_hex_rows.len()
                != usize::try_from(self.image.height).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        let mut bounds = [self.image.width, self.image.height, 0, 0];
        for (y, row) in self.sample_hex_rows.iter().enumerate() {
            if row.len() != usize::try_from(self.image.width).map_err(|_| invalid())? * 2
                || !row
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid());
            }
            for (x, pair) in row.as_bytes().chunks_exact(2).enumerate() {
                if pair != b"00" {
                    let x = u32::try_from(x).map_err(|_| invalid())?;
                    let y = u32::try_from(y).map_err(|_| invalid())?;
                    bounds = [
                        bounds[0].min(x),
                        bounds[1].min(y),
                        bounds[2].max(x + 1),
                        bounds[3].max(y + 1),
                    ];
                }
            }
        }
        Ok((bounds[0] < bounds[2] && bounds[1] < bounds[3]).then_some(bounds))
    }
}

impl Receipt {
    pub fn parse(raw: &[u8], page: &ReadingPage) -> Result<Self> {
        if raw.len() > super::PAGE_BYTES {
            return Err(invalid());
        }
        let receipt: Self = serde_json::from_slice(raw)?;
        if receipt.schema_version != 1
            || receipt.page != page.number
            || receipt.bounds != [0.0, 0.0, f64::from(page.width), f64::from(page.height)]
            || receipt.operations.len() > 512
        {
            return Err(invalid());
        }
        let mut pixels = 0_usize;
        for (position, operation) in receipt.operations.iter().enumerate() {
            if operation.event_index != position {
                return Err(invalid());
            }
            if !matches!(operation.kind.as_str(), "fill_image" | "clip_image_mask")
                || operation.matrix.iter().any(|v| !v.is_finite())
            {
                return Err(invalid());
            }
            if let Some(mask) = &operation.mask {
                pixels = pixels
                    .checked_add(mask.image.pixels().ok_or_else(invalid)?)
                    .ok_or_else(invalid)?;
                if pixels > TOTAL_PIXELS {
                    return Err(invalid());
                }
            }
        }
        Ok(receipt)
    }
    pub fn len(&self) -> usize {
        self.operations.len()
    }
    pub fn binds(&self, position: usize, value: &Tag<'_>) -> Result<()> {
        let operation = self.operations.get(position).ok_or_else(invalid)?;
        let matrix = numbers::<6>(value.attrs.get("transform").ok_or_else(invalid)?)?;
        if operation.kind != value.name
            || operation.matrix != matrix.map(f64::from)
            || Some(operation.image.width) != value.attrs.get("width").and_then(|v| v.parse().ok())
            || Some(operation.image.height)
                != value.attrs.get("height").and_then(|v| v.parse().ok())
            || (value.name == "fill_image"
                && operation.alpha != value.attrs.get("alpha").and_then(|v| v.parse().ok()))
        {
            return Err(invalid());
        }
        Ok(())
    }
    /// Only one mask with identical image coordinates is supported. Additional
    /// cropping must not intersect a bbox which contains unpainted holes.
    pub fn region(
        &self,
        mask_position: usize,
        image_position: usize,
        page: &ReadingPage,
    ) -> Result<Option<TextRect>> {
        let mask = self.operations.get(mask_position).ok_or_else(invalid)?;
        let image = self.operations.get(image_position).ok_or_else(invalid)?;
        let opacity = mask.mask.as_ref().ok_or_else(invalid)?;
        let base = image.base.as_ref().ok_or_else(invalid)?;
        if mask.kind != "clip_image_mask"
            || image.kind != "fill_image"
            || mask.error.is_some()
            || image.error.is_some()
            || mask.matrix != image.matrix
            || mask.image != opacity.image
            || image.mask.as_ref() != Some(opacity)
            || !image.image.plain()
            || image.image.width != opacity.image.width
            || image.image.height != opacity.image.height
            || image.alpha != Some(1.0)
            || !base.matches(&image.image)
            || base.alpha
            || !matches!(base.colorspace_components, Some(1 | 3 | 4))
            || base.colorspace_components != Some(base.components)
            || base.components != image.image.components
            || base.stride
                != image
                    .image
                    .width
                    .checked_mul(base.components)
                    .ok_or_else(invalid)?
        {
            return Err(invalid());
        }
        let Some(bounds) = opacity.support()? else {
            return Ok(None);
        };
        transformed_bounds(
            bounds,
            opacity.image.width,
            opacity.image.height,
            image.matrix,
            page,
        )
        .map(Some)
    }
}

#[allow(clippy::cast_possible_truncation)] // Page-checked finite coordinates narrow to the native f32 geometry type.
fn transformed_bounds(
    bounds: [u32; 4],
    width: u32,
    height: u32,
    m: [f64; 6],
    page: &ReadingPage,
) -> Result<TextRect> {
    let [a, b, c, d, e, f] = m;
    if !((b == 0.0 && c == 0.0 && a != 0.0 && d != 0.0)
        || (a == 0.0 && d == 0.0 && b != 0.0 && c != 0.0))
    {
        return Err(invalid());
    }
    let points = [
        (bounds[0], bounds[1]),
        (bounds[2], bounds[1]),
        (bounds[0], bounds[3]),
        (bounds[2], bounds[3]),
    ]
    .map(|(x, y)| {
        let x = f64::from(x) / f64::from(width);
        let y = f64::from(y) / f64::from(height);
        (a.mul_add(x, c.mul_add(y, e)), b.mul_add(x, d.mul_add(y, f)))
    });
    let low_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let low_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let high_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let high_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite())
        || low_x < 0.0
        || low_y < 0.0
        || high_x > f64::from(page.width)
        || high_y > f64::from(page.height)
        || low_x >= high_x
        || low_y >= high_y
    {
        return Err(invalid());
    }
    let rect = TextRect {
        x_min: low_x as f32,
        y_min: low_y as f32,
        x_max: high_x as f32,
        y_max: high_y as f32,
    };
    if rect.x_min >= rect.x_max || rect.y_min >= rect.y_max {
        return Err(invalid());
    }
    Ok(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../../eval/fixtures/mask-events.json")).unwrap()
    }
    fn page() -> ReadingPage {
        ReadingPage {
            number: 1,
            width: 400.0,
            height: 300.0,
            start: 0,
            end: 1,
            provenance: crate::source_index::Provenance::Native,
            confidence: None,
        }
    }
    fn evaluate(case: &Value) -> Result<super::super::ParsedTrace> {
        let raw = serde_json::to_vec(&case["receipt"])?;
        let receipt = Receipt::parse(&raw, &page())?;
        super::super::parse_trace_with_masks(
            case["trace"].as_str().unwrap().as_bytes(),
            &page(),
            Some(&receipt),
        )
    }
    fn expected_rect(value: &Value) -> [f64; 4] {
        serde_json::from_value(value.clone()).unwrap()
    }
    fn rect_values(rect: TextRect) -> [f64; 4] {
        [rect.x_min, rect.y_min, rect.x_max, rect.y_max].map(f64::from)
    }
    #[test]
    fn should_use_complete_pixel_support_when_independent_mask_fixtures_define_the_evidence() {
        let data = fixtures();
        assert_eq!(
            data["independent_packet_sha256"],
            digest(include_bytes!("../../../eval/fixtures/mask-support.json"))
        );
        let cases = data["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 43);
        for case in cases {
            let result = evaluate(case);
            let name = case["id"].as_str().unwrap();
            match case["expected"]["status"].as_str().unwrap() {
                "withhold" => assert!(
                    result
                        .as_ref()
                        .map_or(true, |parsed| parsed.images.is_empty()),
                    "{name}"
                ),
                "no_image" => {
                    let parsed = result.unwrap();
                    assert!(parsed.images.is_empty(), "{name}");
                    assert_eq!(parsed.mask_empty, 1);
                    assert_eq!(parsed.unsupported, 0);
                }
                "candidate_bounds" => {
                    let parsed = result.unwrap();
                    assert_eq!(parsed.images.len(), 1, "{name}");
                    assert_eq!(parsed.mask_supported, 1);
                    assert_eq!(parsed.unsupported, 0);
                    let expected = expected_rect(&case["expected"]["page_bbox"]);
                    for (actual, want) in rect_values(parsed.images[0]).into_iter().zip(expected) {
                        assert!((actual - want).abs() < 0.0001, "{name}: {actual} != {want}");
                    }
                }
                "two_distinct_candidates" => {
                    let parsed = result.unwrap();
                    let expected = case["expected"]["events"].as_array().unwrap();
                    assert_eq!(parsed.images.len(), expected.len());
                    for (rect, event) in parsed.images.into_iter().zip(expected) {
                        for (actual, want) in rect_values(rect)
                            .into_iter()
                            .zip(expected_rect(&event["page_bbox"]))
                        {
                            assert!((actual - want).abs() < 0.0001, "{name}");
                        }
                    }
                }
                other => panic!("unknown independent disposition {other}"),
            }
        }
    }
    #[test]
    fn should_preserve_trace_exclusions_when_mask_receipts_are_absent_or_do_not_bind() {
        let data = fixtures();
        let case = data["cases"][1].clone();
        let before =
            super::super::parse_trace(case["trace"].as_str().unwrap().as_bytes(), &page()).unwrap();
        assert!(before.0.is_empty());
        assert_eq!(before.1, 1);
        for field in ["page", "schema_version"] {
            let mut changed = case.clone();
            changed["receipt"][field] = json!(9);
            assert!(evaluate(&changed).is_err());
        }
        let mut changed = case.clone();
        changed["receipt"]["operations"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert!(evaluate(&changed).is_err());
        let mut changed = case.clone();
        changed["receipt"]["operations"][1]["matrix"][4] = json!(11);
        assert!(evaluate(&changed).is_err());
        let after =
            super::super::parse_trace(case["trace"].as_str().unwrap().as_bytes(), &page()).unwrap();
        assert_eq!(before, after);
    }
    #[test]
    fn should_reject_unbounded_or_malformed_opacity_when_receipt_metadata_cannot_prove_support() {
        let data = fixtures();
        let case = data["cases"][1].clone();
        let mut oversized = case.clone();
        oversized["receipt"]["operations"][0]["mask"]["image"]["width"] = json!(4097);
        assert!(evaluate(&oversized).is_err());
        let mut repeated = case.clone();
        let operation = repeated["receipt"]["operations"][0].clone();
        repeated["receipt"]["operations"] = json!(vec![operation; 513]);
        assert!(evaluate(&repeated).is_err());
        for replacement in [json!("ff"), json!("GG000000"), json!("ff00000000")] {
            let mut changed = case.clone();
            for index in [0, 1] {
                changed["receipt"]["operations"][index]["mask"]["sample_hex_rows"][0] =
                    replacement.clone();
            }
            let result = evaluate(&changed).unwrap();
            assert!(result.images.is_empty());
            assert_eq!(result.unsupported, 1);
        }
        let mut changed = case;
        changed["receipt"]["operations"][1]["base"]["stride"] = json!(1);
        assert!(evaluate(&changed).unwrap().images.is_empty());
    }
}
