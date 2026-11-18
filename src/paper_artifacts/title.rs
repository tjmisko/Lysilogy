//! A deterministic title candidate from the geometry of page 1.
//!
//! The candidate is a hint for the model, not an answer: the largest
//! horizontal text in the upper part of the first page, with adjacent lines
//! of the same size joined so a two-line title comes back whole. Rotated
//! margin stamps (arXiv prints its identifier vertically down the left edge),
//! drop caps, and cite-as banners are ignored, because each of them was the
//! tallest "line" on some real first page.

use crate::domain::{LayoutPage, TextRect};

/// Only lines that start above this fraction of the page height qualify.
const UPPER_FRACTION: f32 = 0.55;

/// Lines whose box lies entirely within this fraction of either side edge
/// are margin furniture, not a heading.
const EDGE_FRACTION: f32 = 0.08;

/// Lines this much smaller than the tallest line still count as the same size.
const SIZE_TOLERANCE: f32 = 0.12;

/// The largest vertical gap, in line heights, between two lines of one title.
const LINE_GAP: f32 = 1.0;

/// Lines with this many visible characters or fewer are drop caps or marks.
const MIN_CHARS: usize = 3;

/// One printed line: its words in reading order, its box and its type size.
#[derive(Clone, Debug)]
struct Line {
    text: String,
    bounds: TextRect,
    size: f32,
}

/// The likely title of a paper from its first layout page, or `None` when no
/// line qualifies.
#[must_use]
pub fn likely_title(page: &LayoutPage) -> Option<String> {
    let mut lines = candidate_lines(page);
    lines.sort_by(|left, right| left.bounds.y_min.total_cmp(&right.bounds.y_min));
    let anchor = lines
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            // Ties go to the topmost line.
            left.size
                .total_cmp(&right.size)
                .then_with(|| right.bounds.y_min.total_cmp(&left.bounds.y_min))
        })
        .map(|(index, _)| index)?;
    let size = lines[anchor].size;
    let same_size = |line: &Line| line.size >= size * (1.0 - SIZE_TOLERANCE);
    // The white space between one line's bottom and the next line's top.
    let close = |upper: &Line, lower: &Line| {
        let gap = lower.bounds.y_min - upper.bounds.y_max;
        gap <= size * LINE_GAP
    };
    let mut first = anchor;
    while first > 0 && same_size(&lines[first - 1]) && close(&lines[first - 1], &lines[first]) {
        first -= 1;
    }
    let mut last = anchor;
    while last + 1 < lines.len()
        && same_size(&lines[last + 1])
        && close(&lines[last], &lines[last + 1])
    {
        last += 1;
    }
    let title = lines[first..=last]
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// Whether a short string is an identifier or a stamp rather than a title:
/// an arXiv stamp, a DOI, an ISSN or ISBN, or a bare number.
#[must_use]
pub fn looks_like_identifier(text: &str) -> bool {
    let lowered = text.trim().to_lowercase();
    if lowered.starts_with("arxiv:")
        || lowered.starts_with("doi:")
        || lowered.starts_with("issn")
        || lowered.starts_with("isbn")
        || lowered.starts_with("https://doi.org/")
        || lowered.starts_with("http://")
        || lowered.starts_with("https://")
    {
        return true;
    }
    // A DOI such as 10.1257/aer.20131234.
    if lowered.starts_with("10.") && lowered.contains('/') && !lowered.contains(' ') {
        return true;
    }
    // A bare arXiv identifier such as 1706.03762v5 or hep-th/9901001.
    let digits = lowered.chars().filter(char::is_ascii_digit).count();
    let letters = lowered.chars().filter(|c| c.is_alphabetic()).count();
    !lowered.is_empty() && digits >= 4 && digits > letters * 2
}

/// The page's horizontal lines in the upper part, away from the edges, with
/// enough characters to be a heading and no cite-as or stamp wording.
fn candidate_lines(page: &LayoutPage) -> Vec<Line> {
    let mut grouped = std::collections::BTreeMap::<u32, Vec<(&str, TextRect)>>::new();
    for token in &page.tokens {
        if let Some(rect) = token.rects.first() {
            grouped
                .entry(token.line)
                .or_default()
                .push((token.text.as_str(), *rect));
        }
    }
    grouped
        .into_values()
        .filter_map(|mut words| {
            words.sort_by(|left, right| left.1.x_min.total_cmp(&right.1.x_min));
            let bounds = union(words.iter().map(|(_, rect)| *rect))?;
            let size = words
                .iter()
                .map(|(_, rect)| rect.y_max - rect.y_min)
                .fold(0.0_f32, f32::max);
            let text = words
                .iter()
                .map(|(text, _)| *text)
                .collect::<Vec<_>>()
                .join(" ");
            Some(Line { text, bounds, size })
        })
        .filter(|line| qualifies(line, page))
        .collect()
}

fn qualifies(line: &Line, page: &LayoutPage) -> bool {
    let bounds = line.bounds;
    let width = bounds.x_max - bounds.x_min;
    let height = bounds.y_max - bounds.y_min;
    // Rotated text stacks its words vertically: taller than it is wide.
    let vertical = height > width;
    let near_edge = bounds.x_max <= page.width * EDGE_FRACTION
        || bounds.x_min >= page.width * (1.0 - EDGE_FRACTION);
    let upper = bounds.y_min <= page.height * UPPER_FRACTION;
    let visible = line.text.chars().filter(|c| !c.is_whitespace()).count();
    let lowered = line.text.to_lowercase();
    let banner = lowered.contains("cite")
        || lowered.contains("accepted for publication")
        || looks_like_identifier(&line.text);
    !vertical && !near_edge && upper && visible >= MIN_CHARS && !banner && line.size > 0.0
}

fn union(rects: impl Iterator<Item = TextRect>) -> Option<TextRect> {
    rects.reduce(|left, right| TextRect {
        x_min: left.x_min.min(right.x_min),
        y_min: left.y_min.min(right.y_min),
        x_max: left.x_max.max(right.x_max),
        y_max: left.y_max.max(right.y_max),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LayoutToken;

    /// A US-letter page built from `(line, text, x_min, y_min, x_max, y_max)`.
    fn page(words: &[(u32, &str, f32, f32, f32, f32)]) -> LayoutPage {
        LayoutPage {
            number: 1,
            width: 612.0,
            height: 792.0,
            tokens: words
                .iter()
                .enumerate()
                .map(
                    |(index, (line, text, x_min, y_min, x_max, y_max))| LayoutToken {
                        index: u32::try_from(index).expect("index"),
                        text: (*text).to_owned(),
                        line: *line,
                        rects: vec![TextRect {
                            x_min: *x_min,
                            y_min: *y_min,
                            x_max: *x_max,
                            y_max: *y_max,
                        }],
                    },
                )
                .collect(),
            sentences: Vec::new(),
        }
    }

    #[test]
    fn should_ignore_the_rotated_arxiv_stamp_in_the_left_margin() {
        let page = page(&[
            // The stamp: words stacked down the left edge, each taller than wide.
            (0, "arXiv:1706.03762v5", 12.0, 220.0, 32.0, 420.0),
            (0, "[cs.CL]", 12.0, 430.0, 32.0, 500.0),
            (0, "6", 12.0, 510.0, 32.0, 520.0),
            (0, "Dec", 12.0, 530.0, 32.0, 560.0),
            (0, "2017", 12.0, 570.0, 32.0, 610.0),
            (1, "Attention", 220.0, 100.0, 300.0, 117.0),
            (1, "Is", 305.0, 100.0, 320.0, 117.0),
            (1, "All", 325.0, 100.0, 350.0, 117.0),
            (1, "You", 355.0, 100.0, 385.0, 117.0),
            (1, "Need", 390.0, 100.0, 430.0, 117.0),
            (2, "Ashish", 150.0, 150.0, 190.0, 160.0),
            (2, "Vaswani", 195.0, 150.0, 240.0, 160.0),
        ]);
        assert_eq!(
            likely_title(&page).as_deref(),
            Some("Attention Is All You Need")
        );
    }

    #[test]
    fn should_ignore_a_horizontal_stamp_hugging_the_page_edge() {
        let page = page(&[
            (0, "arXiv", 2.0, 40.0, 40.0, 70.0),
            (1, "Scaling", 200.0, 100.0, 260.0, 116.0),
            (1, "Laws", 265.0, 100.0, 305.0, 116.0),
        ]);
        assert_eq!(likely_title(&page).as_deref(), Some("Scaling Laws"));
    }

    #[test]
    fn should_join_a_title_printed_on_two_lines() {
        let page = page(&[
            (0, "Journal", 250.0, 30.0, 290.0, 38.0),
            (0, "of", 292.0, 30.0, 302.0, 38.0),
            (1, "The", 160.0, 100.0, 195.0, 117.0),
            (1, "China", 200.0, 100.0, 250.0, 117.0),
            (1, "Syndrome:", 255.0, 100.0, 340.0, 117.0),
            (1, "Local", 345.0, 100.0, 390.0, 117.0),
            (1, "Labor", 395.0, 100.0, 440.0, 117.0),
            (2, "Market", 200.0, 121.0, 260.0, 137.5),
            (2, "Effects", 265.0, 121.0, 320.0, 137.5),
            (3, "David", 220.0, 170.0, 255.0, 180.0),
            (3, "Autor", 260.0, 170.0, 295.0, 180.0),
        ]);
        assert_eq!(
            likely_title(&page).as_deref(),
            Some("The China Syndrome: Local Labor Market Effects")
        );
    }

    #[test]
    fn should_not_return_a_drop_cap() {
        let page = page(&[
            (0, "A", 150.0, 60.0, 172.0, 80.0),
            (0, "Mathematical", 175.0, 60.0, 300.0, 80.0),
            (0, "Theory", 305.0, 60.0, 370.0, 80.0),
            (1, "By", 260.0, 100.0, 275.0, 110.0),
            (1, "C.", 280.0, 100.0, 290.0, 110.0),
            (1, "E.", 292.0, 100.0, 302.0, 110.0),
            (1, "SHANNON", 305.0, 100.0, 360.0, 110.0),
            // A drop cap opening the first paragraph, far taller than the title.
            (2, "T", 72.0, 300.0, 110.0, 345.0),
            (3, "HE", 112.0, 300.0, 130.0, 310.0),
            (3, "recent", 133.0, 300.0, 170.0, 310.0),
        ]);
        assert_eq!(
            likely_title(&page).as_deref(),
            Some("A Mathematical Theory")
        );
    }

    #[test]
    fn should_skip_cite_as_banners_and_text_below_the_upper_part() {
        let page = page(&[
            (0, "Please", 100.0, 30.0, 140.0, 50.0),
            (0, "cite", 145.0, 30.0, 175.0, 50.0),
            (0, "as", 180.0, 30.0, 195.0, 50.0),
            (1, "Resilient", 180.0, 100.0, 260.0, 116.0),
            (1, "Maximization", 265.0, 100.0, 380.0, 116.0),
            // A large heading low on the page, e.g. a journal footer.
            (2, "SPRINGER", 200.0, 700.0, 400.0, 740.0),
        ]);
        assert_eq!(
            likely_title(&page).as_deref(),
            Some("Resilient Maximization")
        );
    }

    /// Poppler's `-bbox-layout` output for a generated page with a rotated
    /// arXiv stamp, a 17pt title, a byline and a 40pt drop cap.
    const POPPLER_PAGE: &str = r#"<page width="612.000000" height="792.000000">
<line xMin="200.0" yMin="100.389" xMax="387.935" yMax="115.689">
<word xMin="200.000000" yMin="100.389000" xMax="268.935000" yMax="115.689000">Attention</word>
<word xMin="273.185000" yMin="100.389000" xMax="286.411000" yMax="115.689000">Is</word>
<word xMin="290.661000" yMin="100.389000" xMax="312.387000" yMax="115.689000">All</word>
<word xMin="316.637000" yMin="100.389000" xMax="346.863000" yMax="115.689000">You</word>
<word xMin="351.113000" yMin="100.389000" xMax="387.935000" yMax="115.689000">Need</word>
</line>
<line xMin="16.34" yMin="213.16" xMax="34.34" yMax="542.0">
<word xMin="16.340000" yMin="375.900000" xMax="34.340000" yMax="542.000000">arXiv:1706.03762v5</word>
<word xMin="16.340000" yMin="310.360000" xMax="34.340000" yMax="370.900000">[cs.CL]</word>
<word xMin="16.340000" yMin="295.360000" xMax="34.340000" yMax="305.360000">6</word>
<word xMin="16.340000" yMin="258.160000" xMax="34.340000" yMax="290.360000">Dec</word>
<word xMin="16.340000" yMin="213.160000" xMax="34.340000" yMax="253.160000">2017</word>
</line>
<line xMin="200.0" yMin="134.487" xMax="271.797" yMax="144.387">
<word xMin="200.000000" yMin="134.487000" xMax="230.558000" yMax="144.387000">Ashish</word>
<word xMin="233.308000" yMin="134.487000" xMax="271.797000" yMax="144.387000">Vaswani</word>
</line>
<line xMin="72.0" yMin="364.68" xMax="96.44" yMax="400.68">
<word xMin="72.000000" yMin="364.680000" xMax="96.440000" yMax="400.680000">T</word>
</line>
<line xMin="100.0" yMin="385.17" xMax="188.87" yMax="394.17">
<word xMin="100.000000" yMin="385.170000" xMax="109.440000" yMax="394.170000">he</word>
<word xMin="111.940000" yMin="385.170000" xMax="149.720000" yMax="394.170000">dominant</word>
</line>
</page>"#;

    #[test]
    fn should_find_the_title_in_real_poppler_output_with_a_stamp_and_a_drop_cap() {
        let layout = crate::layout::parse_bbox_layout(POPPLER_PAGE).expect("layout");
        assert_eq!(
            likely_title(&layout.pages[0]).as_deref(),
            Some("Attention Is All You Need")
        );
    }

    #[test]
    fn should_return_nothing_for_an_empty_page() {
        assert_eq!(likely_title(&page(&[])), None);
    }

    #[test]
    fn should_recognize_identifiers_and_stamps() {
        for stamp in [
            "arXiv:1706.03762v5 [cs.CL] 6 Dec 2017",
            "10.1257/aer.103.6.2121",
            "doi:10.1000/182",
            "1706.03762v5",
            "https://example.org/paper",
        ] {
            assert!(looks_like_identifier(stamp), "{stamp}");
        }
        for title in [
            "Attention Is All You Need",
            "The Mariel Boatlift",
            "GPT-4 Technical Report",
        ] {
            assert!(!looks_like_identifier(title), "{title}");
        }
    }
}
