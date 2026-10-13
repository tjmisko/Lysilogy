//! The paper's own title and authors, accepted only when its front matter
//! confirms them.
//!
//! The hints come from the catalog (file name, PDF info dictionary, layout
//! guesses) and are unreliable. A hint is kept only when the printed front
//! matter contains it, which makes the result `paper`-tier evidence.
//! No year is read: dates on the first pages are usually version dates.

use lysilogy_core::title_key;

use crate::identify::PaperHints;

/// Title keys need at least this many words to be trusted as a substring match.
const MIN_TITLE_KEY_WORDS: usize = 3;

/// Author names need at least this many words (rejects file-name surnames).
const MIN_AUTHOR_NAME_WORDS: usize = 2;

/// The fields the front matter confirmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaperFields {
    pub title: Option<String>,
    /// Printed author names in order; empty when not confirmed.
    pub authors: Vec<String>,
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn confirmed_title(front_matter: &str, hint: Option<&str>) -> Option<String> {
    let title = collapse_whitespace(hint?);
    let key = title_key(&title);
    if key.split_whitespace().count() < MIN_TITLE_KEY_WORDS {
        return None;
    }
    title_key(front_matter).contains(&key).then_some(title)
}

fn confirmed_authors(front_matter: &str, hints: &[String]) -> Vec<String> {
    let names: Vec<String> = hints.iter().map(|name| collapse_whitespace(name)).collect();
    if names.is_empty() {
        return Vec::new();
    }
    let collapsed = collapse_whitespace(front_matter);
    let confirmed = names.iter().all(|name| {
        name.split_whitespace().count() >= MIN_AUTHOR_NAME_WORDS && collapsed.contains(name)
    });
    if confirmed { names } else { Vec::new() }
}

/// The title and authors from `hints` that `front_matter` confirms.
#[must_use]
pub fn front_matter_fields(front_matter: &str, hints: PaperHints<'_>) -> PaperFields {
    PaperFields {
        title: confirmed_title(front_matter, hints.title),
        authors: confirmed_authors(front_matter, hints.authors),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONT: &str = "Deep Reinforcement Learning\nfrom Human Preferences\n\n\
        Paul F. Christiano\nJan Leike   Tom B. Brown\nOpenAI\nAbstract";

    fn hints<'a>(title: Option<&'a str>, authors: &'a [String]) -> PaperHints<'a> {
        PaperHints {
            title,
            authors,
            ..PaperHints::default()
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn should_accept_title_when_present_in_front_matter() {
        let fields = front_matter_fields(
            FRONT,
            hints(
                Some("Deep Reinforcement  Learning from Human Preferences"),
                &[],
            ),
        );
        assert_eq!(
            fields.title.as_deref(),
            Some("Deep Reinforcement Learning from Human Preferences")
        );
    }

    #[test]
    fn should_reject_title_when_absent_or_too_short() {
        for title in ["A Different Title Entirely", "Human Preferences", "", "  "] {
            let fields = front_matter_fields(FRONT, hints(Some(title), &[]));
            assert_eq!(fields.title, None, "{title:?}");
        }
        assert_eq!(front_matter_fields(FRONT, hints(None, &[])).title, None);
    }

    #[test]
    fn should_accept_authors_when_every_name_occurs() {
        let authors = names(&["Paul F. Christiano", "Jan Leike", "Tom B. Brown"]);
        let fields = front_matter_fields(FRONT, hints(None, &authors));
        assert_eq!(fields.authors, authors);
    }

    #[test]
    fn should_reject_authors_when_a_name_is_missing_or_single_word() {
        let missing = names(&["Paul F. Christiano", "Ilya Sutskever"]);
        assert!(
            front_matter_fields(FRONT, hints(None, &missing))
                .authors
                .is_empty()
        );
        let single = names(&["Christiano", "Jan Leike"]);
        assert!(
            front_matter_fields(FRONT, hints(None, &single))
                .authors
                .is_empty()
        );
        assert!(
            front_matter_fields(FRONT, hints(None, &[]))
                .authors
                .is_empty()
        );
    }
}
