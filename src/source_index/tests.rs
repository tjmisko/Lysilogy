use super::*;

fn word(text: &str, line: u32, block: u32, x: f32, y: f32) -> SourceWord {
    SourceWord {
        text: text.into(),
        line,
        block,
        rect: TextRect {
            x_min: x,
            y_min: y,
            x_max: x + 40.0,
            y_max: y + 10.0,
        },
    }
}

fn page(words: Vec<SourceWord>) -> SourcePage {
    SourcePage {
        number: 1,
        width: 600.0,
        height: 800.0,
        words,
        provenance: Provenance::Native,
        confidence: None,
    }
}

fn prose_line(text: &str, line: u32, x: f32, y: f32) -> SourceWord {
    let mut word = word(text, line, 0, x, y);
    word.rect.x_max = x + 225.0;
    word
}

fn paragraph_texts(index: &ReadingIndex) -> Vec<String> {
    index
        .objects
        .paragraph
        .iter()
        .map(|paragraph| {
            paragraphs::pieces(paragraph)
                .iter()
                .map(|span| figures::utf16_slice(&index.text, span.start, span.end))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn modest_first_line_indents_split_successive_paragraphs_inside_one_poppler_block() {
    let index = assemble(&[page(vec![
        prose_line(
            "First paragraph begins with a modest indent",
            0,
            54.0,
            100.0,
        ),
        prose_line(
            "and continues at the body margin. A new sentence",
            1,
            50.0,
            114.0,
        ),
        prose_line(
            "on the next line is still the same paragraph.",
            2,
            50.0,
            128.0,
        ),
        prose_line(
            "Second paragraph has no extra vertical space",
            3,
            54.0,
            142.0,
        ),
        prose_line("and ends at exactly the same body margin.", 4, 50.0, 156.0),
        prose_line(
            "optimization can begin a lowercase paragraph",
            5,
            54.0,
            170.0,
        ),
        prose_line(
            "without being mistaken for a hanging continuation.",
            6,
            50.0,
            184.0,
        ),
    ])]);
    assert_eq!(
        paragraph_texts(&index),
        vec![
            "First paragraph begins with a modest indent and continues at the body margin. A new sentence on the next line is still the same paragraph.",
            "Second paragraph has no extra vertical space and ends at exactly the same body margin.",
            "optimization can begin a lowercase paragraph without being mistaken for a hanging continuation.",
        ]
    );
}

#[test]
fn local_margins_keep_two_columns_separate_and_split_paragraphs_in_each() {
    let index = assemble(&[page(vec![
        prose_line("The first column has a paragraph", 0, 54.0, 100.0),
        prose_line("that continues on a second line.", 1, 50.0, 114.0),
        prose_line("Another left column paragraph follows", 2, 54.0, 128.0),
        prose_line("with no blank vertical line.", 3, 50.0, 142.0),
        prose_line("The right column has its own margin", 4, 334.0, 100.0),
        prose_line("that is independent of the left column.", 5, 330.0, 114.0),
        prose_line("A second right paragraph begins here", 6, 334.0, 128.0),
        prose_line("and continues to the column margin.", 7, 330.0, 142.0),
    ])]);
    let paragraphs = paragraph_texts(&index);
    assert_eq!(paragraphs.len(), 4, "{paragraphs:?}");
    assert!(paragraphs[1].starts_with("Another left"));
    assert!(paragraphs[2].starts_with("The right"));
    assert!(paragraphs[3].starts_with("A second right"));
}

#[test]
fn body_paragraph_after_a_hanging_definition_uses_body_margin_instead_of_previous_line() {
    let index = assemble(&[page(vec![
        prose_line(
            "Goodhart definition - When selecting a proxy, select not",
            0,
            50.0,
            100.0,
        ),
        prose_line(
            "only the true goal but also the error of the proxy",
            1,
            75.0,
            114.0,
        ),
        prose_line(
            "which leads to a different optimization result.",
            2,
            75.0,
            128.0,
        ),
        prose_line(
            "optimization can then introduce a new paragraph",
            3,
            54.0,
            142.0,
        ),
        prose_line("that should be selected on its own.", 4, 50.0, 156.0),
    ])]);
    let paragraphs = paragraph_texts(&index);
    assert_eq!(paragraphs.len(), 2, "{paragraphs:?}");
    assert!(paragraphs[0].contains("select not only the true goal"));
    assert!(paragraphs[0].ends_with("optimization result."));
    assert!(paragraphs[1].starts_with("optimization can then"));
}

#[test]
fn blank_lines_are_measured_against_local_leading_in_double_spaced_prose() {
    let index = assemble(&[page(vec![
        prose_line("The first paragraph uses double spacing", 0, 50.0, 100.0),
        prose_line(
            "and wraps normally despite larger line gaps",
            1,
            50.0,
            122.0,
        ),
        prose_line("until the actual paragraph ending.", 2, 50.0, 144.0),
        prose_line(
            "An additional blank line starts this paragraph",
            3,
            50.0,
            176.0,
        ),
        prose_line("which uses the same double spaced leading", 4, 50.0, 198.0),
        prose_line("and should remain one complete paragraph.", 5, 50.0, 220.0),
    ])]);
    let paragraphs = paragraph_texts(&index);
    assert_eq!(paragraphs.len(), 2, "{paragraphs:?}");
    assert!(paragraphs[1].starts_with("An additional blank line"));
}

#[test]
fn headings_and_hanging_lists_do_not_absorb_neighboring_body_paragraphs() {
    let index = assemble(&[page(vec![
        prose_line("The first prose paragraph ends here.", 0, 50.0, 100.0),
        prose_line("2 Related work", 1, 50.0, 114.0),
        prose_line("Our research follows these earlier ideas.", 2, 50.0, 128.0),
        prose_line("1. A list item begins and wraps onto", 3, 50.0, 142.0),
        prose_line("another line with a hanging indent.", 4, 60.0, 156.0),
        prose_line("The body paragraph returns to its margin", 5, 50.0, 170.0),
        prose_line(
            "and does not inherit the preceding list kind.",
            6,
            50.0,
            184.0,
        ),
    ])]);
    assert_eq!(
        index
            .objects
            .paragraph
            .iter()
            .map(|paragraph| paragraph.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["body", "heading", "body", "list", "body"]
    );
    assert!(paragraph_texts(&index)[3].ends_with("hanging indent."));
    assert!(paragraph_texts(&index)[4].starts_with("The body paragraph"));
}

#[test]
fn wraps_words_and_preserves_utf16_geometry_across_ligatures_and_hyphens() {
    let page = page(vec![
        word("The", 0, 0, 50.0, 50.0),
        word("efﬁcient", 0, 0, 100.0, 50.0),
        word("opti-", 0, 0, 150.0, 50.0),
        word("mization", 1, 0, 50.0, 64.0),
        word("😀", 1, 0, 100.0, 64.0),
        word("works.", 1, 0, 150.0, 64.0),
    ]);
    let index = assemble(&[page]);
    assert_eq!(index.text, "The efficient optimization 😀 works.");
    assert_eq!(index.objects.paragraph.len(), 1);
    for token in &index.tokens {
        assert_eq!(
            figures::utf16_slice(&index.text, token.start, token.end),
            token.text
        );
    }
    let optimization = index
        .objects
        .word
        .iter()
        .find(|range| figures::utf16_slice(&index.text, range.start, range.end) == "optimization")
        .unwrap();
    let covered = index
        .tokens
        .iter()
        .filter(|token| token.start < optimization.end && token.end > optimization.start)
        .collect::<Vec<_>>();
    assert_eq!(covered.len(), 2);
    assert!((covered[0].rects[0].y_min - covered[1].rects[0].y_min).abs() > 1.0);
    assert_eq!(index.objects.sentence.len(), 1);
}

#[test]
fn distinct_columns_indentation_footnotes_and_folios_do_not_merge() {
    let mut note = word("1 Footnote.", 4, 2, 50.0, 720.0);
    note.rect.y_max = 727.0;
    let page = page(vec![
        word("Left column starts", 0, 0, 50.0, 100.0),
        word("and continues.", 1, 0, 50.0, 114.0),
        word("New paragraph.", 2, 0, 65.0, 128.0),
        word("Right column.", 3, 1, 330.0, 100.0),
        note,
        word("1", 5, 3, 300.0, 770.0),
    ]);
    let index = assemble(&[page]);
    assert!(
        index
            .text
            .starts_with("Left column starts and continues.\n\nNew paragraph.\n\nRight column.")
    );
    assert_eq!(index.objects.paragraph.last().unwrap().kind, "footnote");
    assert!(!index.text.ends_with("\n\n1"));
    assert_eq!(index.objects.paragraph.len(), 4);
}

#[test]
fn hanging_definition_indent_keeps_the_complete_paragraph() {
    let index = assemble(&[page(vec![
        word(
            "Regressional Goodhart - When selecting a proxy, select not",
            0,
            0,
            133.8,
            364.0,
        ),
        word(
            "only the goal, but also the proxy error.",
            1,
            0,
            158.6,
            376.0,
        ),
    ])]);
    assert_eq!(index.objects.paragraph.len(), 1);
    assert!(index.text.contains("select not only the goal"));
}

#[test]
fn native_block_structure_retains_a_complete_real_abstract() {
    let pages =
        native::parse(include_str!("../../tests/fixtures/debate-frontmatter.html")).unwrap();
    let index = assemble(&pages.pages);
    let paragraph = index
        .objects
        .paragraph
        .iter()
        .find(|paragraph| {
            figures::utf16_slice(&index.text, paragraph.start, paragraph.end)
                .starts_with("To make AI systems")
        })
        .unwrap();
    let text = figures::utf16_slice(&index.text, paragraph.start, paragraph.end);
    assert!(text.contains("89%") || text.contains("88.9%"));
    assert!(text.ends_with("these properties."), "{text}");
    assert!(!text.contains("arXiv"));
}

#[test]
fn wrapped_dash_variants_continue_prose_and_keep_the_next_indent_separate() {
    for dash in ["-", "–", "—"] {
        let index = assemble(&[page(vec![
            prose_line("Some relationships remain implicit", 0, 50.0, 100.0),
            prose_line(
                &format!("{dash} others are encoded in the hierarchy."),
                1,
                50.0,
                114.0,
            ),
            prose_line("A different paragraph begins here", 2, 54.0, 128.0),
            prose_line("and continues at the body margin.", 3, 50.0, 142.0),
        ])]);
        let paragraphs = paragraph_texts(&index);
        assert_eq!(paragraphs.len(), 2, "{dash}: {paragraphs:?}");
        assert!(paragraphs[0].ends_with("encoded in the hierarchy."));
        assert!(paragraphs[1].starts_with("A different paragraph"));
        assert!(
            index
                .objects
                .paragraph
                .iter()
                .all(|span| span.kind == "body")
        );
    }
}

#[test]
fn real_dash_lists_keep_their_boundaries() {
    for marker in ["-", "–", "—", "•", "●"] {
        let index = assemble(&[page(vec![
            prose_line("We evaluate two distinct settings:", 0, 50.0, 100.0),
            prose_line(&format!("{marker} The first setting uses"), 1, 50.0, 114.0),
            prose_line("a hanging continuation on this line.", 2, 60.0, 128.0),
            prose_line(
                &format!("{marker} The second setting differs."),
                3,
                50.0,
                142.0,
            ),
        ])]);
        let kinds = index
            .objects
            .paragraph
            .iter()
            .map(|span| span.kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["body", "list", "list"],
            "{marker}: {:?}",
            paragraph_texts(&index)
        );
    }
    let index = assemble(&[page(vec![
        prose_line("An unfinished lead-in introduces", 0, 50.0, 100.0),
        prose_line("— a first parallel item", 1, 50.0, 114.0),
        prose_line("— a second parallel item", 2, 50.0, 128.0),
    ])]);
    assert_eq!(index.objects.paragraph.len(), 3);
    assert_eq!(index.objects.paragraph[1].kind, "list");
    assert_eq!(index.objects.paragraph[2].kind, "list");
}

#[test]
fn native_dash_wrapped_hierarchy_paragraph_reaches_desai_citation() {
    let pages = native::parse(include_str!(
        "../../tests/fixtures/hierarchy-introduction.html"
    ))
    .unwrap();
    let index = assemble(&pages.pages);
    let paragraphs = paragraph_texts(&index);
    let humans = index.text[..index.text.find("humans").unwrap()]
        .encode_utf16()
        .count();
    let paragraph = index
        .objects
        .paragraph
        .iter()
        .find(|span| span.start <= humans && span.end > humans)
        .unwrap();
    let selected = figures::utf16_slice(&index.text, paragraph.start, paragraph.end);
    assert!(selected.starts_with("As humans,"), "{selected}");
    assert!(selected.contains("explicit — ImageNet"), "{selected}");
    assert!(selected.ends_with("(Desai et al., 2021)."), "{selected}");
    assert_eq!(paragraphs.len(), 4, "{paragraphs:#?}");
    assert!(paragraphs[1].starts_with("As a result,"));
    assert!(paragraphs[2].starts_with("The task of learning"));
    assert!(paragraphs[3].starts_with("In this paper,"));
    assert!(
        index
            .objects
            .paragraph
            .iter()
            .all(|span| span.kind == "body")
    );
}

#[test]
fn real_goodhart_prose_definition_and_numbered_footnotes_have_independent_boundaries() {
    let pages = native::parse(include_str!(
        "../../tests/fixtures/goodhart-paragraphs.html"
    ))
    .unwrap();
    let index = assemble(&pages.pages);
    let paragraphs = paragraph_texts(&index);
    assert_eq!(paragraphs.len(), 6, "{paragraphs:#?}");
    assert!(paragraphs[0].starts_with("proxy necessarily"));
    assert!(paragraphs[0].ends_with("subcategories which differ in important ways."));
    assert!(paragraphs[1].starts_with("To formalize the intuitive description"));
    assert!(paragraphs[2].starts_with("Regressional Goodhart - When selecting"));
    assert!(paragraphs[2].contains("select not only for the true goal"));
    assert!(paragraphs[2].ends_with("“Tails come apart.” [5]"));
    assert!(paragraphs[3].starts_with("Due to the noise"));
    assert!(paragraphs[4].starts_with("2 In general"));
    assert!(paragraphs[5].starts_with("3 This restriction"));
    assert_eq!(index.objects.paragraph[4].kind, "footnote");
    assert_eq!(index.objects.paragraph[5].kind, "footnote");
}

#[test]
fn ocr_coordinates_map_from_pixels_and_drop_low_confidence_noise() {
    let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n1\t1\t0\t0\t0\t0\t0\t0\t1200\t1600\t-1\t\n5\t1\t1\t1\t1\t1\t100\t200\t80\t20\t95\tOptimization\n5\t1\t1\t1\t1\t2\t200\t200\t80\t20\t12\tnoise\n";
    let ocr = ocr::parse_tsv(tsv, &page(Vec::new())).unwrap();
    assert_eq!(ocr.words.len(), 1);
    assert_eq!(
        ocr.words[0].rect,
        TextRect {
            x_min: 50.0,
            y_min: 100.0,
            x_max: 90.0,
            y_max: 110.0
        }
    );
    assert_eq!(ocr.provenance, Provenance::Ocr);
    assert!(
        ocr.confidence
            .is_some_and(|confidence| (confidence - 0.95).abs() < 0.001)
    );
}

#[test]
fn figures_resolve_outside_the_referencing_page_and_keep_unresolved_geometry_explicit() {
    let first = page(vec![
        word("See", 0, 0, 50.0, 100.0),
        word("Figure", 0, 0, 100.0, 100.0),
        word("2", 0, 0, 150.0, 100.0),
        word("for results.", 0, 0, 200.0, 100.0),
    ]);
    let mut second = page(vec![
        word("Figure", 0, 0, 50.0, 400.0),
        word("2:", 0, 0, 100.0, 400.0),
        word("Optimization", 0, 0, 150.0, 400.0),
        word("results.", 1, 0, 50.0, 414.0),
    ]);
    second.number = 2;
    let mut index = assemble(&[first, second]);
    index.figures = figures::find(&index);
    assert_eq!(index.figures.len(), 1);
    let figure = &index.figures[0];
    assert_eq!(figure.label, "Figure 2");
    assert_eq!(figure.page, 2);
    assert_eq!(figure.references.len(), 1);
    assert_eq!(figure.references[0].page, 1);
    assert_eq!(figure.caption, "Figure 2: Optimization results.");
    assert_eq!(figure.confidence, "candidate");
}

#[test]
fn figure_ranges_resolve_each_number_and_body_mentions_are_not_captions() {
    let mut pages = vec![page(vec![
        word("Figures", 0, 0, 50.0, 100.0),
        word("1–3", 0, 0, 100.0, 100.0),
        word("show the results.", 0, 0, 150.0, 100.0),
    ])];
    for number in 1..=3 {
        let mut caption = page(vec![
            word("Figure", 0, 0, 50.0, 300.0),
            word(&format!("{number}:"), 0, 0, 100.0, 300.0),
            word("Results.", 0, 0, 150.0, 300.0),
        ]);
        caption.number = number + 1;
        pages.push(caption);
    }
    let index = assemble(&pages);
    let figures = figures::find(&index);
    assert_eq!(figures.len(), 3);
    assert!(
        figures
            .iter()
            .all(|figure| figure.references.len() == 1 && figure.references[0].page == 1)
    );
    let prose = assemble(&[page(vec![
        word("Figure", 0, 0, 50.0, 100.0),
        word("2", 0, 0, 100.0, 100.0),
        word("supports this result.", 0, 0, 150.0, 100.0),
    ])]);
    assert_eq!(prose.objects.paragraph[0].kind, "body");
}

#[test]
fn word_and_big_word_distinguish_scientific_punctuation() {
    let index = assemble(&[page(vec![word("loss(x)", 0, 0, 50.0, 100.0)])]);
    assert_eq!(index.objects.word.len(), 4);
    assert_eq!(index.objects.big_word.len(), 1);
    assert_eq!(
        figures::utf16_slice(
            &index.text,
            index.objects.word[2].start,
            index.objects.word[2].end
        ),
        "x"
    );
}

#[test]
fn clear_prose_continuation_crosses_a_page_but_does_not_include_footnotes() {
    let first = page(vec![word("We optimize the", 0, 0, 50.0, 700.0)]);
    let mut second = page(vec![word(
        "objective with gradient descent.",
        0,
        0,
        50.0,
        60.0,
    )]);
    second.number = 2;
    let index = assemble(&[first.clone(), second.clone()]);
    assert_eq!(
        index.text,
        "We optimize the objective with gradient descent."
    );
    assert_eq!(index.objects.paragraph.len(), 1);
    assert_eq!(index.objects.sentence.len(), 1);
    let mut footnoted = first;
    let mut note = word("1 Additional details.", 1, 1, 50.0, 730.0);
    note.rect.y_max = 737.0;
    footnoted.words.push(note);
    assert!(assemble(&[footnoted, second]).objects.paragraph.len() > 1);
}

#[tokio::test]
async fn bounded_command_rejects_excess_output() {
    let mut command = Command::new("printf");
    command.arg("too much output");
    let error = bounded_command(&mut command, "printf", 4)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("bounded limit"));
}

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    use std::fmt::Write;
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        writeln!(tail, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        tail,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    bytes.extend_from_slice(tail.as_bytes());
    bytes
}

fn stream(dictionary: &str, content: &[u8]) -> Vec<u8> {
    let mut result = format!("<< {dictionary} /Length {} >>\nstream\n", content.len()).into_bytes();
    result.extend_from_slice(content);
    result.extend_from_slice(b"\nendstream");
    result
}

#[tokio::test]
async fn real_image_only_pdf_uses_local_ocr_then_the_source_keyed_cache() {
    // This is an executable integration test when the optional local OCR stack exists.
    for program in ["pdftotext", "pdftoppm", "tesseract"] {
        if Command::new(program).arg("-v").output().await.is_err() {
            return;
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let native_path = directory.path().join("native.pdf");
    let source = b"BT /F1 22 Tf 45 720 Td (Optimization improves the objective.) Tj 0 -32 Td (This paragraph is visible in the scanned image.) Tj 0 -32 Td (A source index should preserve its coordinates.) Tj ET";
    let objects = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(), b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(), b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(), stream("", source)];
    tokio::fs::write(&native_path, pdf(&objects)).await.unwrap();
    let prefix = directory.path().join("scan");
    let mut render = Command::new("pdftoppm");
    render
        .args(["-singlefile", "-scale-to", "1600", "-gray"])
        .arg(&native_path)
        .arg(&prefix);
    bounded_command(&mut render, "pdftoppm", 1024)
        .await
        .unwrap();
    let image = tokio::fs::read(prefix.with_extension("pgm")).await.unwrap();
    let mut cursor = 0;
    let mut header = Vec::<String>::new();
    while header.len() < 4 {
        if image[cursor] == b'#' {
            while image[cursor] != b'\n' {
                cursor += 1;
            }
        }
        if image[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while !image[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        header.push(String::from_utf8_lossy(&image[start..cursor]).into_owned());
    }
    cursor += 1;
    assert_eq!(header[0], "P5");
    let scan_path = directory.path().join("image-only.pdf");
    let objects = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(), b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>".to_vec(), stream(&format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8", header[1], header[2]), &image[cursor..]), stream("", b"q 600 0 0 800 0 0 cm /Im1 Do Q")];
    tokio::fs::write(&scan_path, pdf(&objects)).await.unwrap();
    let cache = directory.path().join("cache");
    let index = load_or_build(&scan_path, &cache, false).await.unwrap();
    assert_eq!(index.pages[0].provenance, Provenance::Ocr);
    assert!(
        index.text.to_ascii_lowercase().contains("optimization"),
        "{}",
        index.text
    );
    let token = index
        .tokens
        .iter()
        .find(|token| token.text.to_ascii_lowercase().contains("optimization"))
        .unwrap();
    assert!((35.0..65.0).contains(&token.rects[0].x_min));
    assert!((45.0..100.0).contains(&token.rects[0].y_min));
    assert_eq!(token.provenance, Provenance::Ocr);
    let cached = load_or_build(&scan_path, &cache, false).await.unwrap();
    assert_eq!(
        serde_json::to_value(&cached).unwrap(),
        serde_json::to_value(&index).unwrap()
    );
    let mixed_path = directory.path().join("mixed.pdf");
    let mixed = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), b"<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>".to_vec(), b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(), b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(), stream("", source), b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /XObject << /Im1 7 0 R >> >> /Contents 8 0 R >>".to_vec(), objects[3].clone(), objects[4].clone()];
    tokio::fs::write(&mixed_path, pdf(&mixed)).await.unwrap();
    let mixed = load_or_build(&mixed_path, &directory.path().join("mixed-cache"), false)
        .await
        .unwrap();
    assert_eq!(
        mixed
            .pages
            .iter()
            .map(|page| page.provenance)
            .collect::<Vec<_>>(),
        [Provenance::Native, Provenance::Ocr]
    );
    // Replacing the PDF must not return the now-stale index.
    tokio::fs::write(&scan_path, b"no longer a pdf")
        .await
        .unwrap();
    assert!(load_or_build(&scan_path, &cache, false).await.is_err());
}

fn floating_continuation_pages() -> Vec<SourcePage> {
    let first = page(vec![
        prose_line(
            "We compare the single-view hand pose estimator",
            0,
            334.0,
            690.0,
        ),
        prose_line("with the UmeTrack baseline [11], which", 1, 330.0, 704.0),
    ]);
    let mut table = prose_line(
        "Method   MPJPE   Accuracy   32.91   50.3   54.7",
        0,
        50.0,
        65.0,
    );
    table.rect.x_max = 550.0;
    let mut caption = prose_line(
        "Table 5. Evaluation of action classification from hand poses.",
        1,
        50.0,
        100.0,
    );
    caption.block = 1;
    caption.rect.x_max = 550.0;
    let mut label = prose_line(
        "pick up put down position remove screw unscrew",
        2,
        50.0,
        220.0,
    );
    label.block = 2;
    let mut figure = prose_line(
        "Figure 6. Confusion matrices of verb classification.",
        3,
        50.0,
        260.0,
    );
    figure.block = 3;
    figure.rect.x_max = 550.0;
    let mut continuation = vec![
        prose_line(
            "was used to provide the original annotations.",
            4,
            50.0,
            310.0,
        ),
        prose_line(
            "Our classifier outperforms the comparison using",
            5,
            50.0,
            324.0,
        ),
        prose_line("poses estimated with UmeTrack.", 6, 50.0, 338.0),
        prose_line(
            "Additionally, we present classification confusion",
            7,
            54.0,
            352.0,
        ),
        prose_line("matrices and discuss remaining errors.", 8, 50.0, 366.0),
    ];
    for line in &mut continuation {
        line.block = 4;
    }
    let mut second = page(vec![table, caption, label, figure]);
    second.number = 2;
    second.words.extend(continuation);
    vec![first, second]
}

#[test]
fn logical_paragraph_skips_floating_tables_figures_and_captions_across_pages() {
    let index = assemble(&floating_continuation_pages());
    let paragraphs = paragraph_texts(&index);
    assert_eq!(
        paragraphs[0],
        "We compare the single-view hand pose estimator with the UmeTrack baseline [11], which was used to provide the original annotations. Our classifier outperforms the comparison using poses estimated with UmeTrack."
    );
    assert_eq!(index.objects.paragraph[0].spans.len(), 2);
    assert!(paragraphs.last().unwrap().starts_with("Additionally,"));
    assert_eq!(
        index
            .objects
            .paragraph
            .iter()
            .filter(|p| p.kind == "caption")
            .count(),
        2
    );
    assert_eq!(
        index
            .objects
            .paragraph
            .iter()
            .filter(|p| p.kind == "float")
            .count(),
        2
    );
    // Every original token belongs to exactly one paragraph, including floats.
    for token in &index.tokens {
        assert_eq!(
            index
                .objects
                .paragraph
                .iter()
                .filter(|p| paragraphs::pieces(p)
                    .iter()
                    .any(|span| span.start <= token.start && token.end <= span.end))
                .count(),
            1,
            "{}",
            token.text
        );
    }
    assert!(
        index
            .tokens
            .windows(2)
            .all(|pair| pair[0].end <= pair[1].start)
    );
    assert!(
        index
            .pages
            .windows(2)
            .all(|pair| pair[0].end <= pair[1].start)
    );
}

#[test]
fn floating_paragraph_link_rejects_new_indents_headings_prose_and_missing_pages() {
    for variant in 0..6 {
        let mut pages = floating_continuation_pages();
        match variant {
            0 => pages[0].words.last_mut().unwrap().text.push('.'),
            1 => pages[1].words[4].rect.x_min += 5.0,
            2 => pages[1].words[4].text = "Introduction".into(),
            3 => pages[1].number = 3,
            4 => {
                pages[1].words[4].rect.y_max += 4.0;
                pages[1].words[5].rect.y_max += 4.0;
                pages[1].words[6].rect.y_max += 4.0;
            },
            _ => pages[1].words[0].text = "Unrelated prose provides a complete explanation of another mechanism before the table.".into(),
        }
        let index = assemble(&pages);
        assert!(
            index.objects.paragraph[0].spans.is_empty(),
            "variant {variant}: {:?}",
            paragraph_texts(&index)
        );
    }
}

#[test]
fn logical_paragraph_can_continue_through_more_than_two_pages() {
    let mut pages = floating_continuation_pages();
    pages[1].words.truncate(7);
    pages[1].words[6].text = "poses estimated with UmeTrack, which".into();
    let mut third = pages[1].clone();
    third.number = 3;
    third.words[4].text = "provides a further independent comparison.".into();
    pages.push(third);
    let index = assemble(&pages);
    assert_eq!(index.objects.paragraph[0].spans.len(), 3);
    assert!(paragraph_texts(&index)[0].contains("which provides a further"));
}

#[test]
fn table_and_figure_namespaces_and_stacked_bounds_are_independent() {
    let index = assemble(&floating_continuation_pages());
    let figures = figures::find(&index);
    let table = figures.iter().find(|f| f.id == "table-5").unwrap();
    let figure = figures.iter().find(|f| f.id == "figure-6").unwrap();
    assert_eq!(table.kind, "table");
    assert_eq!(figure.kind, "figure");
    assert!(figure.rect.unwrap().y_min > table.rect.unwrap().y_max - 4.0);
    let source = page(vec![
        word("See", 0, 0, 50.0, 50.0),
        word("Table", 0, 0, 100.0, 50.0),
        word("1", 0, 0, 150.0, 50.0),
        word("for results.", 0, 0, 200.0, 50.0),
        word("Table", 1, 1, 50.0, 200.0),
        word("1.", 1, 1, 100.0, 200.0),
        word("Measurements.", 1, 1, 150.0, 200.0),
        word("Figure", 2, 2, 50.0, 400.0),
        word("1.", 2, 2, 100.0, 400.0),
        word("A diagram.", 2, 2, 150.0, 400.0),
    ]);
    let index = assemble(&[source]);
    let figures = figures::find(&index);
    assert_eq!(
        figures
            .iter()
            .find(|f| f.id == "table-1")
            .unwrap()
            .references
            .len(),
        1
    );
    assert!(
        figures
            .iter()
            .find(|f| f.id == "figure-1")
            .unwrap()
            .references
            .is_empty()
    );
}

#[test]
fn figure_members_include_long_body_classified_diagram_labels() {
    let mut label = prose_line(
        "Step 1: Bootstrapped annotations from multiple cameras; Step 2: Pose estimation; Step 3: Evaluation through action classification",
        2,
        50.0,
        220.0,
    );
    label.block = 1;
    label.rect.x_max = 550.0;
    label.rect.y_max = 228.0;
    let mut caption = prose_line(
        "Figure 2. Construction of the dataset and benchmark.",
        4,
        50.0,
        350.0,
    );
    caption.block = 3;
    caption.rect.x_max = 550.0;
    let mut index = assemble(&[page(vec![
        prose_line(
            "The preceding paragraph describes the experimental setup and dataset",
            0,
            50.0,
            100.0,
        ),
        prose_line(
            "and continues across a second line of body text in the left column.",
            1,
            50.0,
            114.0,
        ),
        label,
        word("Action classifier", 3, 2, 450.0, 290.0),
        caption,
        prose_line(
            "The next paragraph discusses the results without becoming part of the image.",
            5,
            50.0,
            410.0,
        ),
    ])]);
    let label = index
        .objects
        .paragraph
        .iter_mut()
        .find(|p| figures::utf16_slice(&index.text, p.start, p.end).starts_with("Step 1"))
        .unwrap();
    label.kind = "body".into(); // A text classifier is not authoritative about figure membership.
    let figures = figures::find(&index);
    let figure = &figures[0];
    let text = figure
        .spans
        .iter()
        .map(|s| figures::utf16_slice(&index.text, s.start, s.end))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("Step 1:"), "{figure:#?}");
    assert!(text.contains("Action classifier"));
    assert!(!text.contains("preceding paragraph"));
    assert!(!text.contains("next paragraph"));
}

#[test]
fn table_caption_above_cells_bounds_the_grid_and_excludes_following_prose() {
    let mut caption = prose_line("Table 3. Evaluation of the models.", 0, 50.0, 100.0);
    caption.rect.x_max = 500.0;
    let index = assemble(&[page(vec![
        caption,
        word("Method", 1, 1, 50.0, 125.0),
        word("Score", 1, 1, 200.0, 125.0),
        word("Baseline", 2, 2, 50.0, 140.0),
        word("32.91", 2, 2, 200.0, 140.0),
        word("Proposed", 3, 3, 50.0, 155.0),
        word("54.70", 3, 3, 200.0, 155.0),
        prose_line(
            "The results show that the model improves performance across the dataset",
            4,
            50.0,
            195.0,
        ),
        prose_line(
            "and the next paragraph continues with an analysis of the remaining failures.",
            5,
            50.0,
            209.0,
        ),
    ])]);
    let figures = figures::find(&index);
    let table = &figures[0];
    assert_eq!(table.kind, "table");
    assert!(table.rect.unwrap().y_max >= 165.0);
    let text = table
        .spans
        .iter()
        .map(|s| figures::utf16_slice(&index.text, s.start, s.end))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("54.70"));
    assert!(!text.contains("results show"));
}
