//! Conservative parsing of one printed author name.
//!
//! Raw evidence is never rewritten. Commas and explicit initials constrain the
//! split; unmarked full names retain every plausible word boundary in both
//! orders. Alternatives are deterministic, not ranked. A blocking-key collision
//! retrieves candidates only: it cannot establish that two observations identify
//! the same person (including people with different suffixes or given names).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

const MAX_NAME_BYTES: usize = 1024;
const MAX_NAME_WORDS: usize = 16;
const MAX_ALTERNATIVES: usize = 256;

/// The relationship between the printed words and the parsed components.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameOrder {
    GivenFirst,
    FamilyFirst,
    /// One undivided token: no given/family boundary can be inferred.
    Undivided,
}

/// One possible interpretation, preserving casing and accents in components.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct NameParts {
    pub family_name: String,
    pub given_names: Vec<String>,
    pub particles: Vec<String>,
    pub suffix: Option<String>,
    pub order: NameOrder,
}

/// A coarse candidate key, never an identity or an automatic-merge decision.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct NameBlockingKey {
    pub family: String,
    pub first_initial: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnparsedReason {
    Empty,
    TooLong,
    TooManyWords,
    TooManyAlternatives,
    UnsupportedSyntax,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParsedName {
    pub raw: String,
    /// No alternative is preferred. Empty when `unparsed_reason` is present.
    pub alternatives: Vec<NameParts>,
    pub unparsed_reason: Option<UnparsedReason>,
}

impl ParsedName {
    #[must_use]
    pub const fn is_ambiguous(&self) -> bool {
        self.alternatives.len() > 1
    }

    /// Retrieve candidates for every interpretation without cross-pairing a
    /// family from one alternative with a given name from another.
    #[must_use]
    pub fn blocking_keys(&self) -> BTreeSet<NameBlockingKey> {
        self.alternatives
            .iter()
            .flat_map(NameParts::blocking_keys)
            .collect()
    }
}

impl NameParts {
    /// Initials retain hyphenated given-name components (Jean-Paul -> j, p).
    /// Dotted/compact initial groups have already been split during parsing.
    #[must_use]
    pub fn initials(&self) -> Vec<String> {
        self.given_names
            .iter()
            .flat_map(|given| given.split('-'))
            .flat_map(split_dotted_initials)
            .filter_map(|given| given.chars().find(|ch| ch.is_alphabetic()))
            .map(|ch| fold_initial(ch).to_string())
            .collect()
    }

    /// Particle-bearing and particle-free spellings may retrieve one another.
    /// Missing given names never produce an empty-initial wildcard key.
    #[must_use]
    pub fn blocking_keys(&self) -> BTreeSet<NameBlockingKey> {
        let Some(first_initial) = self.initials().into_iter().next() else {
            return BTreeSet::new();
        };
        let full_family = self
            .particles
            .iter()
            .chain(std::iter::once(&self.family_name))
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        family_keys(&self.family_name)
            .into_iter()
            .chain(family_keys(&full_family))
            .map(|family| NameBlockingKey {
                family,
                first_initial: first_initial.clone(),
            })
            .collect()
    }
}

/// Parse a single bibliographic name without inferring order from ethnicity,
/// language, a surname dictionary, or capitalization.
///
/// Bounds are explicit; unsupported input stays unresolved rather than being truncated.
#[must_use]
pub fn parse_name(raw: &str) -> ParsedName {
    let mut result = ParsedName {
        raw: raw.to_owned(),
        alternatives: Vec::new(),
        unparsed_reason: None,
    };
    let normalized = normalize_printed(raw);
    let invalid = if normalized.is_empty() {
        Some(UnparsedReason::Empty)
    } else if raw.len() > MAX_NAME_BYTES {
        Some(UnparsedReason::TooLong)
    } else if normalized.split_whitespace().count() > MAX_NAME_WORDS {
        Some(UnparsedReason::TooManyWords)
    } else if !valid_name_syntax(&normalized) {
        Some(UnparsedReason::UnsupportedSyntax)
    } else {
        None
    };
    if let Some(reason) = invalid {
        result.unparsed_reason = Some(reason);
        return result;
    }
    let fields: Vec<_> = normalized.split(',').map(str::trim).collect();
    let mut alternatives = Candidates::default();
    parse_fields(&fields, &mut alternatives);
    if alternatives.overflow {
        result.unparsed_reason = Some(UnparsedReason::TooManyAlternatives);
        return result;
    }
    result.alternatives = alternatives.parts.into_iter().collect();
    if result.alternatives.is_empty() {
        result.unparsed_reason = Some(UnparsedReason::UnsupportedSyntax);
    }
    result
}

#[derive(Default)]
struct Candidates {
    parts: BTreeSet<NameParts>,
    overflow: bool,
}

impl Candidates {
    fn insert(&mut self, parts: NameParts) {
        if !self.overflow {
            self.parts.insert(parts);
            self.overflow = self.parts.len() > MAX_ALTERNATIVES;
        }
    }
}

fn parse_fields(fields: &[&str], output: &mut Candidates) {
    match fields {
        [name] => parse_unmarked(name, None, output),
        [name, suffix] if is_suffix(suffix) => {
            parse_unmarked(name, Some((*suffix).to_owned()), output);
            parse_explicit(name, suffix, None, output);
        }
        [family, given] => parse_explicit(family, given, None, output),
        [family, given, suffix] if is_suffix(suffix) => {
            parse_explicit(family, given, Some((*suffix).to_owned()), output);
        }
        [family, suffix, given] if is_suffix(suffix) => {
            parse_explicit(family, given, Some((*suffix).to_owned()), output);
        }
        _ => {}
    }
}

fn parse_explicit(family: &str, given: &str, mut suffix: Option<String>, output: &mut Candidates) {
    let mut family_words: Vec<_> = family.split_whitespace().collect();
    let mut given_words: Vec<_> = given.split_whitespace().collect();
    if suffix.is_none() {
        add_parts(
            &family_words,
            &given_words,
            None,
            NameOrder::FamilyFirst,
            true,
            output,
        );
    }
    take_suffix(&mut family_words, &mut suffix);
    take_suffix(&mut given_words, &mut suffix);
    add_parts(
        &family_words,
        &given_words,
        suffix.as_deref(),
        NameOrder::FamilyFirst,
        true,
        output,
    );
}

fn parse_unmarked(name: &str, mut suffix: Option<String>, output: &mut Candidates) {
    let mut words: Vec<_> = name.split_whitespace().collect();
    let original = words.clone();
    take_suffix(&mut words, &mut suffix);
    if words != original {
        parse_words(&original, None, output);
    }
    parse_words(&words, suffix, output);
}

fn parse_words(words: &[&str], suffix: Option<String>, output: &mut Candidates) {
    if words.len() == 1 {
        if is_word(words[0]) && !is_explicit_initial(words[0]) && !is_particle(words[0]) {
            output.insert(NameParts {
                family_name: words[0].to_owned(),
                given_names: Vec::new(),
                particles: Vec::new(),
                suffix,
                order: NameOrder::Undivided,
            });
        }
        return;
    }
    for boundary in 1..words.len() {
        if output.overflow {
            return;
        }
        add_parts(
            &words[boundary..],
            &words[..boundary],
            suffix.as_deref(),
            NameOrder::GivenFirst,
            false,
            output,
        );
        add_parts(
            &words[..boundary],
            &words[boundary..],
            suffix.as_deref(),
            NameOrder::FamilyFirst,
            false,
            output,
        );
    }
}

fn add_parts(
    family_words: &[&str],
    given_words: &[&str],
    suffix: Option<&str>,
    order: NameOrder,
    family_explicit: bool,
    output: &mut Candidates,
) {
    if family_words.is_empty()
        || given_words.is_empty()
        || family_words
            .iter()
            .any(|word| !is_word(word) || !family_explicit && is_explicit_initial(word))
        || given_words.iter().any(|word| !is_word(word))
        || family_words.last().is_some_and(|word| is_particle(word))
        || given_words.last().is_some_and(|word| is_particle(word))
    {
        return;
    }
    let particle_count = family_words
        .iter()
        .take_while(|word| is_particle(word))
        .count();
    let family_name = family_words[particle_count..].join(" ");
    let particles: Vec<_> = family_words[..particle_count]
        .iter()
        .map(|word| (*word).to_owned())
        .collect();
    let given_names: Vec<_> = given_words
        .iter()
        .flat_map(|word| split_dotted_initials(word))
        .collect();
    let base = NameParts {
        family_name,
        given_names,
        particles,
        suffix: suffix.map(str::to_owned),
        order,
    };
    // All-caps short words can be names or compact initials (ANN / JD). Keep
    // both interpretations instead of treating every capitalized word as initials.
    let mut variants = vec![Vec::new()];
    for word in &base.given_names {
        let choices = if is_compact_initial_candidate(word) {
            vec![
                vec![word.clone()],
                word.chars().map(|ch| ch.to_string()).collect(),
            ]
        } else {
            vec![vec![word.clone()]]
        };
        if variants.len() * choices.len() > MAX_ALTERNATIVES {
            output.overflow = true;
            return;
        }
        variants = variants
            .into_iter()
            .flat_map(|prefix| {
                choices
                    .iter()
                    .map(move |choice| prefix.iter().chain(choice).cloned().collect())
            })
            .collect();
    }
    for given_names in variants {
        output.insert(NameParts {
            given_names,
            ..base.clone()
        });
    }
}

fn take_suffix(words: &mut Vec<&str>, suffix: &mut Option<String>) {
    if suffix.is_none() && words.len() > 1 && words.last().is_some_and(|word| is_suffix(word)) {
        *suffix = words.pop().map(str::to_owned);
    }
}

fn is_suffix(word: &str) -> bool {
    matches!(
        word.trim_end_matches('.').to_lowercase().as_str(),
        "jr" | "sr" | "ii" | "iii" | "iv" | "v"
    )
}

fn is_particle(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "al" | "el"
            | "da"
            | "das"
            | "de"
            | "del"
            | "della"
            | "den"
            | "der"
            | "des"
            | "di"
            | "do"
            | "dos"
            | "du"
            | "la"
            | "le"
            | "ten"
            | "ter"
            | "van"
            | "von"
    )
}

fn normalize_printed(raw: &str) -> String {
    raw.nfc()
        .map(|ch| match ch {
            '\u{2010}' | '\u{2011}' => '-',
            '\u{2018}' | '\u{2019}' | '\u{02bc}' => '\'',
            _ => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn valid_name_syntax(name: &str) -> bool {
    name.chars().all(|ch| {
        ch.is_alphabetic() || is_combining_mark(ch) || ch.is_whitespace() || ",.'-".contains(ch)
    }) && !name.to_lowercase().contains(" et al")
}

fn is_word(word: &str) -> bool {
    word.split('-').all(|part| {
        !part.is_empty()
            && part.chars().any(char::is_alphabetic)
            && (!part.contains('.')
                || is_explicit_initial(part)
                || part.eq_ignore_ascii_case("St."))
    })
}

fn is_explicit_initial(word: &str) -> bool {
    word.split('-').all(|part| {
        let letters: Vec<_> = part.chars().filter(|ch| ch.is_alphabetic()).collect();
        !letters.is_empty()
            && !part.starts_with('.')
            && !part.contains("..")
            && (letters.len() == 1
                || part.contains('.')
                    && part
                        .split('.')
                        .filter(|s| !s.is_empty())
                        .all(|s| s.chars().filter(|ch| ch.is_alphabetic()).count() == 1))
            && letters
                .iter()
                .all(|ch| ch.is_uppercase() || ch.is_lowercase())
            && part
                .chars()
                .all(|ch| ch.is_alphabetic() || is_combining_mark(ch) || ch == '.')
    })
}

fn split_dotted_initials(word: &str) -> Vec<String> {
    if word.contains('.') && !word.contains('-') && is_explicit_initial(word) {
        word.split('.')
            .filter(|part| !part.is_empty())
            .map(|part| format!("{part}."))
            .collect()
    } else {
        vec![word.to_owned()]
    }
}

fn is_compact_initial_candidate(word: &str) -> bool {
    (2..=3).contains(&word.chars().count()) && word.chars().all(char::is_uppercase)
}

fn fold_initial(ch: char) -> char {
    family_keys(&ch.to_string())
        .iter()
        .next()
        .and_then(|key| key.chars().next())
        .unwrap_or(ch)
}

/// Accent-folded spellings for candidate retrieval.
///
/// Umlauts also receive the
/// explicit ae/oe/ue spelling, so Müller/Mueller overlap without rewriting raw
/// evidence. Lossy keys are intentionally not suitable for equality decisions.
#[must_use]
pub fn family_keys(family: &str) -> BTreeSet<String> {
    let normalized: String = normalize_printed(family).to_lowercase();
    let transliterated = normalized
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue");
    [normalized, transliterated]
        .iter()
        .map(|variant| {
            let mut key = String::new();
            for ch in variant.nfkd().filter(|ch| !is_combining_mark(*ch)) {
                match ch {
                    'ß' => key.push_str("ss"),
                    'ς' => key.push('σ'),
                    'ł' => key.push('l'),
                    'ø' => key.push('o'),
                    'æ' => key.push_str("ae"),
                    'œ' => key.push_str("oe"),
                    '\'' => {}
                    ch if ch.is_alphabetic() => key.push(ch),
                    _ if !key.ends_with(' ') => key.push(' '),
                    _ => {}
                }
            }
            key.trim().to_owned()
        })
        .filter(|key| !key.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_parts(parsed: &ParsedName, family: &str, given: &[&str], particles: &[&str]) -> bool {
        parsed.alternatives.iter().any(|parts| {
            parts.family_name == family
                && parts.given_names == given
                && parts.particles == particles
        })
    }

    type Case<'a> = (
        &'a str,
        &'a str,
        &'a [&'a str],
        &'a [&'a str],
        Option<&'a str>,
    );
    const CASES: &[Case<'_>] = &[
        (
            "van der Waals, J. D.",
            "Waals",
            &["J.", "D."],
            &["van", "der"],
            None,
        ),
        ("J.-P. Serre", "Serre", &["J.-P."], &[], None),
        ("J-P Serre", "Serre", &["J-P"], &[], None),
        ("Serre, Jean-Paul", "Serre", &["Jean-Paul"], &[], None),
        (
            "Smith, John Ronald",
            "Smith",
            &["John", "Ronald"],
            &[],
            None,
        ),
        (
            "de la Cruz, Juan Carlos",
            "Cruz",
            &["Juan", "Carlos"],
            &["de", "la"],
            None,
        ),
        ("von Neumann, John", "Neumann", &["John"], &["von"], None),
        (
            "van Beethoven, Ludwig",
            "Beethoven",
            &["Ludwig"],
            &["van"],
            None,
        ),
        (
            "VAN DER WAALS, J. D.",
            "WAALS",
            &["J.", "D."],
            &["VAN", "DER"],
            None,
        ),
        (
            "García Márquez, Gabriel",
            "García Márquez",
            &["Gabriel"],
            &[],
            None,
        ),
        (
            "García-Márquez, Gabriel",
            "García-Márquez",
            &["Gabriel"],
            &[],
            None,
        ),
        ("Smith, Mary Ann", "Smith", &["Mary", "Ann"], &[], None),
        ("O’Neill, Eugene", "O'Neill", &["Eugene"], &[], None),
        ("d'Alembert, Jean", "d'Alembert", &["Jean"], &[], None),
        ("Müller, J.", "Müller", &["J."], &[], None),
        ("Mueller, J", "Mueller", &["J"], &[], None),
        ("Łukasiewicz, Jan", "Łukasiewicz", &["Jan"], &[], None),
        ("Nguyễn, Thị Minh", "Nguyễn", &["Thị", "Minh"], &[], None),
        ("王, 小明", "王", &["小明"], &[], None),
        ("O, K.", "O", &["K."], &[], None),
        ("St. John, Mary", "St. John", &["Mary"], &[], None),
        ("J.-Paul Sartre", "Sartre", &["J.-Paul"], &[], None),
        ("김, 민수", "김", &["민수"], &[], None),
        ("Smith, John, Jr.", "Smith", &["John"], &[], Some("Jr.")),
        ("Smith, Jr., John", "Smith", &["John"], &[], Some("Jr.")),
        ("Smith Jr., John", "Smith", &["John"], &[], Some("Jr.")),
        ("Smith, John III", "Smith", &["John"], &[], Some("III")),
        ("J. Smith III", "Smith", &["J."], &[], Some("III")),
        ("J. Smith, Jr.", "Smith", &["J."], &[], Some("Jr.")),
        ("J D Smith", "Smith", &["J", "D"], &[], None),
        ("J.D. Smith", "Smith", &["J.", "D."], &[], None),
        ("Smith J. D.", "Smith", &["J.", "D."], &[], None),
        (
            "van der Waals J D",
            "Waals",
            &["J", "D"],
            &["van", "der"],
            None,
        ),
        (
            "J D van der Waals",
            "Waals",
            &["J", "D"],
            &["van", "der"],
            None,
        ),
        ("J.‑P. Serre", "Serre", &["J.-P."], &[], None),
    ];

    #[test]
    fn should_parse_documented_forms_when_printed_names_have_explicit_structure() {
        for (raw, family, given, particles, suffix) in CASES {
            let parsed = parse_name(raw);
            assert_eq!(parsed.raw, *raw);
            assert!(
                has_parts(&parsed, family, given, particles),
                "{raw}: {parsed:?}"
            );
            assert!(
                parsed
                    .alternatives
                    .iter()
                    .any(|parts| parts.suffix.as_deref() == *suffix)
            );
            assert_eq!(parsed.unparsed_reason, None);
        }
    }

    #[test]
    fn should_keep_hyphenated_initials_when_a_given_name_has_multiple_components() {
        for (raw, initials) in [
            ("J.-P. Serre", vec!["j", "p"]),
            ("Serre, Jean-Paul", vec!["j", "p"]),
            ("Smith, Mary Anne", vec!["m", "a"]),
            ("J.D. Smith", vec!["j", "d"]),
            ("É. Noël", vec!["e"]),
            ("J.D.-P. Smith", vec!["j", "d", "p"]),
        ] {
            let parsed = parse_name(raw);
            assert_eq!(parsed.alternatives.len(), 1, "{raw}");
            assert_eq!(parsed.alternatives[0].initials(), initials, "{raw}");
        }
    }

    #[test]
    fn should_return_alternatives_when_unmarked_order_or_compound_boundaries_are_ambiguous() {
        for (raw, first, last) in [
            ("John Smith", "John", "Smith"),
            ("Wei Wang", "Wei", "Wang"),
            ("Wang Wei", "Wang", "Wei"),
            ("MARIA GARCIA", "MARIA", "GARCIA"),
            ("小明 王", "小明", "王"),
        ] {
            let parsed = parse_name(raw);
            assert!(parsed.is_ambiguous(), "{raw}: {parsed:?}");
            assert!(has_parts(&parsed, last, &[first], &[]));
            assert!(has_parts(&parsed, first, &[last], &[]));
        }
        let parsed = parse_name("Gabriel García Márquez");
        assert!(has_parts(&parsed, "García Márquez", &["Gabriel"], &[]));
        assert!(has_parts(&parsed, "Márquez", &["Gabriel", "García"], &[]));
        assert!(has_parts(&parsed, "Gabriel", &["García", "Márquez"], &[]));
        assert!(has_parts(&parsed, "Gabriel García", &["Márquez"], &[]));
    }

    #[test]
    fn should_preserve_word_and_initial_interpretations_when_compact_capitals_are_ambiguous() {
        for raw in ["Smith, JD", "JD Smith"] {
            let parsed = parse_name(raw);
            assert!(has_parts(&parsed, "Smith", &["JD"], &[]));
            assert!(has_parts(&parsed, "Smith", &["J", "D"], &[]));
            assert!(parsed.is_ambiguous());
        }
        let parsed = parse_name("SMITH, ANN");
        assert!(has_parts(&parsed, "SMITH", &["ANN"], &[]));
        assert!(has_parts(&parsed, "SMITH", &["A", "N", "N"], &[]));
        let parsed = parse_name("NG, Ann");
        assert!(has_parts(&parsed, "NG", &["Ann"], &[]));
        assert_eq!(parsed.alternatives.len(), 1);
    }

    #[test]
    fn should_match_candidate_keys_when_accents_or_transliterations_vary() {
        for (left, right) in [
            ("Müller, Johann", "Mueller, J."),
            ("Mu\u{308}ller, Johann", "Mueller, J."),
            ("MÜLLER, JOHANN", "Muller, J."),
            ("Gödel, Kurt", "Goedel, K."),
            ("Händel, Georg", "Haendel, G."),
            ("Straße, Hans", "Strasse, H."),
            ("Łukasiewicz, Jan", "Lukasiewicz, J."),
            ("Søren, Anne", "Soren, A."),
            ("Noël, Émile", "Noel, Emile"),
            ("García Márquez, Gabriel", "Garcia-Marquez, G."),
            ("O’Neill, Eugene", "ONeill, E."),
            ("van der Waals, Johannes", "Waals, J."),
            ("王, 小明", "王, 小华"),
            ("Σωκράτης, Άννα", "ΣΩΚΡΆΤΗΣ, ΑΝΝΑ"),
            ("Smith, Ægir", "Smith, Aegir"),
        ] {
            let a = parse_name(left).blocking_keys();
            let b = parse_name(right).blocking_keys();
            assert!(!a.is_disjoint(&b), "{left} / {right}: {a:?} / {b:?}");
        }
    }

    #[test]
    fn should_preserve_distinct_evidence_when_coarse_keys_collide() {
        for (left, right) in [
            ("Smith, John", "Smith, Jane"),
            ("Smith, John, Jr.", "Smith, John, Sr."),
            ("Müller, Johann", "Muller, James"),
        ] {
            let a = parse_name(left);
            let b = parse_name(right);
            assert!(!a.blocking_keys().is_disjoint(&b.blocking_keys()));
            assert_ne!(a.raw, b.raw);
            assert_ne!(a.alternatives, b.alternatives);
        }
        assert!(
            parse_name("Smith, John")
                .blocking_keys()
                .is_disjoint(&parse_name("Smith, Anne").blocking_keys())
        );
        assert!(
            parse_name("Smith, John")
                .blocking_keys()
                .is_disjoint(&parse_name("Jones, John").blocking_keys())
        );
    }

    #[test]
    fn should_pair_keys_per_interpretation_when_name_order_is_ambiguous() {
        let keys = parse_name("Alice Brown").blocking_keys();
        assert_eq!(
            keys,
            BTreeSet::from([
                NameBlockingKey {
                    family: "alice".into(),
                    first_initial: "b".into()
                },
                NameBlockingKey {
                    family: "brown".into(),
                    first_initial: "a".into()
                },
            ])
        );
    }

    #[test]
    fn should_keep_mixed_word_and_initial_options_when_multiple_capital_groups_occur() {
        let parsed = parse_name("Smith, JD ANN");
        for given in [
            vec!["JD", "ANN"],
            vec!["J", "D", "ANN"],
            vec!["JD", "A", "N", "N"],
            vec!["J", "D", "A", "N", "N"],
        ] {
            assert!(has_parts(&parsed, "Smith", &given, &[]));
        }
        assert_eq!(parsed.alternatives.len(), 4);
    }

    #[test]
    fn should_leave_all_candidates_unresolved_when_ambiguity_exceeds_the_explicit_bound() {
        let parsed = parse_name("Smith, AB CD EF GH IJ KL MN OP QR");
        assert_eq!(
            parsed.unparsed_reason,
            Some(UnparsedReason::TooManyAlternatives)
        );
        assert!(parsed.alternatives.is_empty());
        assert!(parsed.blocking_keys().is_empty());
    }

    #[test]
    fn should_keep_suffix_and_initial_alternatives_when_a_roman_suffix_is_unmarked() {
        let parsed = parse_name("Smith, John V");
        assert!(
            parsed
                .alternatives
                .iter()
                .any(|p| p.given_names == ["John"] && p.suffix.as_deref() == Some("V"))
        );
        assert!(
            parsed
                .alternatives
                .iter()
                .any(|p| p.given_names == ["John", "V"] && p.suffix.is_none())
        );
        let explicit = parse_name("Smith, John, V");
        assert_eq!(explicit.alternatives.len(), 1);
        assert_eq!(explicit.alternatives[0].suffix.as_deref(), Some("V"));
    }

    #[test]
    fn should_preserve_raw_unicode_when_components_are_canonically_normalized() {
        let raw = " \tMu\u{308}ller,\u{a0}E\u{301}mile  ";
        let parsed = parse_name(raw);
        assert_eq!(parsed.raw, raw);
        assert!(has_parts(&parsed, "Müller", &["Émile"], &[]));
        let json = serde_json::to_string(&parsed).unwrap();
        assert_eq!(serde_json::from_str::<ParsedName>(&json).unwrap(), parsed);
        assert_eq!(parse_name(raw), parsed);
    }

    #[test]
    fn should_avoid_wildcard_keys_when_given_names_are_unknown() {
        for raw in ["Müller", "王小明", "Cher"] {
            let parsed = parse_name(raw);
            assert_eq!(parsed.alternatives.len(), 1);
            assert_eq!(parsed.alternatives[0].order, NameOrder::Undivided);
            assert!(parsed.blocking_keys().is_empty());
        }
    }

    #[test]
    fn should_leave_input_unresolved_when_syntax_or_size_is_unsupported() {
        for raw in [
            "",
            "  ",
            "123",
            "Smith; Jones",
            "Smith & Jones",
            "Smith et al.",
            "Smith,",
            ", John",
            "A. B.",
            "Smith, John, Jane",
            "Smith (editor)",
            "..",
            "J.. Smith",
            "John -- Smith",
        ] {
            let parsed = parse_name(raw);
            assert!(parsed.alternatives.is_empty(), "{raw}: {parsed:?}");
            assert!(parsed.unparsed_reason.is_some());
            assert_eq!(parsed.raw, raw);
            assert!(parsed.blocking_keys().is_empty());
        }
        assert_eq!(
            parse_name(&"A".repeat(MAX_NAME_BYTES + 1)).unparsed_reason,
            Some(UnparsedReason::TooLong)
        );
        assert_eq!(
            parse_name(&"Ann ".repeat(MAX_NAME_WORDS + 1)).unparsed_reason,
            Some(UnparsedReason::TooManyWords)
        );
    }
}
