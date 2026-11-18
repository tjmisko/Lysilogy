//! How closely the model's abstract agrees with the deterministic extraction.
//!
//! The model is asked to copy the authors' abstract; the deterministic locator
//! finds it from the page layout without a model. Neither is ground truth, but
//! when both exist and disagree, one of them is wrong, and a reviewer should
//! look. The score is the benchmark's token F1 (`scripts/spike/benchmark.py`):
//! alphabetic words of two or more letters, accents folded, as a multiset.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{AbstractResult, AbstractStatus};

/// Below this token F1 the two abstracts are reported as disagreeing. The
/// benchmark counts 0.95 as a match; the gap between leaves room for
/// hyphenation and ligature noise in the text layer.
pub const DISAGREEMENT_F1: f64 = 0.8;

/// The model's abstract against the deterministic extraction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbstractAgreement {
    /// Token F1 between the two texts; `None` when either is missing.
    pub token_f1: Option<f64>,
    /// The model returned an abstract.
    pub model_found: bool,
    /// The deterministic locator proposed a text, admitted or not.
    pub deterministic_found: bool,
    /// The deterministic locator's verdict on its own candidate.
    pub deterministic_status: AbstractStatus,
}

impl AbstractAgreement {
    /// Both texts exist and their token F1 is under [`DISAGREEMENT_F1`].
    #[must_use]
    pub fn disagrees(&self) -> bool {
        self.token_f1.is_some_and(|f1| f1 < DISAGREEMENT_F1)
    }
}

/// Compare the model's abstract with the deterministic result. The
/// deterministic text is its admitted text, else its unadmitted candidate.
#[must_use]
pub fn agreement(model: Option<&str>, deterministic: &AbstractResult) -> AbstractAgreement {
    let model = model.map(str::trim).filter(|text| !text.is_empty());
    let located = deterministic
        .text
        .as_deref()
        .or_else(|| {
            deterministic
                .candidate
                .as_ref()
                .map(|candidate| candidate.text.as_str())
        })
        .map(str::trim)
        .filter(|text| !text.is_empty());
    AbstractAgreement {
        token_f1: model
            .zip(located)
            .map(|(model, located)| token_f1(model, located)),
        model_found: model.is_some(),
        deterministic_found: located.is_some(),
        deterministic_status: deterministic.status.clone(),
    }
}

/// Token F1 over alphabetic words of two or more letters, accents folded and
/// lower-cased, counted as a multiset; 0 when nothing overlaps.
#[must_use]
pub fn token_f1(predicted: &str, truth: &str) -> f64 {
    let predicted = word_counts(predicted);
    let truth = word_counts(truth);
    let overlap: usize = predicted
        .iter()
        .map(|(word, count)| (*count).min(truth.get(word).copied().unwrap_or(0)))
        .sum();
    if overlap == 0 {
        return 0.0;
    }
    let total = |counts: &HashMap<String, usize>| counts.values().sum::<usize>();
    #[allow(clippy::cast_precision_loss)]
    let (overlap, predicted_total, truth_total) = (
        overlap as f64,
        total(&predicted) as f64,
        total(&truth) as f64,
    );
    let precision = overlap / predicted_total;
    let recall = overlap / truth_total;
    2.0 * precision * recall / (precision + recall)
}

fn word_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    let folded = fold(text);
    for word in folded
        .split(|character: char| !character.is_ascii_lowercase())
        .filter(|word| word.len() >= 2)
    {
        *counts.entry(word.to_owned()).or_insert(0) += 1;
    }
    counts
}

/// Lower-case ASCII with common accents and ligatures folded, so `é` counts
/// as `e` and `ﬁ` as `fi`.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for character in super::normalize(text).chars() {
        match character {
            'à'..='å' | 'À'..='Å' => folded.push('a'),
            'è'..='ë' | 'È'..='Ë' => folded.push('e'),
            'ì'..='ï' | 'Ì'..='Ï' | 'ı' => folded.push('i'),
            'ò'..='ö' | 'Ò'..='Ö' | 'ø' | 'Ø' => folded.push('o'),
            'ù'..='ü' | 'Ù'..='Ü' => folded.push('u'),
            'ç' | 'Ç' => folded.push('c'),
            'ñ' | 'Ñ' => folded.push('n'),
            'ß' => folded.push_str("ss"),
            'æ' | 'Æ' => folded.push_str("ae"),
            'œ' | 'Œ' => folded.push_str("oe"),
            'ł' | 'Ł' => folded.push('l'),
            other => folded.extend(other.to_lowercase()),
        }
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: Option<&str>, status: AbstractStatus) -> AbstractResult {
        AbstractResult {
            schema_version: 1,
            source_fingerprint: String::new(),
            status,
            text: text.map(str::to_owned),
            start_page: None,
            end_page: None,
            candidate: None,
            checks: Vec::new(),
            review: None,
        }
    }

    #[test]
    fn should_score_identical_abstracts_as_one_and_disjoint_ones_as_zero() {
        assert!(
            (token_f1("We show that pruning works.", "We show that pruning works.") - 1.0).abs()
                < 1e-9
        );
        assert!(token_f1("Pruning works.", "Attention suffices.").abs() < 1e-9);
    }

    #[test]
    fn should_ignore_case_accents_numbers_and_single_letters() {
        let f1 = token_f1("Café results: x = 3 for a model", "cafe RESULTS for model");
        assert!((f1 - 1.0).abs() < 1e-9, "{f1}");
    }

    #[test]
    fn should_report_disagreement_when_the_model_returns_a_summary() {
        let deterministic = result(
            Some(
                "We use a natural experiment to estimate the wage effect of a sudden migrant inflow on local workers.",
            ),
            AbstractStatus::Accepted,
        );
        let agreement = agreement(Some("This paper is about immigration."), &deterministic);
        assert!(agreement.model_found && agreement.deterministic_found);
        assert!(agreement.disagrees(), "{agreement:?}");
    }

    #[test]
    fn should_leave_the_score_empty_when_either_side_has_no_abstract() {
        let none = result(None, AbstractStatus::NotFound);
        let agreement = agreement(Some("An abstract."), &none);
        assert_eq!(agreement.token_f1, None);
        assert!(!agreement.disagrees());
        assert!(agreement.model_found && !agreement.deterministic_found);
    }
}
