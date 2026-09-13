//! Deterministic title keys and conservative fuzzy candidate scores.
//!
//! These are candidate-generation inputs, not entity identity decisions. In
//! particular, accents and ordinary punctuation are intentionally folded; a
//! resolver must still consider identifiers, authors, years, and its truth set.

use std::collections::BTreeMap;
use std::iter::Peekable;
use std::str::Chars;

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Maximum normalized character count on either side of a fuzzy comparison.
///
/// Unequal titles above this bound receive zero similarity, bounding the trigram
/// multiset's work and memory. Exact nonempty matches remain comparable in linear
/// time, regardless of length.
pub const MAX_FUZZY_TITLE_CHARS: usize = 1_024;

/// Fold title spelling and presentation variants into a whitespace-stable key.
///
/// The key retains every word, including articles, negation, and repeated words.
/// Known LaTeX Greek letters, accents, and presentation commands are decoded;
/// unknown commands keep their backslash and name instead of disappearing.
/// Mathematical operators and parentheses remain significant. The result may
/// be empty for a title consisting only of whitespace or presentation markup.
#[must_use]
pub fn title_key(title: &str) -> String {
    let decoded = decode_latex(title);
    let mut key = String::with_capacity(decoded.len());
    let mut accent_base = false;
    let mut unknown_command = false;
    for character in decoded.nfkd() {
        if unknown_command && character.is_ascii_alphabetic() {
            key.push(character);
            continue;
        }
        unknown_command = false;
        if is_combining_mark(character) {
            // Negated mathematical relations decompose into an operator plus
            // U+0338. Removing that mark would turn ≠ into = and ∉ into ∈.
            if accent_base && character != '\u{0338}' {
                continue;
            }
        } else {
            accent_base = character.is_alphabetic();
        }
        for folded in character.to_lowercase() {
            match folded {
                'ß' => key.push_str("ss"),
                'ς' => key.push('σ'),
                '\\' => {
                    space(&mut key);
                    key.push('\\');
                    unknown_command = true;
                }
                '\u{00ad}' | '\u{200b}' | '\u{feff}' => {}
                character
                    if character.is_whitespace()
                        || character.is_control()
                        || separator(character) =>
                {
                    space(&mut key);
                }
                character if character.is_alphanumeric() => key.push(character),
                character => {
                    space(&mut key);
                    key.push(character);
                    space(&mut key);
                }
            }
        }
    }
    key.trim().to_owned()
}

/// Multiset character-trigram Sørensen–Dice similarity in the inclusive 0–1 range.
///
/// Keys receive two boundary sentinels at each end, so even one-character titles
/// have trigrams. Repeated grams retain their counts. Zero means no shared grams
/// (or empty/oversized input). A score of one can also arise from distinct strings
/// with identical gram multisets; compare title keys for exact equality. This
/// symmetric candidate score is not a match probability or a merge threshold.
#[must_use]
pub fn title_similarity(left: &str, right: &str) -> f64 {
    let left = title_key(left);
    let right = title_key(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let left: Vec<_> = left.chars().take(MAX_FUZZY_TITLE_CHARS + 1).collect();
    let right: Vec<_> = right.chars().take(MAX_FUZZY_TITLE_CHARS + 1).collect();
    if left.len() > MAX_FUZZY_TITLE_CHARS || right.len() > MAX_FUZZY_TITLE_CHARS {
        return 0.0;
    }
    let left = trigrams(&left);
    let right = trigrams(&right);
    let shared: u32 = left
        .iter()
        .map(|(gram, count)| (*count).min(right.get(gram).copied().unwrap_or(0)))
        .sum();
    let total = left.values().sum::<u32>() + right.values().sum::<u32>();
    2.0 * f64::from(shared) / f64::from(total)
}

fn trigrams(characters: &[char]) -> BTreeMap<[char; 3], u32> {
    // NUL cannot occur inside a title key: controls normalize to whitespace.
    let padded: Vec<_> = ['\0', '\0']
        .into_iter()
        .chain(characters.iter().copied())
        .chain(['\0', '\0'])
        .collect();
    let mut grams = BTreeMap::new();
    for window in padded.windows(3) {
        *grams.entry([window[0], window[1], window[2]]).or_default() += 1;
    }
    grams
}

fn space(output: &mut String) {
    if !output.is_empty() && !output.ends_with(' ') {
        output.push(' ');
    }
}

const fn separator(character: char) -> bool {
    matches!(
        character,
        '.' | ','
            | ':'
            | ';'
            | '!'
            | '?'
            | '\''
            | '"'
            | '-'
            | '…'
            | '‘'
            | '’'
            | '‚'
            | '‛'
            | '“'
            | '”'
            | '„'
            | '‟'
            | '«'
            | '»'
            | '‹'
            | '›'
            | '‐'
            | '‑'
            | '‒'
            | '–'
            | '—'
            | '―'
    )
}

fn decode_latex(title: &str) -> String {
    let mut characters = title.chars().peekable();
    let mut output = String::with_capacity(title.len());
    let mut math = false;
    let mut presentation_group = false;
    let mut semantic_group = false;
    let mut groups = Vec::new();
    while let Some(character) = characters.next() {
        if let Some((kind, digit)) = script_digit(character) {
            output.push(kind);
            output.push('{');
            output.push(digit);
            while let Some((next_kind, next_digit)) =
                characters.peek().copied().and_then(script_digit)
            {
                if next_kind != kind {
                    break;
                }
                characters.next();
                output.push(next_digit);
            }
            output.push('}');
            presentation_group = false;
            continue;
        }
        match character {
            '$' => {
                let double = characters.peek() == Some(&'$');
                if double {
                    characters.next();
                }
                if math || characters.clone().any(|character| character == '$') {
                    math = !math;
                } else {
                    // An unmatched dollar may be currency, not TeX syntax.
                    output.push('$');
                    if double {
                        output.push('$');
                    }
                }
            }
            '\\' => presentation_group = decode_command(&mut characters, &mut output, &mut math),
            '{' => {
                let preserve = semantic_group
                    || (math && !presentation_group && groups.last().copied().unwrap_or(true));
                groups.push(preserve);
                if preserve {
                    output.push('{');
                }
                presentation_group = false;
                semantic_group = false;
            }
            '}' => {
                if groups.pop().unwrap_or(math) {
                    output.push('}');
                }
                presentation_group = false;
            }
            '^' | '_' => {
                output.push(character);
                skip_whitespace(&mut characters);
                if characters.peek() == Some(&'{') {
                    semantic_group = true;
                } else {
                    output.push('{');
                    match characters.next() {
                        Some('\\') => {
                            decode_command(&mut characters, &mut output, &mut math);
                        }
                        Some(atom) => output.push(atom),
                        None => {}
                    }
                    output.push('}');
                }
                presentation_group = false;
            }
            '~' => output.push(' '),
            '-' if math => output.push('−'),
            character => {
                output.push(character);
                if !character.is_whitespace() {
                    presentation_group = false;
                }
            }
        }
    }
    output
}

fn decode_command(
    characters: &mut Peekable<Chars<'_>>,
    output: &mut String,
    math: &mut bool,
) -> bool {
    let Some(next) = characters.next() else {
        output.push('\\');
        return false;
    };
    if !next.is_ascii_alphabetic() {
        match next {
            '(' | '[' => *math = true,
            ')' | ']' => *math = false,
            '\\' | ' ' | ',' | ';' | ':' => output.push(' '),
            '\'' | '"' | '`' | '^' | '~' | '=' | '.' => return true,
            '!' => {}
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => output.push(next),
            character => {
                output.push('\\');
                output.push(character);
            }
        }
        return false;
    }
    let mut command = String::from(next);
    while characters.peek().is_some_and(char::is_ascii_alphabetic) {
        if let Some(character) = characters.next() {
            command.push(character);
        }
    }
    if let Some(replacement) = command_text(&command) {
        output.push_str(replacement);
        let separated = skip_whitespace(characters);
        if separated && !replacement.is_empty() {
            space(output);
        }
        replacement.is_empty()
    } else {
        output.push('\\');
        output.push_str(&command);
        output.push(' ');
        skip_whitespace(characters);
        // Unknown argument boundaries may be semantic: do not equate
        // \frac{a}{bc} with \frac{ab}{c}. No recursive parsing is needed.
        while characters.peek() == Some(&'{') {
            retain_group(characters, output);
            output.push(' ');
            skip_whitespace(characters);
        }
        false
    }
}

fn skip_whitespace(characters: &mut Peekable<Chars<'_>>) -> bool {
    let mut skipped = false;
    while characters
        .peek()
        .is_some_and(|character| character.is_whitespace())
    {
        characters.next();
        skipped = true;
    }
    skipped
}

fn retain_group(characters: &mut Peekable<Chars<'_>>, output: &mut String) {
    let mut depth = 0;
    for character in characters.by_ref() {
        output.push(character);
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
}

fn command_text(command: &str) -> Option<&'static str> {
    Some(match command {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" | "varkappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "omicron" => "ο",
        "pi" | "varpi" => "π",
        "rho" | "varrho" => "ρ",
        "sigma" | "varsigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "ell" => "ℓ",
        "imath" => "i",
        "jmath" => "j",
        "times" => "×",
        "cdot" => "·",
        "pm" => "±",
        "mp" => "∓",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "cup" => "∪",
        "cap" => "∩",
        "forall" => "∀",
        "exists" => "∃",
        "rightarrow" | "to" => "→",
        "leftarrow" => "←",
        "leftrightarrow" => "↔",
        "ldots" | "dots" => "…",
        "ae" => "æ",
        "AE" => "Æ",
        "oe" => "œ",
        "OE" => "Œ",
        "ss" => "ß",
        "o" => "ø",
        "O" => "Ø",
        "l" => "ł",
        "L" => "Ł",
        "quad" | "qquad" | "enspace" | "thinspace" | "space" | "newline" => " ",
        "text" | "textrm" | "textnormal" | "textit" | "textbf" | "textsf" | "texttt" | "mathrm"
        | "mathbf" | "mathit" | "mathsf" | "mathtt" | "mathcal" | "mathbb" | "mathnormal"
        | "boldsymbol" | "operatorname" | "mbox" | "emph" | "displaystyle" | "textstyle"
        | "scriptstyle" | "scriptscriptstyle" | "left" | "right" | "v" | "H" | "c" | "k" | "r"
        | "u" | "b" | "d" => "",
        _ => return None,
    })
}

const fn script_digit(character: char) -> Option<(char, char)> {
    Some(match character {
        '⁰' => ('^', '0'),
        '¹' => ('^', '1'),
        '²' => ('^', '2'),
        '³' => ('^', '3'),
        '⁴' => ('^', '4'),
        '⁵' => ('^', '5'),
        '⁶' => ('^', '6'),
        '⁷' => ('^', '7'),
        '⁸' => ('^', '8'),
        '⁹' => ('^', '9'),
        '₀' => ('_', '0'),
        '₁' => ('_', '1'),
        '₂' => ('_', '2'),
        '₃' => ('_', '3'),
        '₄' => ('_', '4'),
        '₅' => ('_', '5'),
        '₆' => ('_', '6'),
        '₇' => ('_', '7'),
        '₈' => ('_', '8'),
        '₉' => ('_', '9'),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{MAX_FUZZY_TITLE_CHARS, title_key, title_similarity};

    #[test]
    fn should_normalize_common_variants_when_titles_use_different_presentation() {
        let pairs = [
            (r"$\alpha$-Divergence", "α-divergence"),
            ("Title: Subtitle", "Title - Subtitle"),
            ("Title — Subtitle", "Title: Subtitle"),
            ("Title – Subtitle", "Title: Subtitle"),
            ("‘Attention’ and “Learning”", "'Attention' and \"Learning\""),
            ("Attention Is All You Need.", "attention is all you need"),
            ("Über naïve café models", "Uber naive cafe models"),
            ("Cafe\u{301} models", "Café models"),
            ("Ｆｕｌｌｗｉｄｔｈ Learning", "Fullwidth Learning"),
            ("Eﬃcient ﬁltering", "Efficient filtering"),
            ("𝕃earning 𝐌odels", "Learning Models"),
            ("Straße and STRAẞE", "strasse and strasse"),
            ("ΟΣ and ος", "οσ and οσ"),
            ("Deep\u{00a0}Learning\tModels\n", "Deep Learning Models"),
            ("Soft\u{00ad}ware", "Software"),
            (r"\textbf{Deep} \emph{Learning}", "Deep Learning"),
            (r"{Deep} {Learning}", "Deep Learning"),
            (r"infor{ma}tion theory", "information theory"),
            (r#"G{\"o}del's theorem"#, "Gödel’s theorem"),
            (r"Fran\c{c}ois and \v{S}imek", "François and Šimek"),
            (r"\'{E}tude and \`a", "Étude and à"),
            (r"\H{o} and \~n", "ő and ñ"),
            (r"\ss{} and \o{} and \ae{}", "ß and ø and æ"),
            (r"\(\beta\)-Models", "β-models"),
            (r"\[\Gamma\] Methods", "Γ Methods"),
            (r"$$\Delta$$ Methods", "Δ Methods"),
            (r"$\mathbb{R}$ Models", "ℝ Models"),
            (r"$\ell_2$ regularization", "ℓ₂ regularization"),
            (r"$x^{23}$ methods", "x²³ methods"),
            (r"$x_{23}$ methods", "x₂₃ methods"),
            (r"$x^2$ methods", "x² methods"),
            (r"$x_2$ methods", "x₂ methods"),
            (r"\alpha Learning", "α Learning"),
            (r"\v S Models", "Š Models"),
            (r"$\textbf{{Deep}}$ Learning", "Deep Learning"),
            (r"$a\leq b$", "a ≤ b"),
            (r"$a\neq b$", "a ≠ b"),
            (r"$a\times b$", "a × b"),
            (r"$\sum_i x_i$", "∑_i x_i"),
            (r"$x-y$", "x−y"),
            (r"Research\&Development", "Research & Development"),
            (r"A~B\quad C", "A B C"),
            (r"A\\B", "A B"),
        ];
        for (left, right) in pairs {
            assert_eq!(
                title_key(left),
                title_key(right),
                "{left:?} versus {right:?}"
            );
            assert_eq!(title_similarity(left, right).to_bits(), 1.0_f64.to_bits());
        }
    }

    #[test]
    fn should_preserve_distinct_content_when_titles_share_words_or_prefixes() {
        let pairs = [
            ("Attention Is All You Need", "Attention Is Not All You Need"),
            ("A Theory of Learning", "A Theory of Learning with Noise"),
            ("The Learning Model", "Learning Model"),
            ("Learning Learning Models", "Learning Models"),
            ("Models for Learning", "Learning for Models"),
            ("Deep Learning", "Deeplearning"),
            ("Model 2", "Model 3"),
            ("C++ Learning", "C Learning"),
            ("A+B", "A−B"),
            ("a ≤ b", "a ≥ b"),
            ("a ≠ b", "a = b"),
            ("a ∉ b", "a ∈ b"),
            ("σ\u{0338}", "σ"),
            ("A $5 Model", "A 5 Model"),
            ("x²", "x2"),
            ("x₂", "x²"),
            (r"\unknown{Learning}", "Learning"),
            (r"\unknown{Learning}", "unknown Learning"),
            (r"\unknown{Learning}", r"\different{Learning}"),
            (r"\unknown{Learning}", r"\Unknown{Learning}"),
            (r"\frac{a}{b}", "ab"),
            (r"$\frac{a}{bc}$", r"$\frac{ab}{c}$"),
            (r"$x^{ab}$", r"$x^a b$"),
            (r"$x^{23}$", r"$x^23$"),
        ];
        for (left, right) in pairs {
            assert_ne!(
                title_key(left),
                title_key(right),
                "{left:?} versus {right:?}"
            );
            assert!(title_similarity(left, right) < 1.0);
        }
    }

    #[test]
    fn should_keep_word_boundaries_when_punctuation_is_folded() {
        assert_eq!(title_key("  Title:  Subtitle...  "), "title subtitle");
        assert_eq!(title_key("state-of-the-art"), "state of the art");
        assert_ne!(title_key("ab cd"), title_key("a bcd"));
        assert_eq!(title_key("C++"), "c + +");
        assert_eq!(title_key(r"\notknown{abc}"), r"\notknown { abc }");
    }

    #[test]
    fn should_keep_keys_stable_when_normalizing_an_existing_key() {
        for title in [
            "α-Divergence.",
            "C++ and x²",
            "Über a model",
            "a ≤ b",
            "A (B)",
            r"\unknown{Learning}",
        ] {
            let key = title_key(title);
            assert_eq!(title_key(&key), key, "{title}");
        }
    }

    #[test]
    fn should_score_shared_trigrams_symmetrically_when_titles_differ() {
        for (left, right, expected) in [
            ("cat", "cut", 0.4),
            ("cat", "cats", 6.0 / 11.0),
            ("αβ", "αγ", 0.25),
        ] {
            assert!((title_similarity(left, right) - expected).abs() < 1e-12);
            assert_eq!(
                title_similarity(left, right).to_bits(),
                title_similarity(right, left).to_bits()
            );
        }
        assert!(
            title_similarity("Learning Models", "Learning Model")
                > title_similarity("Learning Models", "Quantum Physics")
        );
    }

    #[test]
    fn should_keep_exact_keys_distinct_when_fuzzy_trigram_multisets_collide() {
        let left = "abc xxx abc yyy abc";
        let right = "abc yyy abc xxx abc";
        assert_ne!(title_key(left), title_key(right));
        assert_eq!(title_similarity(left, right).to_bits(), 1.0_f64.to_bits());
        // Repeated grams count, so repeating a word changes the fuzzy score.
        assert!(title_similarity("learning learning", "learning") < 1.0);
    }

    #[test]
    fn should_withhold_similarity_when_either_title_has_no_content() {
        for blank in ["", "   ", "...", r"\textbf{}", "{}$ $"] {
            assert_eq!(title_key(blank), "");
            assert_eq!(title_similarity(blank, blank).to_bits(), 0.0_f64.to_bits());
            assert_eq!(
                title_similarity(blank, "Learning").to_bits(),
                0.0_f64.to_bits()
            );
            assert_eq!(
                title_similarity("Learning", blank).to_bits(),
                0.0_f64.to_bits()
            );
        }
    }

    #[test]
    fn should_bound_fuzzy_work_when_titles_exceed_the_character_limit() {
        let long = "α".repeat(MAX_FUZZY_TITLE_CHARS + 1);
        assert_eq!(title_similarity(&long, &long).to_bits(), 1.0_f64.to_bits());
        assert_eq!(
            title_similarity(&long, &(long.clone() + "β")).to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(title_similarity(&long, "α").to_bits(), 0.0_f64.to_bits());
        let boundary = "α".repeat(MAX_FUZZY_TITLE_CHARS);
        assert!(title_similarity(&boundary, &("α".repeat(MAX_FUZZY_TITLE_CHARS - 1) + "β")) > 0.99);
    }

    #[test]
    fn should_finish_without_recursion_when_markup_is_malformed_or_deeply_nested() {
        for (title, expected) in [
            (r"\", r"\"),
            (r"\unknown{", r"\unknown {"),
            ("{{{{", ""),
            ("$$x", "$ $ x"),
            (r"\textbf{unfinished", "unfinished"),
        ] {
            assert_eq!(title_key(title), expected);
        }
        let nested = "{".repeat(20_000) + "Learning" + &"}".repeat(20_000);
        assert_eq!(title_key(&nested), "learning");
    }
}
