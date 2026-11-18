//! Whether each verification stage of a run actually ran.
//!
//! A budget stop or a failed call used to leave the self-check out of a
//! record without a trace, so an unchecked draft looked like a checked one.
//! The record now says, stage by stage, what happened.

use serde::Serialize;

use crate::domain::{QualityWarning, QualityWarningKind};

/// What happened to one verification stage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    /// The stage ran and its answer was used.
    Ran,
    /// The run was configured without this stage.
    #[default]
    Disabled,
    /// There was nothing to verify: no draft, or no failing quote to repair.
    NotNeeded,
    /// The paper budget was spent before the stage could start.
    SkippedBudget,
    /// The stage ran but returned no usable answer.
    Failed,
}

impl StageStatus {
    /// Whether the stage was meant to run and did not.
    #[must_use]
    pub const fn incomplete(self) -> bool {
        matches!(self, Self::SkippedBudget | Self::Failed)
    }
}

/// The verification stages of one run.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Verification {
    /// Pass 2: the model checks and corrects its own draft.
    pub self_check: StageStatus,
    /// Quote admission against the page text. Always local, so it runs
    /// whenever there is a draft.
    pub admission: StageStatus,
    /// The model repair of quotes admission could not place.
    pub repair: StageStatus,
}

impl Verification {
    /// Whether every stage that was meant to run did.
    #[must_use]
    pub const fn complete(&self) -> bool {
        !self.self_check.incomplete() && !self.admission.incomplete() && !self.repair.incomplete()
    }

    /// One warning per stage that was meant to run and did not.
    #[must_use]
    pub fn warnings(&self) -> Vec<QualityWarning> {
        [
            (
                self.self_check,
                "self_check",
                "The self-check pass did not run",
                "Nothing has checked the draft against the paper.",
            ),
            (
                self.admission,
                "admission",
                "Quote admission did not run",
                "Quotes have not been checked against the page text.",
            ),
            (
                self.repair,
                "repair",
                "The quote repair call did not run",
                "Quotes that were not verbatim were removed instead of repaired.",
            ),
        ]
        .into_iter()
        .filter_map(|(status, field, what, consequence)| {
            let (kind, why) = match status {
                StageStatus::SkippedBudget => (
                    QualityWarningKind::VerificationSkipped,
                    "because the paper budget was spent",
                ),
                StageStatus::Failed => (
                    QualityWarningKind::VerificationFailed,
                    "because the call returned no usable answer",
                ),
                _ => return None,
            };
            Some(QualityWarning {
                kind,
                field: field.to_owned(),
                message: format!("{what} {why}. {consequence}"),
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_warn_for_each_stage_that_was_meant_to_run_and_did_not() {
        let verification = Verification {
            self_check: StageStatus::SkippedBudget,
            admission: StageStatus::Ran,
            repair: StageStatus::Failed,
        };
        assert!(!verification.complete());
        let warnings = verification.warnings();
        assert_eq!(
            warnings
                .iter()
                .map(|warning| (warning.kind, warning.field.as_str()))
                .collect::<Vec<_>>(),
            [
                (QualityWarningKind::VerificationSkipped, "self_check"),
                (QualityWarningKind::VerificationFailed, "repair"),
            ]
        );
        assert!(warnings[0].message.contains("paper budget"));
    }

    #[test]
    fn should_count_disabled_and_unneeded_stages_as_complete() {
        let verification = Verification {
            self_check: StageStatus::Disabled,
            admission: StageStatus::Ran,
            repair: StageStatus::NotNeeded,
        };
        assert!(verification.complete());
        assert!(verification.warnings().is_empty());
    }
}
