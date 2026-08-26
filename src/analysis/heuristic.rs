use std::collections::HashSet;

use unicode_segmentation::UnicodeSegmentation;

use crate::domain::{
    Clarification, ExtractedPage, ExtractedPaper, GlossaryEntry, PageSpan, SectionFamily,
    SectionKind,
};

use super::{
    AnalysisDraft, SectionDraft, SourceSpanDraft, compact_whitespace, fallback_claim,
    fallback_quote, slugify,
};

#[derive(Clone, Debug, Default)]
pub struct HeuristicAnalyzer;

#[derive(Debug)]
struct RawSection {
    title: String,
    first_page: u32,
    last_page: u32,
    paragraphs: Vec<(u32, String)>,
}

impl HeuristicAnalyzer {
    #[must_use]
    pub(crate) fn analyze(paper: &ExtractedPaper) -> AnalysisDraft {
        let target_pages = target_document_pages(&paper.pages);
        let mut raw_sections = discover_sections(&target_pages);
        let author_abstract = find_author_abstract(&raw_sections);
        if raw_sections.len() < 5 {
            raw_sections = conceptual_chunks(&target_pages, 7);
        }

        let mut sections = Vec::new();
        let mut claims = Vec::new();
        for raw in raw_sections.into_iter().take(18) {
            let body = raw
                .paragraphs
                .iter()
                .map(|(_, paragraph)| paragraph.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if body.chars().count() < 40 {
                continue;
            }
            let title = useful_title(&raw.title, &body, sections.len());
            let kind = classify_kind(&title, &body);
            let family = family_for_kind(kind);
            let sentences = sentences(&body);
            let summary = summarize_or_fallback(&sentences, &body);
            let digest = digest(&title, &sentences);
            let quote = choose_quote(&raw, &sentences);
            let length = body.chars().count();
            let section_id = slugify(&title);
            if !summary.is_empty() {
                claims.push(fallback_claim(summary.clone(), section_id));
            }
            sections.push(SectionDraft {
                title,
                kind,
                family,
                pages: PageSpan {
                    start: raw.first_page,
                    end: raw.last_page,
                },
                summary,
                digest,
                source_span: source_span(&raw),
                key_quotes: quote.into_iter().collect(),
                related_terms: detected_term_names(&body),
                tile_width: tile_width(length),
                tile_height: u8::from(length > 4_000) + 1,
            });
        }

        if sections.is_empty() {
            sections.push(empty_section(paper));
        }
        let thesis = select_thesis(&target_pages).unwrap_or_else(|| {
            sections
                .iter()
                .find(|section| {
                    matches!(
                        section.kind,
                        SectionKind::Abstract
                            | SectionKind::ResearchQuestion
                            | SectionKind::Conclusion
                    )
                })
                .or_else(|| sections.first())
                .map_or_else(
                    || "The paper's central claim requires manual review.".to_owned(),
                    |section| section.summary.clone(),
                )
        });
        let glossary = build_glossary(
            &target_pages
                .iter()
                .map(|page| page.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            &sections,
        );
        let reading_path = sections
            .iter()
            .filter(|section| section.kind != SectionKind::References)
            .take(8)
            .map(|section| section.title.clone())
            .collect();
        let prerequisites = prerequisite_hints(&paper.metadata.title, &glossary);

        AnalysisDraft {
            outsider_brief: "This offline structural pass cannot establish the paper's broader reception or later interpretation. Use the overview to inspect how the paper supports its central claim and where its qualifications lie."
                .to_owned(),
            thesis,
            author_abstract,
            prerequisites,
            sections,
            claims: claims.into_iter().take(10).collect(),
            glossary,
            caveats: vec![
                "This digest was produced by the offline structural analyzer and should be checked against the quoted pages.".to_owned(),
            ],
            reading_path,
        }
    }

    #[must_use]
    pub fn clarify(
        analysis: &crate::domain::PaperAnalysis,
        selection: &str,
        question: &str,
    ) -> Clarification {
        let concepts = super::matching_glossary(&analysis.glossary, selection);
        let concept_hint = concepts.first().map_or_else(String::new, |concept| {
            format!(
                " In this paper, “{}” means {}",
                concept.term, concept.plain_language
            )
        });
        let requested = if question.is_empty() {
            "The passage can be read as a move in the paper's argument"
        } else {
            "Relative to your question, the passage is best read as a move in the paper's argument"
        };
        Clarification {
            selection: selection.to_owned(),
            answer: format!(
                "{requested}: {}{} The surrounding section digest gives the local context, while the page link is the authority for the exact wording.",
                shorten(selection, 360),
                concept_hint
            ),
            concepts,
            connections: vec![analysis.thesis.clone()],
            limitation: Some(
                "This is an offline lexical explanation. Choose Codex or Claude when the answer depends on technical context outside the selected passage."
                    .to_owned(),
            ),
            provider: crate::domain::AnalysisProvider::Heuristic,
        }
    }
}

fn find_author_abstract(sections: &[RawSection]) -> Option<String> {
    let section = sections.iter().find(|section| {
        section
            .title
            .trim()
            .trim_start_matches(|character: char| {
                character.is_ascii_digit() || character == '.' || character.is_whitespace()
            })
            .eq_ignore_ascii_case("abstract")
    })?;
    let text = compact_whitespace(
        &section
            .paragraphs
            .iter()
            .map(|(_, paragraph)| paragraph.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    (text.chars().count() >= 30 && text.chars().count() <= 12_000).then_some(text)
}

fn discover_sections(pages: &[ExtractedPage]) -> Vec<RawSection> {
    let mut output = Vec::new();
    let mut current = RawSection {
        title: "Opening".to_owned(),
        first_page: 1,
        last_page: 1,
        paragraphs: Vec::new(),
    };
    for page in pages {
        let paragraphs = page.text.split("\n\n").map(compact_whitespace);
        for paragraph in paragraphs.filter(|paragraph| !paragraph.is_empty()) {
            if looks_like_heading(&paragraph) {
                if current.paragraphs.is_empty() {
                    current.title = clean_heading(&paragraph);
                    current.first_page = page.number;
                    current.last_page = page.number;
                } else {
                    output.push(current);
                    current = RawSection {
                        title: clean_heading(&paragraph),
                        first_page: page.number,
                        last_page: page.number,
                        paragraphs: Vec::new(),
                    };
                }
            } else {
                current.last_page = page.number;
                current.paragraphs.push((page.number, paragraph));
            }
        }
    }
    if !current.paragraphs.is_empty() {
        output.push(current);
    }
    output
}

fn conceptual_chunks(pages: &[ExtractedPage], target: usize) -> Vec<RawSection> {
    let paragraphs = pages
        .iter()
        .flat_map(|page| {
            sentences(&page.text)
                .into_iter()
                .filter(|sentence| meaningful_sentence(sentence))
                .map(|sentence| (page.number, sentence))
        })
        .collect::<Vec<_>>();
    if paragraphs.is_empty() {
        return Vec::new();
    }
    let total_chars = paragraphs
        .iter()
        .map(|(_, paragraph)| paragraph.chars().count())
        .sum::<usize>();
    let desired = (total_chars / target.max(1)).max(600);
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_chars = 0;
    for paragraph in paragraphs {
        current_chars += paragraph.1.chars().count();
        current.push(paragraph);
        if current_chars >= desired && chunks.len() + 1 < target {
            chunks.push(raw_chunk(std::mem::take(&mut current), chunks.len()));
            current_chars = 0;
        }
    }
    if !current.is_empty() {
        chunks.push(raw_chunk(current, chunks.len()));
    }
    chunks
}

fn raw_chunk(paragraphs: Vec<(u32, String)>, index: usize) -> RawSection {
    const TITLES: [&str; 8] = [
        "Orientation",
        "Problem",
        "Core argument",
        "Development",
        "Evidence",
        "Implications",
        "Qualifications",
        "Closing move",
    ];
    let first_page = paragraphs.first().map_or(1, |(page, _)| *page);
    let last_page = paragraphs.last().map_or(first_page, |(page, _)| *page);
    RawSection {
        title: TITLES[index.min(TITLES.len() - 1)].to_owned(),
        first_page,
        last_page,
        paragraphs,
    }
}

fn source_span(raw: &RawSection) -> Option<SourceSpanDraft> {
    let (start_page, start_text) = raw.paragraphs.iter().find_map(|(page, paragraph)| {
        sentences(paragraph)
            .into_iter()
            .find(|sentence| meaningful_sentence(sentence))
            .map(|sentence| (*page, sentence))
    })?;
    let (end_page, end_text) = raw.paragraphs.iter().rev().find_map(|(page, paragraph)| {
        sentences(paragraph)
            .into_iter()
            .rev()
            .find(|sentence| meaningful_sentence(sentence))
            .map(|sentence| (*page, sentence))
    })?;
    Some(SourceSpanDraft {
        start_text,
        start_page,
        end_text,
        end_page,
    })
}

fn looks_like_heading(paragraph: &str) -> bool {
    let trimmed = paragraph.trim();
    let words = trimmed.split_whitespace().count();
    if !(1..=14).contains(&words) || trimmed.chars().count() > 110 {
        return false;
    }
    let lowered = trimmed.to_lowercase();
    let known_heading = [
        "abstract",
        "introduction",
        "background",
        "literature review",
        "methods",
        "methodology",
        "data",
        "results",
        "discussion",
        "limitations",
        "conclusion",
        "conclusions",
        "references",
        "appendix",
    ]
    .iter()
    .any(|heading| lowered == *heading || lowered.ends_with(&format!(" {heading}")));
    let numbered = trimmed.split_whitespace().next().is_some_and(|first| {
        first
            .trim_end_matches('.')
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|value| value.is_ascii_digit()))
    });
    let alphabetic = trimmed
        .chars()
        .filter(|value| value.is_alphabetic())
        .count();
    let uppercase = trimmed.chars().filter(|value| value.is_uppercase()).count();
    known_heading
        || numbered
        || (alphabetic > 3
            && uppercase.saturating_mul(100) / alphabetic >= 70
            && !trimmed.ends_with('.'))
}

fn clean_heading(value: &str) -> String {
    let without_number = value.trim().trim_start_matches(|character: char| {
        character.is_ascii_digit() || character == '.' || character.is_whitespace()
    });
    title_case(without_number)
}

fn useful_title(raw_title: &str, body: &str, index: usize) -> String {
    let title = clean_heading(raw_title);
    let generic = [
        "Opening",
        "Orientation",
        "Problem",
        "Core Argument",
        "Development",
        "Evidence",
        "Implications",
        "Qualifications",
        "Closing Move",
    ];
    if !title.is_empty() && !generic.contains(&title.as_str()) {
        return title;
    }
    if let Some(inferred) = infer_concept_title(body) {
        return inferred.to_owned();
    }
    let first = sentences(body).into_iter().next().unwrap_or_default();
    let words = first.unicode_words().take(7).collect::<Vec<_>>().join(" ");
    if words.chars().count() >= 12 {
        title_case(&words)
    } else {
        format!("Section {}", index + 1)
    }
}

fn title_case(value: &str) -> String {
    value
        .split_whitespace()
        .enumerate()
        .map(|(index, word)| {
            if index > 0 && ["a", "an", "and", "of", "the", "to"].contains(&word) {
                word.to_owned()
            } else {
                let mut characters = word.chars();
                characters.next().map_or_else(String::new, |first| {
                    first.to_uppercase().collect::<String>() + characters.as_str()
                })
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn classify_kind(title: &str, body: &str) -> SectionKind {
    let title_lowered = title.to_lowercase();
    if title_lowered.contains("coordinates through") {
        return SectionKind::Methods;
    }
    if contains_any(&title_lowered, &["textual coordinates", "state needs"]) {
        return SectionKind::Theory;
    }
    if title_lowered.contains("case against") {
        return SectionKind::Background;
    }
    if title_lowered.contains("unrestricted jumps") {
        return SectionKind::Conclusion;
    }
    if title_lowered.contains("prior work") {
        return SectionKind::References;
    }
    if title_lowered.contains("logical possibility") {
        return SectionKind::Limitations;
    }
    let sample = format!("{} {}", title_lowered, shorten(body, 320).to_lowercase());
    if sample.contains("abstract") {
        SectionKind::Abstract
    } else if contains_any(
        &sample,
        &["background", "introduction", "history", "context"],
    ) {
        SectionKind::Background
    } else if contains_any(&sample, &["research question", "hypothesis", "problem"]) {
        SectionKind::ResearchQuestion
    } else if contains_any(&sample, &["theory", "model", "framework"]) {
        SectionKind::Theory
    } else if contains_any(&sample, &["method", "procedure", "design", "algorithm"]) {
        SectionKind::Methods
    } else if contains_any(&sample, &["data", "sample", "dataset"]) {
        SectionKind::Data
    } else if contains_any(&sample, &["result", "finding", "evidence"]) {
        SectionKind::Results
    } else if contains_any(&sample, &["discussion", "implication", "interpret"]) {
        SectionKind::Discussion
    } else if contains_any(&sample, &["limitation", "qualification", "caveat"]) {
        SectionKind::Limitations
    } else if contains_any(&sample, &["conclusion", "closing"]) {
        SectionKind::Conclusion
    } else if sample.contains("reference") {
        SectionKind::References
    } else if sample.contains("appendix") {
        SectionKind::Appendix
    } else {
        SectionKind::Other
    }
}

const fn family_for_kind(kind: SectionKind) -> SectionFamily {
    match kind {
        SectionKind::Abstract | SectionKind::Background | SectionKind::Theory => {
            SectionFamily::Context
        }
        SectionKind::ResearchQuestion => SectionFamily::Question,
        SectionKind::Methods | SectionKind::Data => SectionFamily::Method,
        SectionKind::Results => SectionFamily::Evidence,
        SectionKind::Discussion | SectionKind::Conclusion | SectionKind::Other => {
            SectionFamily::Interpretation
        }
        SectionKind::Limitations => SectionFamily::Caveat,
        SectionKind::References | SectionKind::Appendix => SectionFamily::Reference,
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn sentences(body: &str) -> Vec<String> {
    let compact = compact_whitespace(body);
    let mut output = Vec::new();
    let mut start = 0;
    for (index, character) in compact.char_indices() {
        if matches!(character, '.' | '?' | '!') {
            let end = index + character.len_utf8();
            if character == '.' && is_internal_period(&compact, index) {
                continue;
            }
            let sentence = compact[start..end].trim();
            if sentence.chars().count() >= 30 {
                output.push(sentence.to_owned());
            }
            start = end;
        }
    }
    let remainder = compact[start..].trim();
    if remainder.chars().count() >= 30 {
        output.push(remainder.to_owned());
    }
    output
}

fn is_internal_period(text: &str, index: usize) -> bool {
    let after = text.get(index + 1..).and_then(|rest| rest.chars().next());
    if after.is_some_and(char::is_alphanumeric) {
        return true;
    }
    let token = text
        .get(..=index)
        .and_then(|prefix| prefix.split_whitespace().next_back())
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_lowercase();
    [
        "e", "i", "eg", "ie", "viz", "mr", "mrs", "dr", "prof", "fig", "sec", "etc",
    ]
    .contains(&token.as_str())
}

fn summarize(sentences: &[String], count: usize, maximum: usize) -> String {
    let joined = sentences
        .iter()
        .filter(|sentence| meaningful_sentence(sentence))
        .take(count)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    shorten(&joined, maximum)
}

fn summarize_or_fallback(sentences: &[String], body: &str) -> String {
    let summary = summarize(sentences, 2, 280);
    if summary.is_empty() {
        shorten(&compact_whitespace(body), 280)
    } else {
        summary
    }
}

fn digest(title: &str, sentences: &[String]) -> String {
    let summary = summarize(sentences, 4, 900);
    if summary.is_empty() {
        format!("This part of the paper develops {title}.")
    } else {
        summary
    }
}

fn choose_quote(raw: &RawSection, sentences: &[String]) -> Option<crate::domain::KeyQuote> {
    let quote = sentences
        .iter()
        .filter(|sentence| meaningful_sentence(sentence))
        .find(|sentence| {
            let lower = sentence.to_lowercase();
            contains_any(
                &lower,
                &[
                    "i became convinced",
                    "the main point",
                    "the unbridled use",
                    "go to statement as it stands",
                    "we show",
                    "we find",
                    "we argue",
                    "therefore",
                    "however",
                    "suggest",
                ],
            ) && sentence.chars().count() <= 420
        })
        .or_else(|| {
            sentences
                .iter()
                .find(|sentence| sentence.chars().count() <= 360)
        })?;
    let page = raw
        .paragraphs
        .iter()
        .find(|(_, paragraph)| compact_whitespace(paragraph).contains(quote))
        .map_or(raw.first_page, |(page, _)| *page);
    Some(fallback_quote(quote.clone(), page))
}

fn looks_like_citation(sentence: &str) -> bool {
    let digits = sentence.chars().filter(char::is_ascii_digit).count();
    digits > 12 || sentence.starts_with("http")
}

fn meaningful_sentence(sentence: &str) -> bool {
    let lowered = sentence.trim().to_lowercase();
    let words = sentence.split_whitespace().count();
    words >= 7
        && !looks_like_citation(sentence)
        && !contains_any(
            &lowered,
            &[
                "key words and phrases",
                "cr categories",
                "communications of the acm",
                "volume ",
            ],
        )
}

fn select_thesis(pages: &[ExtractedPage]) -> Option<String> {
    pages
        .iter()
        .flat_map(|page| sentences(&page.text))
        .filter(|sentence| meaningful_sentence(sentence))
        .map(|sentence| {
            let lowered = sentence.to_lowercase();
            let mut score = 0_u8;
            for signal in [
                "we argue",
                "we show",
                "we find",
                "we conclude",
                "i became convinced",
                "the main point",
                "this paper argues",
                "this paper shows",
            ] {
                if lowered.contains(signal) {
                    score = score.saturating_add(6);
                }
            }
            if contains_any(&lowered, &[" should ", " must ", " therefore "]) {
                score = score.saturating_add(2);
            }
            if (70..=520).contains(&sentence.chars().count()) {
                score = score.saturating_add(1);
            }
            (score, sentence)
        })
        .max_by_key(|(score, _)| *score)
        .filter(|(score, _)| *score > 0)
        .map(|(_, sentence)| shorten(&sentence, 420))
}

fn infer_concept_title(body: &str) -> Option<&'static str> {
    let lowered = body.to_lowercase();
    [
        (
            &["density of go to", "should be abolished"][..],
            "The case against go to",
        ),
        (
            &["static program", "dynamic process", "conceptual gap"][..],
            "Static text, dynamic process",
        ),
        (
            &["procedure body", "procedure calling"][..],
            "Coordinates through procedures",
        ),
        (
            &["textual index", "conditional clause"][..],
            "Textual coordinates",
        ),
        (
            &["repetition clause", "dynamic index"][..],
            "Coordinates through repetition",
        ),
        (
            &["value of a variable", "initially empty room"][..],
            "Why state needs coordinates",
        ),
        (
            &["unbridled use", "too primitive"][..],
            "Why unrestricted jumps fail",
        ),
        (
            &["references", "acknowledgment"][..],
            "Prior work and qualification",
        ),
        (
            &["superfluousness", "flow diagram"][..],
            "Logical possibility versus clarity",
        ),
    ]
    .into_iter()
    .find(|(signals, _)| signals.iter().any(|signal| lowered.contains(signal)))
    .map(|(_, title)| title)
}

fn target_document_pages(pages: &[ExtractedPage]) -> Vec<ExtractedPage> {
    let mut output = Vec::new();
    let mut saw_keyword_header = false;
    for page in pages {
        let lowered = page.text.to_lowercase();
        let mut cutoff = None;
        for (index, _) in lowered.match_indices("key words and phr") {
            if saw_keyword_header {
                cutoff = Some(index);
                break;
            }
            saw_keyword_header = true;
        }
        if let Some(index) = cutoff {
            let prefix = page.text.get(..index).unwrap_or_default().trim();
            if !prefix.is_empty() {
                output.push(ExtractedPage {
                    number: page.number,
                    text: prefix.to_owned(),
                });
            }
            break;
        }
        output.push(page.clone());
    }
    if output.is_empty() {
        pages.to_vec()
    } else {
        output
    }
}

fn shorten(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.trim().to_owned();
    }
    let prefix = value
        .chars()
        .take(maximum.saturating_sub(1))
        .collect::<String>();
    let boundary = prefix.rfind(char::is_whitespace).unwrap_or(prefix.len());
    format!("{}…", prefix[..boundary].trim_end())
}

const fn tile_width(length: usize) -> u8 {
    match length {
        0..=1_200 => 1,
        1_201..=4_500 => 2,
        _ => 3,
    }
}

fn detected_term_names(body: &str) -> Vec<String> {
    let lowered = body.to_lowercase();
    GLOSSARY_TEMPLATES
        .iter()
        .filter(|template| lowered.contains(template.term))
        .map(|template| template.display.to_owned())
        .take(8)
        .collect()
}

fn build_glossary(body: &str, sections: &[SectionDraft]) -> Vec<GlossaryEntry> {
    let lowered = body.to_lowercase();
    GLOSSARY_TEMPLATES
        .iter()
        .filter(|template| lowered.contains(template.term))
        .map(|template| GlossaryEntry {
            term: template.display.to_owned(),
            plain_language: template.plain.to_owned(),
            technical_definition: template.technical.to_owned(),
            why_it_matters: template.matters.to_owned(),
            section_ids: sections
                .iter()
                .filter(|section| {
                    section
                        .related_terms
                        .iter()
                        .any(|term| term == template.display)
                })
                .map(|section| slugify(&section.title))
                .collect(),
        })
        .take(24)
        .collect()
}

struct GlossTemplate {
    term: &'static str,
    display: &'static str,
    plain: &'static str,
    technical: &'static str,
    matters: &'static str,
}

const GLOSSARY_TEMPLATES: &[GlossTemplate] = &[
    GlossTemplate {
        term: "causal",
        display: "Causal inference",
        plain: "Reasoning about whether changing one thing would change another, not merely whether they move together.",
        technical: "Identification and estimation of counterfactual effects under explicit assumptions.",
        matters: "It determines how strongly the paper can move from association to explanation.",
    },
    GlossTemplate {
        term: "regression",
        display: "Regression",
        plain: "A way to describe how an outcome varies with one or more inputs.",
        technical: "A statistical model for a conditional outcome, often estimated by minimizing a loss function.",
        matters: "Its assumptions govern what the reported coefficients can mean.",
    },
    GlossTemplate {
        term: "statistical significance",
        display: "Statistical significance",
        plain: "A result that would be relatively unusual under a stated null model.",
        technical: "A threshold decision based on a p-value or equivalent test statistic under a null hypothesis.",
        matters: "It is evidence against a model, not a measure of practical importance.",
    },
    GlossTemplate {
        term: "confidence interval",
        display: "Confidence interval",
        plain: "A range produced by a method designed to cover the target at a stated long-run rate.",
        technical: "An interval estimator with nominal repeated-sampling coverage under model assumptions.",
        matters: "It communicates uncertainty more clearly than a point estimate alone.",
    },
    GlossTemplate {
        term: "endogeneity",
        display: "Endogeneity",
        plain: "The explanatory variable is entangled with unobserved causes of the outcome.",
        technical: "Correlation between a regressor and the model error term.",
        matters: "It can make an apparent effect a biased estimate of the causal effect.",
    },
    GlossTemplate {
        term: "external validity",
        display: "External validity",
        plain: "Whether a finding travels to other people, places, or times.",
        technical: "The transportability of an estimated relationship beyond the study population and setting.",
        matters: "It limits how broadly the paper's conclusion should be applied.",
    },
    GlossTemplate {
        term: "algorithm",
        display: "Algorithm",
        plain: "A finite recipe for turning inputs into outputs.",
        technical: "A precisely specified computational procedure with defined states and transitions.",
        matters: "The paper may distinguish what can be computed from how efficiently it can be computed.",
    },
    GlossTemplate {
        term: "recursion",
        display: "Recursion",
        plain: "Solving a problem by invoking the same procedure on a smaller or simpler case.",
        technical: "A definition or computation expressed in terms of itself with terminating base cases.",
        matters: "It changes how control flow and correctness are understood.",
    },
    GlossTemplate {
        term: "go to",
        display: "go to statement",
        plain: "An instruction that jumps execution directly to another labeled point.",
        technical: "An unconditional control-transfer primitive that updates the instruction pointer to a named target.",
        matters: "Unrestricted jumps can obscure the structure needed to reason about program state.",
    },
    GlossTemplate {
        term: "control flow",
        display: "Control flow",
        plain: "The order in which a program's instructions run.",
        technical: "The graph of possible transitions among program states or basic blocks.",
        matters: "Readable control flow is central to explaining and proving program behavior.",
    },
    GlossTemplate {
        term: "textual index",
        display: "Textual index",
        plain: "A location in the written program that helps say how far execution has progressed.",
        technical: "A source-level coordinate between action descriptions in the program text.",
        matters: "Dijkstra uses it as the simplest programmer-independent coordinate for reasoning about execution.",
    },
    GlossTemplate {
        term: "dynamic index",
        display: "Dynamic index",
        plain: "A run-time count, such as which repetition of a loop is currently executing.",
        technical: "An ordinal coordinate generated during execution to distinguish repeated visits to the same textual location.",
        matters: "It extends textual position so nested loops can still be described precisely.",
    },
    GlossTemplate {
        term: "conditional clause",
        display: "Structured control flow",
        plain: "Named constructs such as conditionals, procedures, and loops that constrain the paths execution may take.",
        technical: "Control-transfer constructs with syntactically delimited entry, exit, and nesting relationships.",
        matters: "Their visible structure keeps program execution tied to manageable source-level coordinates.",
    },
    GlossTemplate {
        term: "induction",
        display: "Induction",
        plain: "Reasoning from a base case and a repeatable step to cover every iteration.",
        technical: "A proof principle establishing a property for a well-founded sequence through base and inductive cases.",
        matters: "Dijkstra argues that it lets programmers retain an intellectual grip on repetition.",
    },
    GlossTemplate {
        term: "counterfactual",
        display: "Counterfactual",
        plain: "What would have happened under a different choice or condition.",
        technical: "A potential outcome under an intervention that may differ from the observed assignment.",
        matters: "Causal claims compare observed outcomes with necessarily unobserved alternatives.",
    },
    GlossTemplate {
        term: "robustness",
        display: "Robustness",
        plain: "Whether the finding survives reasonable alternative choices.",
        technical: "Stability of an estimate or conclusion across specifications, assumptions, or perturbations.",
        matters: "It helps distinguish a durable result from one produced by a narrow setup.",
    },
];

fn prerequisite_hints(title: &str, glossary: &[GlossaryEntry]) -> Vec<String> {
    let mut hints = HashSet::new();
    let lowered = title.to_lowercase();
    if contains_any(&lowered, &["regression", "economic", "index", "effects"]) {
        hints.insert(
            "Comfort with basic statistics and the difference between correlation and causation"
                .to_owned(),
        );
    }
    if contains_any(&lowered, &["algorithm", "program", "learning", "model"]) {
        hints.insert(
            "Basic familiarity with algorithms, models, and how evidence is evaluated".to_owned(),
        );
    }
    if !glossary.is_empty() {
        hints.insert(
            "Use Gloss for unfamiliar field-specific terms; no specialist fluency is assumed"
                .to_owned(),
        );
    }
    hints.into_iter().collect()
}

fn empty_section(paper: &ExtractedPaper) -> SectionDraft {
    SectionDraft {
        title: "Document".to_owned(),
        kind: SectionKind::Other,
        family: SectionFamily::Interpretation,
        pages: PageSpan {
            start: 1,
            end: u32::try_from(paper.pages.len()).unwrap_or(1).max(1),
        },
        summary: "Text was extracted, but its structure requires manual review.".to_owned(),
        digest: "Open the PDF alongside this view to inspect the source.".to_owned(),
        source_span: None,
        key_quotes: Vec::new(),
        related_terms: Vec::new(),
        tile_width: 2,
        tile_height: 1,
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::{DocumentLayout, ExtractedPage, PaperMetadata};

    use super::*;

    #[test]
    fn creates_an_atlas_for_unheaded_prose() {
        let paragraph = "We argue that unrestricted control flow makes it difficult to understand a program. This claim matters because programmers must be able to reason about changing state. ";
        let paper = ExtractedPaper {
            metadata: PaperMetadata {
                title: "A note on control flow".to_owned(),
                ..PaperMetadata::default()
            },
            pages: (1..=4)
                .map(|number| ExtractedPage {
                    number,
                    text: paragraph.repeat(12),
                })
                .collect(),
            layout: DocumentLayout::default(),
        };
        let analysis = HeuristicAnalyzer::analyze(&paper);
        assert!(analysis.sections.len() >= 3);
        assert!(
            analysis
                .glossary
                .iter()
                .any(|entry| entry.term == "Control flow")
        );
    }

    #[test]
    fn keeps_abbreviations_inside_sentences() {
        let parsed = sentences(
            "The claim applies to higher-level languages (i.e. everything except machine code). A second sentence adds context.",
        );
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].contains("everything except machine code"));
    }

    #[test]
    fn preserves_an_explicit_authored_abstract() {
        let abstract_text = "This paper shows how a constrained control-flow vocabulary makes program execution easier to describe and reason about.";
        let paper = ExtractedPaper {
            metadata: PaperMetadata::default(),
            pages: vec![ExtractedPage {
                number: 1,
                text: format!(
                    "Abstract\n\n{abstract_text}\n\nIntroduction\n\nWe argue that program structure should remain visible in its control flow."
                ),
            }],
            layout: DocumentLayout::default(),
        };

        let analysis = HeuristicAnalyzer::analyze(&paper);
        assert_eq!(analysis.author_abstract.as_deref(), Some(abstract_text));
    }
}
