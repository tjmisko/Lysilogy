//! Bounded final-page RGB support. These are candidate pixels, not semantic truth.
use super::{MaskRuntime, ReadingPage, TextRect, digest};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, time::Instant};
use tokio::process::Command;

pub const DPI: u16 = 144;
pub const CONTRAST: u8 = 5;
pub const MAX_PIXELS: usize = 4_194_304;
pub const MAX_COMPONENTS: usize = 4096;
const MAX_SIDE: u16 = 8192;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InkComponent {
    /// Integer pixel edges, formed from the entire page before any exclusion.
    pub bounds: [u16; 4],
    pub pixels: u32,
}

impl InkComponent {
    #[must_use]
    pub fn rect(&self) -> TextRect {
        let [x0, y0, x1, y1] = self.bounds.map(|v| f32::from(v) * 0.5);
        TextRect {
            x_min: x0,
            y_min: y0,
            x_max: x1,
            y_max: y1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VectorPageEvidence {
    pub status: String,
    pub raster_sha256: Option<String>,
    pub dpi: u16,
    pub contrast: u8,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub components: Vec<InkComponent>,
    pub diagnostic: Option<String>,
}

impl VectorPageEvidence {
    #[must_use]
    pub fn unavailable(status: &str) -> Self {
        Self {
            status: status.into(),
            raster_sha256: None,
            dpi: DPI,
            contrast: CONTRAST,
            width: None,
            height: None,
            components: Vec::new(),
            diagnostic: None,
        }
    }
}

#[must_use]
pub fn command(runtime: &MaskRuntime, program: &Path, source: &Path, page: u32) -> Command {
    // Reuse the already hashed Linux resource wrapper; no script or shell runs.
    let mut command = Command::new(&runtime.wrapper_path);
    command
        .args([
            "--as=805306368:805306368",
            "--cpu=5:5",
            "--core=0:0",
            "--fsize=16777216:16777216",
            "--",
        ])
        .arg(program)
        .args([
            "draw", "-F", "pam", "-c", "rgb", "-r", "144", "-A", "8", "-q", "-o", "-",
        ])
        .arg(source)
        .arg(page.to_string());
    command
}

pub async fn render(
    runtime: &MaskRuntime,
    program: &Path,
    source: &Path,
    page: u32,
    limit: usize,
) -> Result<Vec<u8>> {
    super::super::bounded_command(
        &mut command(runtime, program, source, page),
        "mutool vector support",
        limit,
    )
    .await
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn dimensions(page: &ReadingPage) -> Option<(u16, u16)> {
    // Finite positive values and the side cap prove the casts fit exactly.
    let [width, height] = [page.width, page.height].map(|n| (n * 2.0).ceil());
    if ![width, height]
        .into_iter()
        .all(|n| n.is_finite() && n > 0.0 && n <= f32::from(MAX_SIDE))
    {
        return None;
    }
    let (width, height) = (width as u16, height as u16);
    (usize::from(width) * usize::from(height) <= MAX_PIXELS).then_some((width, height))
}

fn samples(raw: &[u8], width: u16, height: u16) -> Option<&[u8]> {
    let end = raw[..raw.len().min(1031)]
        .windows(7)
        .position(|s| s == b"ENDHDR\n")?;
    if end > 1024 {
        return None;
    }
    let mut lines = std::str::from_utf8(&raw[..end]).ok()?.lines();
    if lines.next()? != "P7" {
        return None;
    }
    let mut fields = BTreeMap::new();
    for line in lines {
        let mut words = line.split_ascii_whitespace();
        let (key, value) = (words.next()?, words.next()?);
        if words.next().is_some() || fields.insert(key, value).is_some() {
            return None;
        }
    }
    if fields.len() != 5
        || fields.get("WIDTH")?.parse::<u16>().ok()? != width
        || fields.get("HEIGHT")?.parse::<u16>().ok()? != height
        || fields.get("DEPTH") != Some(&"3")
        || fields.get("MAXVAL") != Some(&"255")
        || fields.get("TUPLTYPE") != Some(&"RGB")
    {
        return None;
    }
    let pixels = &raw[end + 7..];
    (pixels.len() == usize::from(width) * usize::from(height) * 3).then_some(pixels)
}

pub fn decode(
    raw: &[u8],
    page: &ReadingPage,
    deadline: Instant,
) -> std::result::Result<VectorPageEvidence, &'static str> {
    let (width, height) = dimensions(page).ok_or("pixel_limit")?;
    let pixels = samples(raw, width, height).ok_or("unsupported_raster")?;
    let components = components(pixels, width, height, deadline)?;
    Ok(VectorPageEvidence {
        status: "complete".into(),
        raster_sha256: Some(digest(raw)),
        dpi: DPI,
        contrast: CONTRAST,
        width: Some(width),
        height: Some(height),
        components,
        diagnostic: None,
    })
}

fn components(
    rgb: &[u8],
    width: u16,
    height: u16,
    deadline: Instant,
) -> std::result::Result<Vec<InkComponent>, &'static str> {
    let w = usize::from(width);
    let h = usize::from(height);
    let mut ink = rgb
        .chunks_exact(3)
        .map(|pixel| u8::from(pixel.iter().any(|v| *v <= 255 - CONTRAST)))
        .collect::<Vec<_>>();
    let mut result = Vec::new();
    let mut pending = Vec::new();
    let mut visited = 0_usize;
    for start in 0..ink.len() {
        if start.is_multiple_of(w) && Instant::now() >= deadline {
            return Err("resource_limit");
        }
        if ink[start] == 0 {
            continue;
        }
        if result.len() == MAX_COMPONENTS {
            return Err("component_limit");
        }
        let (mut x0, mut y0) = (width, height);
        let (mut x1, mut y1) = (0, 0);
        let mut count = 0_u32;
        ink[start] = 0;
        pending.push(start);
        while let Some(position) = pending.pop() {
            visited += 1;
            if visited.is_multiple_of(4096) && Instant::now() >= deadline {
                return Err("resource_limit");
            }
            let (x, y) = (position % w, position / w);
            let (px, py) = (
                u16::try_from(x).map_err(|_| "pixel_limit")?,
                u16::try_from(y).map_err(|_| "pixel_limit")?,
            );
            x0 = x0.min(px);
            y0 = y0.min(py);
            x1 = x1.max(px + 1);
            y1 = y1.max(py + 1);
            count += 1;
            for next_y in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for next_x in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let next = next_y * w + next_x;
                    if ink[next] != 0 {
                        ink[next] = 0;
                        pending.push(next);
                    }
                }
            }
        }
        result.push(InkComponent {
            bounds: [x0, y0, x1, y1],
            pixels: count,
        });
    }
    if Instant::now() >= deadline {
        return Err("resource_limit");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::Provenance;
    use std::time::Duration;

    fn page(width: u16, height: u16) -> ReadingPage {
        ReadingPage {
            number: 1,
            width: f32::from(width) * 0.5,
            height: f32::from(height) * 0.5,
            start: 0,
            end: 0,
            provenance: Provenance::Native,
            confidence: None,
        }
    }
    fn pam(width: u16, height: u16, pixels: &[(u16, u16, u8)]) -> Vec<u8> {
        let mut raw = format!(
            "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH 3\nMAXVAL 255\nTUPLTYPE RGB\nENDHDR\n"
        )
        .into_bytes();
        let start = raw.len();
        raw.resize(start + usize::from(width) * usize::from(height) * 3, 255);
        for &(x, y, value) in pixels {
            let position = start + (usize::from(y) * usize::from(width) + usize::from(x)) * 3;
            raw[position..position + 3].fill(value);
        }
        raw
    }
    fn read(raw: &[u8], width: u16, height: u16) -> VectorPageEvidence {
        decode(
            raw,
            &page(width, height),
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap()
    }

    #[test]
    fn should_pin_full_page_renderer_and_limits_when_constructing_the_optional_command() {
        let runtime = MaskRuntime {
            wrapper_path: "/trusted/prlimit".into(),
            wrapper_sha256: "a".repeat(64),
            script_sha256: "b".repeat(64),
        };
        let command = command(
            &runtime,
            Path::new("/trusted/mutool"),
            Path::new("/source.pdf"),
            3,
        );
        assert_eq!(command.as_std().get_program(), "/trusted/prlimit");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            [
                "--as=805306368:805306368",
                "--cpu=5:5",
                "--core=0:0",
                "--fsize=16777216:16777216",
                "--",
                "/trusted/mutool",
                "draw",
                "-F",
                "pam",
                "-c",
                "rgb",
                "-r",
                "144",
                "-A",
                "8",
                "-q",
                "-o",
                "-",
                "/source.pdf",
                "3",
            ]
        );
    }

    #[test]
    fn should_preserve_full_connected_support_when_a_component_crosses_a_future_window() {
        let pixels = (1..15).map(|x| (x, 4, 0)).collect::<Vec<_>>();
        let result = read(&pam(16, 16, &pixels), 16, 16);
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].bounds, [1, 4, 15, 5]);
        assert_eq!(result.components[0].pixels, 14);
        assert_eq!(result.components[0].rect().x_min, 0.5);
        assert_eq!(result.components[0].rect().x_max, 7.5);
    }

    #[test]
    fn should_use_eight_neighbors_when_marks_touch_diagonally() {
        let result = read(&pam(16, 16, &[(1, 1, 0), (2, 2, 0), (8, 8, 0)]), 16, 16);
        assert_eq!(result.components.len(), 2);
        assert_eq!(result.components[0].bounds, [1, 1, 3, 3]);
    }

    #[test]
    fn should_keep_contrast_omissions_explicit_when_paint_is_near_white_or_empty() {
        assert!(read(&pam(8, 8, &[]), 8, 8).components.is_empty());
        let result = read(&pam(8, 8, &[(1, 1, 251), (5, 5, 250)]), 8, 8);
        assert_eq!(result.components.len(), 1);
        assert_eq!(result.components[0].bounds, [5, 5, 6, 6]);
    }

    #[test]
    fn should_reject_raster_bytes_when_samples_dimensions_or_header_are_malformed() {
        let original = pam(8, 8, &[]);
        for raw in [
            original[..original.len() - 1].to_vec(),
            [original.as_slice(), &[0]].concat(),
            original.replace_subslice(b"DEPTH 3", b"DEPTH 4"),
            original.replace_subslice(b"DEPTH 3", b"DEPTH 3\nDEPTH 3"),
            original.replace_subslice(b"WIDTH 8", b"WIDTH 9"),
        ] {
            assert_eq!(
                decode(&raw, &page(8, 8), Instant::now() + Duration::from_secs(1)).unwrap_err(),
                "unsupported_raster"
            );
        }
    }

    #[test]
    fn should_withhold_the_entire_page_when_component_or_pixel_budgets_are_exceeded() {
        let pixels = (0..65)
            .flat_map(|x| (0..65).map(move |y| (x * 2, y * 2, 0)))
            .collect::<Vec<_>>();
        let raw = pam(130, 130, &pixels);
        assert_eq!(
            decode(
                &raw,
                &page(130, 130),
                Instant::now() + Duration::from_secs(2)
            )
            .unwrap_err(),
            "component_limit"
        );
        assert!(dimensions(&page(8192, 8192)).is_none());
        let mut invalid = page(10, 10);
        invalid.width = f32::NAN;
        assert!(dimensions(&invalid).is_none());
    }

    #[test]
    fn should_reject_late_completion_when_the_pixel_deadline_has_expired() {
        assert_eq!(
            decode(&pam(8, 8, &[]), &page(8, 8), Instant::now()).unwrap_err(),
            "resource_limit"
        );
    }

    trait ReplaceSubslice {
        fn replace_subslice(&self, old: &[u8], new: &[u8]) -> Vec<u8>;
    }
    impl ReplaceSubslice for Vec<u8> {
        fn replace_subslice(&self, old: &[u8], new: &[u8]) -> Vec<u8> {
            let position = self
                .windows(old.len())
                .position(|part| part == old)
                .unwrap();
            [&self[..position], new, &self[position + old.len()..]].concat()
        }
    }
}
