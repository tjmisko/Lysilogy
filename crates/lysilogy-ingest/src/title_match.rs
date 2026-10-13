//! Pure selection of a provider record for a local title, guarding against the
//! duplicate and mis-merged records `OpenAlex` is known to return.

use lysilogy_core::{first_author_family, title_key, title_similarity};

use crate::openalex::OpenAlexSummary;

/// A provider record's title must be at least this similar to a known local
/// title to be trusted.
pub const PROVIDER_TITLE_MIN_SIMILARITY: f64 = 0.6;

/// The fewest title key words that make a title specific enough to match on.
const MIN_TITLE_KEY_WORDS: usize = 3;

/// What the local paper claims about itself.
#[derive(Clone, Copy, Debug)]
pub struct TitleQuery<'a> {
    pub title: &'a str,
    pub year: Option<i32>,
    pub first_author: Option<&'a str>,
}

/// Why no candidate was accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleRejection {
    TitleTooGeneric,
    NoCandidates,
    NoExactTitle,
    YearMismatch,
    AuthorMismatch,
}

impl TitleRejection {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TitleTooGeneric => "title_too_generic",
            Self::NoCandidates => "no_candidates",
            Self::NoExactTitle => "no_exact_title",
            Self::YearMismatch => "year_mismatch",
            Self::AuthorMismatch => "author_mismatch",
        }
    }
}

/// The outcome of [`select_title_match`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TitleMatch {
    Accepted {
        candidate: OpenAlexSummary,
        /// How many candidates passed every check before the tie-break.
        considered: usize,
    },
    Rejected(TitleRejection),
}

/// Choose the candidate that is the same paper as the query, or say why none is.
///
/// A candidate needs an equal title key, a year within one when both are known,
/// and a matching first-author family when both are known. Among survivors the
/// most-referenced record wins, then the smallest `OpenAlex` ID.
#[must_use]
pub fn select_title_match(query: &TitleQuery<'_>, candidates: &[OpenAlexSummary]) -> TitleMatch {
    let key = title_key(query.title);
    if key == "untitled" || key.split_whitespace().count() < MIN_TITLE_KEY_WORDS {
        return TitleMatch::Rejected(TitleRejection::TitleTooGeneric);
    }
    if candidates.is_empty() {
        return TitleMatch::Rejected(TitleRejection::NoCandidates);
    }
    let exact: Vec<&OpenAlexSummary> = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .title
                .as_deref()
                .is_some_and(|t| title_key(t) == key)
        })
        .collect();
    if exact.is_empty() {
        return TitleMatch::Rejected(TitleRejection::NoExactTitle);
    }
    let in_year: Vec<&OpenAlexSummary> = exact
        .into_iter()
        .filter(|candidate| match (query.year, candidate.year) {
            (Some(wanted), Some(found)) => (wanted - found).abs() <= 1,
            _ => true,
        })
        .collect();
    if in_year.is_empty() {
        return TitleMatch::Rejected(TitleRejection::YearMismatch);
    }
    let wanted_family = family_key(query.first_author);
    let survivors: Vec<&OpenAlexSummary> = in_year
        .into_iter()
        .filter(|candidate| {
            match (
                &wanted_family,
                family_key(candidate.first_author.as_deref()),
            ) {
                (Some(wanted), Some(found)) => *wanted == found,
                _ => true,
            }
        })
        .collect();
    let considered = survivors.len();
    let best = survivors.into_iter().max_by(|left, right| {
        left.referenced_works_count
            .unwrap_or(0)
            .cmp(&right.referenced_works_count.unwrap_or(0))
            .then_with(|| id_order(&right.openalex_id).cmp(&id_order(&left.openalex_id)))
    });
    best.map_or(
        TitleMatch::Rejected(TitleRejection::AuthorMismatch),
        |candidate| TitleMatch::Accepted {
            candidate: candidate.clone(),
            considered,
        },
    )
}

/// True only when both titles are present and clearly describe different works.
#[must_use]
pub fn titles_disagree(local: Option<&str>, provider: Option<&str>) -> bool {
    let (Some(local), Some(provider)) = (local, provider) else {
        return false;
    };
    if local.trim().is_empty() || provider.trim().is_empty() {
        return false;
    }
    title_similarity(local, provider) < PROVIDER_TITLE_MIN_SIMILARITY
}

fn family_key(author: Option<&str>) -> Option<String> {
    author
        .and_then(first_author_family)
        .map(|family| title_key(&family))
        .filter(|key| !key.is_empty())
}

/// Orders `W123` before `W1000`: shorter IDs first, then lexicographic.
const fn id_order(id: &str) -> (usize, &str) {
    (id.len(), id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, title: &str, year: i32, author: &str, refs: u32) -> OpenAlexSummary {
        OpenAlexSummary {
            openalex_id: id.to_owned(),
            doi: None,
            title: Some(title.to_owned()),
            year: Some(year),
            first_author: Some(author.to_owned()),
            authors: Vec::new(),
            referenced_works_count: Some(refs),
        }
    }

    fn query<'a>(title: &'a str, year: Option<i32>, author: Option<&'a str>) -> TitleQuery<'a> {
        TitleQuery {
            title,
            year,
            first_author: author,
        }
    }

    #[test]
    fn should_accept_exact_title_when_year_within_one() {
        let candidates = [summary(
            "W1",
            "The China Syndrome",
            2013,
            "David H. Autor",
            5,
        )];
        let result = select_title_match(
            &query(
                "The China Syndrome",
                Some(2014),
                Some("Autor, Dorn, and Hanson"),
            ),
            &candidates,
        );
        assert!(matches!(result, TitleMatch::Accepted { considered: 1, .. }));
        let far = select_title_match(&query("The China Syndrome", Some(2016), None), &candidates);
        assert_eq!(far, TitleMatch::Rejected(TitleRejection::YearMismatch));
    }

    #[test]
    fn should_pick_most_referenced_when_duplicate_records_match() {
        let candidates = [
            summary("W9", "The China Syndrome", 2013, "David Autor", 0),
            summary("W20", "The China Syndrome", 2013, "David Autor", 115),
            summary("W3", "The China Syndrome", 2013, "David Autor", 115),
        ];
        let TitleMatch::Accepted {
            candidate,
            considered,
        } = select_title_match(&query("The China Syndrome", Some(2013), None), &candidates)
        else {
            panic!("expected acceptance");
        };
        assert_eq!(candidate.openalex_id, "W3");
        assert_eq!(considered, 3);
    }

    #[test]
    fn should_reject_when_titles_differ_only_by_subtitle() {
        let candidates = [summary(
            "W1",
            "The China Syndrome Ails America Too",
            2013,
            "A B",
            1,
        )];
        let result =
            select_title_match(&query("The China Syndrome", Some(2013), None), &candidates);
        assert_eq!(result, TitleMatch::Rejected(TitleRejection::NoExactTitle));
    }

    #[test]
    fn should_reject_when_title_too_generic() {
        let candidates = [summary("W1", "Introduction", 2013, "A B", 1)];
        for title in ["Introduction", "Untitled", "", "Two words"] {
            let result = select_title_match(&query(title, None, None), &candidates);
            assert_eq!(
                result,
                TitleMatch::Rejected(TitleRejection::TitleTooGeneric),
                "{title}"
            );
        }
    }

    #[test]
    fn should_reject_when_no_candidates_returned() {
        let result = select_title_match(&query("Concrete Problems in AI Safety", None, None), &[]);
        assert_eq!(result, TitleMatch::Rejected(TitleRejection::NoCandidates));
    }

    #[test]
    fn should_reject_when_first_author_differs() {
        let candidates = [summary("W1", "The China Syndrome", 2013, "Jane Smith", 1)];
        let result = select_title_match(
            &query(
                "The China Syndrome",
                Some(2013),
                Some("Autor, Dorn, and Hanson"),
            ),
            &candidates,
        );
        assert_eq!(result, TitleMatch::Rejected(TitleRejection::AuthorMismatch));
    }

    #[test]
    fn should_accept_when_candidate_year_and_author_unknown() {
        let mut candidate = summary("W1", "The China Syndrome", 2013, "x", 1);
        candidate.year = None;
        candidate.first_author = None;
        let result = select_title_match(
            &query("The China Syndrome", Some(1990), Some("Autor")),
            &[candidate],
        );
        assert!(matches!(result, TitleMatch::Accepted { .. }));
    }

    #[test]
    fn should_name_rejections_in_snake_case() {
        assert_eq!(
            TitleRejection::TitleTooGeneric.as_str(),
            "title_too_generic"
        );
        assert_eq!(TitleRejection::NoCandidates.as_str(), "no_candidates");
        assert_eq!(TitleRejection::NoExactTitle.as_str(), "no_exact_title");
        assert_eq!(TitleRejection::YearMismatch.as_str(), "year_mismatch");
        assert_eq!(TitleRejection::AuthorMismatch.as_str(), "author_mismatch");
    }

    #[test]
    fn should_flag_disagreement_when_provider_title_unrelated() {
        assert!(titles_disagree(
            Some("Concrete Problems in AI Safety"),
            Some("Agnostic Learning with Unknown Utilities")
        ));
        assert!(!titles_disagree(
            Some("Concrete Problems in AI Safety"),
            Some("Concrete problems in AI safety.")
        ));
    }

    #[test]
    fn should_not_flag_disagreement_when_local_title_unknown() {
        assert!(!titles_disagree(None, Some("Anything")));
        assert!(!titles_disagree(Some("  "), Some("Anything")));
        assert!(!titles_disagree(Some("Anything"), None));
        assert!(!titles_disagree(Some("Anything"), Some("")));
    }
}
