//! Logical paragraph membership without reordering the PDF's searchable text.
//! Floats keep their own text objects; a continued paragraph points around them.
use super::{Paragraph, ReadingIndex, ReadingToken, TextRange, native::union};
use crate::domain::TextRect;

struct Fragment {
    first_page: u32,
    last_page: u32,
    rect: TextRect,
    first: TextRect,
    first_font: f32,
    last_font: f32,
    text: String,
    sparse: bool,
}

fn fragment(tokens: &[ReadingToken]) -> Option<Fragment> {
    let first = tokens.first()?;
    let last = tokens.last()?;
    let first_rect = *first.rects.first()?;
    let last_rect = *last.rects.last()?;
    let text = tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let digits = text.chars().filter(char::is_ascii_digit).count();
    let letters = text.chars().filter(|ch| ch.is_alphabetic()).count();
    let wide_cells = tokens.windows(2).any(|pair| {
        let (Some(left), Some(right)) = (pair[0].rects.last(), pair[1].rects.first()) else {
            return false;
        };
        (left.y_min - right.y_min).abs() < 2.0
            && right.x_min - left.x_max > (left.y_max - left.y_min) * 2.5
    });
    // Table values and short chart labels need a caption nearby before being
    // classified as floats. Narrative text is a hard stop when looking upward.
    let sparse =
        digits >= 3 && digits * 3 > letters || wide_cells || letters < 65 && !ends_sentence(&text);
    Some(Fragment {
        first_page: first.page,
        last_page: last.page,
        rect: union(tokens.iter().flat_map(|token| token.rects.iter().copied())),
        first: first_rect,
        first_font: first_rect.y_max - first_rect.y_min,
        last_font: last_rect.y_max - last_rect.y_min,
        text,
        sparse,
    })
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '!', '?', ':', ';'])
}

fn non_body(kind: &str) -> bool {
    matches!(kind, "caption" | "float" | "footnote" | "header")
}

fn classify_floats(index: &mut ReadingIndex, fragments: &[Option<Fragment>]) {
    for (caption_index, caption) in fragments.iter().enumerate() {
        if index.objects.paragraph[caption_index].kind != "caption" {
            continue;
        }
        let Some(caption) = caption else { continue };
        let Some(page) = index
            .pages
            .iter()
            .find(|page| page.number == caption.first_page)
        else {
            continue;
        };
        let mut above = fragments
            .iter()
            .enumerate()
            .filter_map(|(at, candidate)| {
                let candidate = candidate.as_ref()?;
                // A caption's last printed character need not reach the last
                // chart label/table cell. Allow a small horizontal overhang.
                let overhang = caption.first_font * 3.0;
                let overlap = (caption.rect.x_max + overhang).min(candidate.rect.x_max)
                    - (caption.rect.x_min - overhang).max(candidate.rect.x_min);
                (candidate.first_page == caption.first_page
                    && candidate.last_page == candidate.first_page
                    && candidate.rect.y_max <= caption.rect.y_min
                    && caption.rect.y_min - candidate.rect.y_min < page.height * 0.55
                    && overlap > 0.0)
                    .then_some((at, candidate))
            })
            .collect::<Vec<_>>();
        above.sort_by(|left, right| right.1.rect.y_max.total_cmp(&left.1.rect.y_max));
        for (at, candidate) in above {
            let kind = &index.objects.paragraph[at].kind;
            if kind == "float" {
                continue;
            }
            if !matches!(kind.as_str(), "body" | "list") || !candidate.sparse {
                break;
            }
            index.objects.paragraph[at].kind = "float".into();
        }
    }
}

pub(super) fn link_continuations(index: &mut ReadingIndex) {
    let fragments = index
        .objects
        .paragraph
        .iter()
        .map(|paragraph| {
            let start = index
                .tokens
                .partition_point(|token| token.end <= paragraph.start);
            let end = index
                .tokens
                .partition_point(|token| token.start < paragraph.end);
            fragment(&index.tokens[start..end])
        })
        .collect::<Vec<_>>();
    classify_floats(index, &fragments);
    let mut owners = (0..fragments.len()).collect::<Vec<_>>();
    for pages in index.pages.windows(2) {
        if pages[0].number + 1 != pages[1].number
            || pages
                .iter()
                .any(|page| page.provenance == super::Provenance::Unavailable)
        {
            continue;
        }
        let previous = fragments.iter().enumerate().rfind(|(at, fragment)| {
            fragment
                .as_ref()
                .is_some_and(|fragment| fragment.last_page == pages[0].number)
                && !non_body(&index.objects.paragraph[*at].kind)
        });
        let next = fragments.iter().enumerate().find(|(at, fragment)| {
            fragment
                .as_ref()
                .is_some_and(|fragment| fragment.first_page == pages[1].number)
                && !non_body(&index.objects.paragraph[*at].kind)
        });
        let (Some((before, Some(tail))), Some((after, Some(head)))) = (previous, next) else {
            continue;
        };
        // Only the last/first substantive blocks can join. Lowercase alone is
        // insufficient: reject new indents, font changes, and complete sentences.
        if index.objects.paragraph[before].kind != "body"
            || index.objects.paragraph[after].kind != "body"
            || ends_sentence(&tail.text)
            || !head.text.chars().next().is_some_and(char::is_lowercase)
            || (head.first_font - tail.last_font).abs() > tail.last_font * 0.18
            || head.first.x_min - head.rect.x_min > (head.first_font * 0.25).max(1.8)
            || before >= after
        {
            continue;
        }
        let owner = owners[before];
        let continuation = &index.objects.paragraph[after];
        let extra = pieces(continuation);
        let previous = &mut index.objects.paragraph[owner];
        if previous.spans.is_empty() {
            previous.spans = pieces(previous);
        }
        previous.spans.extend(extra);
        previous.end = continuation_end(&previous.spans);
        owners[after] = owner;
    }
    let mut at = 0;
    index.objects.paragraph.retain(|_| {
        let keep = owners[at] == at;
        at += 1;
        keep
    });
}

fn continuation_end(spans: &[TextRange]) -> usize {
    spans.last().map_or(0, |span| span.end)
}

pub(super) fn pieces(paragraph: &Paragraph) -> Vec<TextRange> {
    if paragraph.spans.is_empty() {
        vec![TextRange {
            start: paragraph.start,
            end: paragraph.end,
        }]
    } else {
        paragraph.spans.clone()
    }
}
