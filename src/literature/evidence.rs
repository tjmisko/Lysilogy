//! Adapter from the reading index to the ingest crate's plain-data
//! [`SourceText`]. It copies fields and never re-derives anything, so offsets
//! stay UTF-16 code units of `index.text`.

use lysilogy_ingest::bibliography::{
    PageRect, SourcePage, SourceParagraph, SourceText, SourceToken, TextSpan,
};

use crate::source_index::{ReadingIndex, TextRange};

const fn span(range: &TextRange) -> TextSpan {
    TextSpan {
        start: range.start,
        end: range.end,
    }
}

#[allow(
    dead_code,
    reason = "consumed by the local evidence flow in a later unit"
)]
pub fn source_text_from_index(index: &ReadingIndex) -> SourceText {
    SourceText {
        text: index.text.clone(),
        pages: index
            .pages
            .iter()
            .map(|page| SourcePage {
                number: page.number,
                start: page.start,
                end: page.end,
            })
            .collect(),
        tokens: index
            .tokens
            .iter()
            .map(|token| SourceToken {
                start: token.start,
                end: token.end,
                page: token.page,
                rects: token
                    .rects
                    .iter()
                    .map(|rect| PageRect {
                        x_min: rect.x_min,
                        y_min: rect.y_min,
                        x_max: rect.x_max,
                        y_max: rect.y_max,
                    })
                    .collect(),
                text: token.text.clone(),
            })
            .collect(),
        paragraphs: index
            .objects
            .paragraph
            .iter()
            .map(|paragraph| SourceParagraph {
                start: paragraph.start,
                end: paragraph.end,
                kind: paragraph.kind.clone(),
                spans: paragraph.spans.iter().map(span).collect(),
            })
            .collect(),
        sentences: index.objects.sentence.iter().map(span).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_index::test_support;

    #[test]
    fn should_copy_offsets_and_rects_when_converting_reading_index() {
        let index = test_support::reading_index_fixture();
        let source = source_text_from_index(&index);
        assert_eq!(source.text, index.text);
        assert_eq!(source.pages.len(), index.pages.len());
        for (copied, original) in source.pages.iter().zip(&index.pages) {
            assert_eq!(
                (copied.number, copied.start, copied.end),
                (original.number, original.start, original.end)
            );
        }
        assert_eq!(source.tokens.len(), index.tokens.len());
        for (copied, original) in source.tokens.iter().zip(&index.tokens) {
            assert_eq!(
                (copied.start, copied.end, copied.page, &copied.text),
                (original.start, original.end, original.page, &original.text)
            );
            assert_eq!(copied.rects.len(), original.rects.len());
            for (copied, original) in copied.rects.iter().zip(&original.rects) {
                let bits = |values: [f32; 4]| values.map(f32::to_bits);
                assert_eq!(
                    bits([copied.x_min, copied.y_min, copied.x_max, copied.y_max]),
                    bits([
                        original.x_min,
                        original.y_min,
                        original.x_max,
                        original.y_max
                    ])
                );
            }
        }
        assert_eq!(source.paragraphs.len(), index.objects.paragraph.len());
        assert_eq!(source.paragraphs[0].kind, "heading");
        assert_eq!(source.paragraphs[1].spans.len(), 2);
        assert_eq!(source.paragraphs[1].spans[1].start, 20);
        assert_eq!(source.sentences.len(), index.objects.sentence.len());
        assert_eq!(source.sentences[1].end, index.objects.sentence[1].end);
        // Astral text keeps UTF-16 offsets: the token after the emoji starts at 3.
        assert_eq!(source.tokens[1].start, 3);
    }

    #[test]
    fn should_return_an_empty_source_when_the_index_is_empty() {
        let index = test_support::reading_index_fixture_empty();
        let source = source_text_from_index(&index);
        assert!(source.text.is_empty());
        assert!(source.tokens.is_empty() && source.pages.is_empty());
        assert!(source.paragraphs.is_empty() && source.sentences.is_empty());
    }
}
