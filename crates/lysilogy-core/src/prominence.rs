//! A deterministic prominence score for a Work inside one workspace.
//!
//! The score is a weighted mean of the components that are known, so a missing
//! count or year neither helps nor punishes a Work.

use serde::{Deserialize, Serialize};

/// Bumped when the signal definitions or weights change; stale cache rows are recomputed.
pub const PROMINENCE_SIGNALS_VERSION: i16 = 1;

const WEIGHT_DEGREE: f64 = 0.30;
const WEIGHT_GLOBAL: f64 = 0.25;
const WEIGHT_LOCAL: f64 = 0.15;
const WEIGHT_MENTIONS: f64 = 0.10;
const WEIGHT_ROLE: f64 = 0.10;
const WEIGHT_RECENCY: f64 = 0.10;

const DEGREE_SATURATION: f64 = 8.0;
const MENTION_SATURATION: f64 = 10.0;
const GLOBAL_DECADES: f64 = 5.0;
const RECENCY_HALF_SCALE_YEARS: f64 = 15.0;
const BACKGROUND_ROLE_VALUE: f64 = 0.3;

/// The raw facts a score is computed from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProminenceSignals {
    pub citing_works_count: i64,
    pub cited_works_count: i64,
    pub cited_by_global_count: Option<i64>,
    pub display_year: Option<i32>,
    pub local_available: bool,
    pub mention_count: i64,
    pub method_citation_count: i64,
    pub background_citation_count: i64,
}

/// Each component in `0..=1`; `None` when the underlying signal is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ProminenceComponents {
    pub degree: f64,
    pub global: Option<f64>,
    pub local: f64,
    pub mentions: f64,
    pub role: Option<f64>,
    pub recency: Option<f64>,
}

#[allow(clippy::cast_precision_loss)]
fn saturating_log(count: i64, saturation: f64) -> f64 {
    let count = count.max(0) as f64;
    (count.ln_1p() / saturation.ln_1p()).min(1.0)
}

/// Break the signals into their `0..=1` components.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn prominence_components(
    signals: &ProminenceSignals,
    current_year: i32,
) -> ProminenceComponents {
    let global = signals.cited_by_global_count.map(|count| {
        let count = count.max(0) as f64;
        (count.ln_1p() / std::f64::consts::LN_10 / GLOBAL_DECADES).min(1.0)
    });
    let role = if signals.method_citation_count > 0 {
        Some(1.0)
    } else if signals.background_citation_count > 0 {
        Some(BACKGROUND_ROLE_VALUE)
    } else {
        None
    };
    let recency = signals.display_year.map(|year| {
        let age = i64::from(current_year) - i64::from(year);
        let age = age.max(0) as f64;
        (-age / RECENCY_HALF_SCALE_YEARS).exp()
    });
    ProminenceComponents {
        degree: saturating_log(signals.citing_works_count, DEGREE_SATURATION),
        global,
        local: if signals.local_available { 1.0 } else { 0.0 },
        mentions: saturating_log(signals.mention_count, MENTION_SATURATION),
        role,
        recency,
    }
}

/// The weighted mean of known components, in `0..=1`, rounded to 4 decimals.
#[must_use]
pub fn prominence_score(signals: &ProminenceSignals, current_year: i32) -> f64 {
    let components = prominence_components(signals, current_year);
    let mut weighted = 0.0;
    let mut weight = 0.0;
    for (value, component_weight) in [
        (Some(components.degree), WEIGHT_DEGREE),
        (components.global, WEIGHT_GLOBAL),
        (Some(components.local), WEIGHT_LOCAL),
        (Some(components.mentions), WEIGHT_MENTIONS),
        (components.role, WEIGHT_ROLE),
        (components.recency, WEIGHT_RECENCY),
    ] {
        let Some(value) = value else {
            continue;
        };
        weighted = component_weight.mul_add(value, weighted);
        weight += component_weight;
    }
    let score = (weighted / weight).clamp(0.0, 1.0);
    (score * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const YEAR: i32 = 2026;

    fn score(signals: &ProminenceSignals) -> f64 {
        prominence_score(signals, YEAR)
    }

    #[test]
    fn should_score_zero_when_all_signals_are_empty() {
        assert!(score(&ProminenceSignals::default()).abs() < f64::EPSILON);
    }

    #[test]
    fn should_ignore_unknown_components_when_averaging() {
        let signals = ProminenceSignals {
            citing_works_count: 8,
            local_available: true,
            mention_count: 10,
            ..ProminenceSignals::default()
        };
        // degree, local, and mentions are all 1.0 and the unknown ones are skipped.
        assert!((score(&signals) - 1.0).abs() < 1e-9);

        let half = ProminenceSignals {
            citing_works_count: 8,
            ..ProminenceSignals::default()
        };
        let expected = (0.30_f64 / 0.55 * 10_000.0).round() / 10_000.0;
        assert!((score(&half) - expected).abs() < 1e-9);
    }

    #[test]
    fn should_increase_score_when_any_signal_increases() {
        let base = ProminenceSignals {
            citing_works_count: 1,
            cited_by_global_count: Some(10),
            display_year: Some(2000),
            local_available: false,
            mention_count: 1,
            method_citation_count: 0,
            background_citation_count: 1,
            ..ProminenceSignals::default()
        };
        let before = score(&base);
        let better = [
            ProminenceSignals {
                citing_works_count: 4,
                ..base
            },
            ProminenceSignals {
                cited_by_global_count: Some(10_000),
                ..base
            },
            ProminenceSignals {
                display_year: Some(2020),
                ..base
            },
            ProminenceSignals {
                local_available: true,
                ..base
            },
            ProminenceSignals {
                mention_count: 6,
                ..base
            },
            ProminenceSignals {
                method_citation_count: 1,
                ..base
            },
        ];
        for (index, signals) in better.iter().enumerate() {
            assert!(score(signals) > before, "signal {index} did not raise the score");
        }
    }

    #[test]
    fn should_cap_components_at_one_when_counts_are_huge() {
        let signals = ProminenceSignals {
            citing_works_count: i64::MAX,
            cited_by_global_count: Some(i64::MAX),
            mention_count: i64::MAX,
            method_citation_count: i64::MAX,
            local_available: true,
            display_year: Some(YEAR),
            ..ProminenceSignals::default()
        };
        let components = prominence_components(&signals, YEAR);
        assert!((components.degree - 1.0).abs() < f64::EPSILON);
        assert!((components.global.expect("global") - 1.0).abs() < f64::EPSILON);
        assert!((components.mentions - 1.0).abs() < f64::EPSILON);
        assert!((score(&signals) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn should_clamp_negative_counts_and_future_years_when_scoring() {
        let negative = ProminenceSignals {
            citing_works_count: -5,
            cited_by_global_count: Some(-1),
            mention_count: i64::MIN,
            ..ProminenceSignals::default()
        };
        let components = prominence_components(&negative, YEAR);
        assert!(components.degree.abs() < f64::EPSILON);
        assert!(components.global.expect("known").abs() < f64::EPSILON);
        assert!(components.mentions.abs() < f64::EPSILON);

        let future = ProminenceSignals {
            display_year: Some(YEAR + 500),
            ..ProminenceSignals::default()
        };
        let recency = prominence_components(&future, YEAR).recency.expect("year");
        assert!((recency - 1.0).abs() < f64::EPSILON);
        let extreme = ProminenceSignals {
            display_year: Some(i32::MIN),
            ..ProminenceSignals::default()
        };
        let old = prominence_components(&extreme, YEAR).recency.expect("year");
        assert!((0.0..=1.0).contains(&old));
    }

    #[test]
    fn should_prefer_method_over_background_when_role_known() {
        let method = ProminenceSignals {
            method_citation_count: 1,
            background_citation_count: 5,
            ..ProminenceSignals::default()
        };
        let background = ProminenceSignals {
            background_citation_count: 5,
            ..ProminenceSignals::default()
        };
        assert_eq!(prominence_components(&method, YEAR).role, Some(1.0));
        assert_eq!(prominence_components(&background, YEAR).role, Some(0.3));
        assert_eq!(
            prominence_components(&ProminenceSignals::default(), YEAR).role,
            None
        );
        assert!(score(&method) > score(&background));
    }

    #[test]
    fn should_stay_within_unit_interval_when_signals_vary() {
        let counts = [i64::MIN, -1, 0, 1, 7, 8, 1_000, i64::MAX];
        let globals = [None, Some(-3), Some(0), Some(99), Some(i64::MAX)];
        let years = [None, Some(i32::MIN), Some(1500), Some(2026), Some(i32::MAX)];
        for &citing in &counts {
            for &global in &globals {
                for &year in &years {
                    for &local in &[false, true] {
                        for &mentions in &counts {
                            let signals = ProminenceSignals {
                                citing_works_count: citing,
                                cited_by_global_count: global,
                                display_year: year,
                                local_available: local,
                                mention_count: mentions,
                                method_citation_count: citing.max(0) % 2,
                                background_citation_count: mentions.max(0) % 2,
                                ..ProminenceSignals::default()
                            };
                            let value = score(&signals);
                            assert!(value.is_finite());
                            assert!((0.0..=1.0).contains(&value), "{signals:?} -> {value}");
                        }
                    }
                }
            }
        }
    }
}
