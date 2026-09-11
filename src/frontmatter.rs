//! Source prose and author names use literal PDF words, not the merged citation index.
use crate::domain::{DocumentLayout, ExtractedPage, LayoutPage, LayoutToken, PaperMetadata};
use std::collections::BTreeMap;

fn lines(page: &LayoutPage) -> Vec<Vec<&LayoutToken>> {
    let mut lines = BTreeMap::<u32, Vec<&LayoutToken>>::new();
    for token in &page.tokens {
        lines.entry(token.line).or_default().push(token);
    }
    lines.into_values().collect()
}

#[must_use]
pub fn readable_pages(layout: &DocumentLayout) -> Vec<ExtractedPage> {
    layout
        .pages
        .iter()
        .map(|page| ExtractedPage {
            number: page.number,
            text: lines(page)
                .iter()
                .map(|line| {
                    let mut text = String::new();
                    let mut previous: Option<&LayoutToken> = None;
                    for token in line {
                        let raised = previous.is_some_and(|p| {
                            p.text.chars().filter(|c| c.is_alphabetic()).count() > 2
                                && p.rects
                                    .first()
                                    .zip(token.rects.first())
                                    .is_some_and(|(a, b)| {
                                        a.y_max - a.y_min < 30.0
                                            && b.y_max - b.y_min < (a.y_max - a.y_min) * 0.8
                                            && b.y_min < a.y_min
                                    })
                        });
                        if raised && token.text.chars().all(|c| c.is_ascii_digit()) {
                            for c in token.text.chars() {
                                text.push(match c {
                                    '0' => '⁰',
                                    '1' => '¹',
                                    '2' => '²',
                                    '3' => '³',
                                    '4' => '⁴',
                                    '5' => '⁵',
                                    '6' => '⁶',
                                    '7' => '⁷',
                                    '8' => '⁸',
                                    _ => '⁹',
                                });
                            }
                        } else {
                            if !text.is_empty()
                                && !token
                                    .text
                                    .starts_with(['.', ',', ';', ':', '!', '?', ')', ']'])
                            {
                                text.push(' ');
                            }
                            text.push_str(&token.text);
                        }
                        previous = Some(token);
                    }
                    text.replace(" .", ".")
                        .replace(" ,", ",")
                        .replace(" ;", ";")
                        .replace(" :", ":")
                })
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect()
}

fn name_word(word: &str) -> bool {
    matches!(
        word,
        "de" | "del" | "van" | "von" | "der" | "da" | "di" | "al"
    ) || word.chars().next().is_some_and(char::is_uppercase)
        && word
            .chars()
            .all(|c| c.is_alphabetic() || ".-'’".contains(c))
}

fn name(words: &[String]) -> bool {
    (2..=7).contains(&words.len()) && words.iter().all(|w| name_word(w))
}

#[must_use]
pub fn authors(layout: &DocumentLayout, metadata: &PaperMetadata) -> Vec<String> {
    let Some(page) = layout.pages.first() else {
        return Vec::new();
    };
    let title_words = metadata
        .title
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| w.len() > 2)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut found = Vec::new();
    let mut saw_title = false;
    for line in lines(page).iter().take(60) {
        let text = line
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let words = text.split_whitespace().collect::<Vec<_>>();
        let overlap = words
            .iter()
            .filter(|w| {
                title_words.contains(&w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
            })
            .count();
        if overlap * 2 >= words.len() && overlap > 0 && found.is_empty() {
            saw_title = true;
            continue;
        }
        if !saw_title || text.contains("arXiv:") {
            continue;
        }
        if ["abstract", "introduction", "contents", "tableofcontents"]
            .contains(&text.to_lowercase().replace(' ', "").as_str())
        {
            break;
        }
        if [
            "university",
            "institute",
            "laborator",
            "department",
            "reality labs",
            "@",
            "http",
            "january",
            "february",
            "march",
            "april",
            "may ",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
        ]
        .iter()
        .any(|s| text.to_lowercase().contains(s))
        {
            continue;
        }
        if line
            .iter()
            .any(|t| t.rects.iter().any(|r| r.y_min > page.height * 0.5))
        {
            break;
        }
        if let Some(candidates) = names_on_line(line) {
            found.extend(candidates);
        } else if !found.is_empty() && words.len() > 7 {
            break;
        }
    }
    found.dedup();
    found
}

fn names_on_line(line: &[&LayoutToken]) -> Option<Vec<String>> {
    let mut current = Vec::<String>::new();
    let mut candidates = Vec::new();
    let mut previous_right = None;
    let mut valid = true;
    for token in line {
        let word = token.text.trim_matches([',', ';']);
        let marker = word
            .chars()
            .all(|c| c.is_ascii_digit() || ",*∗†‡¹²³".contains(c));
        let rect = token.rects.first();
        let gap = rect
            .zip(previous_right)
            .is_some_and(|(r, right)| r.x_min - right > (r.y_max - r.y_min) * 0.65);
        if marker || matches!(word, "and" | "&") || gap && name(&current) {
            if name(&current) {
                candidates.push(current.join(" "));
                current.clear();
            }
            if marker || matches!(word, "and" | "&") {
                previous_right = rect.map(|r| r.x_max);
                continue;
            }
        }
        let word = word.trim_end_matches(|c: char| c.is_ascii_digit() || "*∗†‡¹²³".contains(c));
        if !name_word(word) {
            valid = false;
            break;
        }
        current.push(word.to_owned());
        if token.text.ends_with([',', ';']) && name(&current) {
            candidates.push(current.join(" "));
            current.clear();
        }
        previous_right = rect.map(|r| r.x_max);
    }
    if name(&current) {
        candidates.push(current.join(" "));
    } else if !current.is_empty() {
        valid = false;
    }
    (valid && !candidates.is_empty()).then_some(candidates)
}
