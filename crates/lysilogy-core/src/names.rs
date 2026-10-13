//! Pure helpers for author-name text.

/// Family name of the first author in a free-text author list.
///
/// Takes the segment before the first `,`, ` and `, `&`, ` et al`, or `;`, then
/// its last whitespace-separated word with trailing periods removed. Returns
/// `None` when nothing alphabetic remains. Diacritics are preserved.
#[must_use]
pub fn first_author_family(raw: &str) -> Option<String> {
    let first_segment = [",", " and ", "&", " et al", ";"]
        .iter()
        .filter_map(|separator| raw.find(separator))
        .min()
        .map_or(raw, |end| &raw[..end]);
    let last_word = first_segment.split_whitespace().next_back()?;
    let family = last_word.trim_end_matches('.');
    if !family.chars().any(char::is_alphabetic) {
        return None;
    }
    Some(family.to_owned())
}

const PARTICLES: &[&str] = &[
    "van", "von", "de", "del", "della", "da", "dos", "das", "la", "der", "den",
];
const MAX_AUTHOR_WORDS: usize = 6;

/// Family name of one printed author, without a full name parser.
///
/// `Family, I.` yields the part before the comma. Otherwise the family is the
/// last word together with any directly preceding lowercase particles
/// (`van der Waals`). Returns `None` when the result has digits or no letters,
/// or when the author has more than six words.
#[must_use]
pub fn citation_family(author: &str) -> Option<String> {
    let author = author.trim();
    let family = if let Some((before, _)) = author.split_once(',') {
        before.trim().to_owned()
    } else {
        let words: Vec<&str> = author.split_whitespace().collect();
        let last = words.len().checked_sub(1)?;
        let mut first = last;
        while first > 0 && PARTICLES.contains(&words[first - 1]) {
            first -= 1;
        }
        words[first..].join(" ").trim_end_matches('.').to_owned()
    };
    if author.split_whitespace().count() > MAX_AUTHOR_WORDS
        || family.chars().any(|ch| ch.is_ascii_digit())
        || !family.chars().any(char::is_alphabetic)
    {
        return None;
    }
    Some(family)
}

/// Whether a printed author string plausibly names a person.
#[must_use]
pub fn looks_like_person_name(author: &str) -> bool {
    let author = author.trim_end_matches('.');
    !author.contains([':', '/']) && citation_family(author).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_extract_family_when_author_list_uses_commas_and_et_al() {
        assert_eq!(
            first_author_family("Autor, Dorn, and Hanson").as_deref(),
            Some("Autor")
        );
        assert_eq!(first_author_family("Bai et al.").as_deref(), Some("Bai"));
        assert_eq!(
            first_author_family("David H. Autor").as_deref(),
            Some("Autor")
        );
        assert_eq!(
            first_author_family("Smith and Jones").as_deref(),
            Some("Smith")
        );
        assert_eq!(
            first_author_family("Smith & Jones").as_deref(),
            Some("Smith")
        );
        assert_eq!(
            first_author_family("Smith; Jones").as_deref(),
            Some("Smith")
        );
        assert_eq!(first_author_family("J. Smith Jr.").as_deref(), Some("Jr"));
    }

    #[test]
    fn should_return_none_when_author_text_is_blank_or_only_et_al() {
        assert_eq!(first_author_family(""), None);
        assert_eq!(first_author_family("   "), None);
        assert_eq!(first_author_family(" et al."), None);
        assert_eq!(first_author_family(", Smith"), None);
        assert_eq!(first_author_family("123."), None);
        assert_eq!(first_author_family("..."), None);
    }

    #[test]
    fn should_keep_diacritics_when_family_is_non_ascii() {
        assert_eq!(
            first_author_family("Ţigănaşu, Bănică, and Wong").as_deref(),
            Some("Ţigănaşu")
        );
        assert_eq!(first_author_family("Émile Zola").as_deref(), Some("Zola"));
    }

    #[test]
    fn should_return_family_before_comma_when_author_is_inverted() {
        assert_eq!(citation_family("Smith, J.").as_deref(), Some("Smith"));
        assert_eq!(
            citation_family("van der Waals, J. D.").as_deref(),
            Some("van der Waals")
        );
        assert_eq!(citation_family("García, E.").as_deref(), Some("García"));
    }

    #[test]
    fn should_keep_particles_when_family_has_van_der() {
        assert_eq!(
            citation_family("Johannes van der Waals").as_deref(),
            Some("van der Waals")
        );
        assert_eq!(
            citation_family("A. de la Cruz").as_deref(),
            Some("de la Cruz")
        );
        assert_eq!(citation_family("J.-P. Serre").as_deref(), Some("Serre"));
    }

    #[test]
    fn should_return_last_word_when_author_is_given_family_order() {
        assert_eq!(citation_family("John Smith").as_deref(), Some("Smith"));
        assert_eq!(
            citation_family("Ashish Vaswani").as_deref(),
            Some("Vaswani")
        );
    }

    #[test]
    fn should_return_none_when_author_has_digits_or_no_letters() {
        assert_eq!(citation_family("2020"), None);
        assert_eq!(citation_family("Smith2"), None);
        assert_eq!(citation_family("..."), None);
        assert_eq!(citation_family(""), None);
        assert_eq!(citation_family("a b c d e f g"), None);
    }

    #[test]
    fn should_recognize_person_names_when_text_is_author_shaped() {
        assert!(looks_like_person_name("Example, A."));
        assert!(looks_like_person_name("Ada Example"));
        assert!(!looks_like_person_name("2021"));
        assert!(!looks_like_person_name("https://example.org/x"));
    }
}
