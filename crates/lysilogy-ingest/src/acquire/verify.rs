//! Sanity check that a fetched PDF is the paper the Work claims to be.

use std::collections::HashSet;

use lysilogy_core::title_key;

/// The fewest title key words that make the check meaningful.
const MIN_TITLE_KEY_WORDS: usize = 3;

/// Whether at least 80 % of the title's key words occur in `text`.
///
/// `None` when the title has fewer than three key words (too generic to judge).
#[must_use]
pub fn title_appears_in(title: &str, text: &str) -> Option<bool> {
    let key = title_key(title);
    let words: Vec<&str> = key.split_whitespace().collect();
    if words.len() < MIN_TITLE_KEY_WORDS {
        return None;
    }
    let text_key = title_key(text);
    let present: HashSet<&str> = text_key.split_whitespace().collect();
    let hits = words.iter().filter(|word| present.contains(*word)).count();
    Some(hits * 5 >= words.len() * 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_report_title_present_when_most_key_words_appear() {
        let front = "Cooperative Inverse Reinforcement Learning\nDylan Hadfield-Menell, \
                     Anca Dragan, Pieter Abbeel, Stuart Russell";
        assert_eq!(
            title_appears_in("Cooperative Inverse Reinforcement Learning", front),
            Some(true)
        );
        assert_eq!(
            title_appears_in(
                "Cooperative inverse reinforcement learning",
                "COOPERATIVE INVERSE REINFORCEMENT LEARNING"
            ),
            Some(true),
            "case does not matter"
        );
        assert_eq!(
            title_appears_in(
                "A very long title about reward modelling and alignment",
                "reward modelling alignment title long very about and a"
            ),
            Some(true)
        );
    }

    #[test]
    fn should_report_title_absent_when_few_key_words_appear() {
        assert_eq!(
            title_appears_in(
                "Cooperative Inverse Reinforcement Learning",
                "Annual report of the committee on fisheries"
            ),
            Some(false)
        );
        assert_eq!(
            title_appears_in("Cooperative Inverse Reinforcement Learning", ""),
            Some(false)
        );
        assert_eq!(
            title_appears_in("One two three four five", "one two three four"),
            Some(true),
            "exactly 80% counts"
        );
        assert_eq!(
            title_appears_in("One two three four five", "one two three"),
            Some(false)
        );
    }

    #[test]
    fn should_skip_title_check_when_title_has_fewer_than_three_words() {
        assert_eq!(
            title_appears_in("Attention", "Attention is all you need"),
            None
        );
        assert_eq!(title_appears_in("Deep learning", "Deep learning"), None);
        assert_eq!(title_appears_in("", "anything"), None);
        assert_eq!(title_appears_in("   ", "anything"), None);
    }
}
