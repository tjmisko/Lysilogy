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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_extract_family_when_author_list_uses_commas_and_et_al() {
        assert_eq!(first_author_family("Autor, Dorn, and Hanson").as_deref(), Some("Autor"));
        assert_eq!(first_author_family("Bai et al.").as_deref(), Some("Bai"));
        assert_eq!(first_author_family("David H. Autor").as_deref(), Some("Autor"));
        assert_eq!(first_author_family("Smith and Jones").as_deref(), Some("Smith"));
        assert_eq!(first_author_family("Smith & Jones").as_deref(), Some("Smith"));
        assert_eq!(first_author_family("Smith; Jones").as_deref(), Some("Smith"));
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
}
