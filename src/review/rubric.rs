//! The bundled review rubric and validation of submitted grades against it.

use std::collections::BTreeSet;

use serde::Deserialize;

use super::{GradeValue, ReviewInput};
use crate::{Error, Result};

/// The rubric the review panel and the summary use, compiled into the binary
/// so a server and its stored reviews always agree on dimension ids.
pub const RUBRIC_JSON: &str = include_str!("../../eval/review/rubric.json");

/// Issue flags that are not about one component use this component id.
pub const GENERAL_COMPONENT: &str = "general";

const MAX_TEXT_CHARS: usize = 8_000;
const MAX_ISSUES: usize = 200;
const MAX_COUNT: u32 = 1_000;
const MAX_ID_CHARS: usize = 64;
const MAX_ITEMS: usize = 50;

#[derive(Clone, Debug, Deserialize)]
pub struct Rubric {
    pub id: String,
    pub scale: Scale,
    pub components: Vec<RubricComponent>,
    pub overall: RubricDimension,
    pub issues: IssueSpec,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Scale {
    pub min: u32,
    pub max: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RubricComponent {
    pub id: String,
    pub label: String,
    /// The analysis fields this component grades; the export copies them
    /// next to each flagged issue.
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub dimensions: Vec<RubricDimension>,
    #[serde(default)]
    pub hard_reject: Option<HardReject>,
    /// One verdict per sampled item (for example per citation label),
    /// stored as a choice under `{prefix}{item id}` in the grade's values.
    #[serde(default)]
    pub item_verdict: Option<ItemVerdict>,
}

/// A per-item verdict: the reviewer picks one option for each sampled item,
/// and the summary derives the counts.
#[derive(Clone, Debug, Deserialize)]
pub struct ItemVerdict {
    pub prefix: String,
    pub label: String,
    pub options: Vec<RubricOption>,
}

impl RubricComponent {
    /// The item id of a per-item verdict key, when `key` is one.
    #[must_use]
    pub fn item_id<'k>(&self, key: &'k str) -> Option<&'k str> {
        let verdict = self.item_verdict.as_ref()?;
        key.strip_prefix(verdict.prefix.as_str())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct HardReject {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RubricDimension {
    pub id: String,
    pub label: String,
    pub kind: DimensionKind,
    #[serde(default)]
    pub options: Vec<RubricOption>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionKind {
    /// 0-4 on the rubric scale.
    Score,
    PassFail,
    /// One of the dimension's options.
    Choice,
    /// A non-negative tally, such as missing headings.
    Count,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RubricOption {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IssueSpec {
    pub severities: Vec<RubricOption>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Rubric {
    /// Parses and checks the bundled rubric.
    pub fn bundled() -> Result<Self> {
        Self::parse(RUBRIC_JSON)
    }

    pub fn parse(json: &str) -> Result<Self> {
        let rubric: Self = serde_json::from_str(json)?;
        rubric.check()?;
        Ok(rubric)
    }

    fn check(&self) -> Result<()> {
        let invalid = |message: String| Err(Error::Task(format!("review rubric: {message}")));
        if self.scale.min >= self.scale.max {
            return invalid("scale minimum must be below its maximum".to_owned());
        }
        if self.overall.kind != DimensionKind::Score {
            return invalid("overall must be a score".to_owned());
        }
        let mut components = BTreeSet::new();
        for component in &self.components {
            if component.id == GENERAL_COMPONENT || !components.insert(component.id.as_str()) {
                return invalid(format!(
                    "component id {} is reserved or repeated",
                    component.id
                ));
            }
            let mut dimensions = BTreeSet::new();
            for dimension in &component.dimensions {
                if !dimensions.insert(dimension.id.as_str()) {
                    return invalid(format!("{}.{} is repeated", component.id, dimension.id));
                }
                if dimension.kind == DimensionKind::Choice && dimension.options.is_empty() {
                    return invalid(format!("{}.{} has no options", component.id, dimension.id));
                }
            }
            if let Some(verdict) = &component.item_verdict
                && (verdict.prefix.is_empty()
                    || verdict.options.is_empty()
                    || component
                        .dimensions
                        .iter()
                        .any(|dimension| dimension.id.starts_with(&verdict.prefix)))
            {
                return invalid(format!(
                    "{} item verdict needs a prefix no dimension uses, and options",
                    component.id
                ));
            }
        }
        if self.issues.severities.is_empty() {
            return invalid("issues need at least one severity".to_owned());
        }
        Ok(())
    }

    #[must_use]
    pub fn component(&self, id: &str) -> Option<&RubricComponent> {
        self.components.iter().find(|component| component.id == id)
    }

    /// Refuses grades that do not fit the rubric, so stored reviews can be
    /// aggregated without guessing what a value meant.
    pub fn validate(&self, input: &ReviewInput) -> Result<()> {
        let invalid = |message: String| Err(Error::InvalidRequest(message));
        for (component_id, grade) in &input.components {
            let Some(component) = self.component(component_id) else {
                return invalid(format!("unknown review component {component_id}"));
            };
            if grade.hard_reject && component.hard_reject.is_none() {
                return invalid(format!("{component_id} has no hard reject"));
            }
            check_text(&grade.notes, "component notes")?;
            let mut items = 0;
            for (dimension_id, value) in &grade.values {
                if let Some(item) = component.item_id(dimension_id) {
                    items += 1;
                    let accepted = component.item_verdict.as_ref().is_some_and(|verdict| {
                        matches!(value, GradeValue::Choice(choice)
                            if verdict.options.iter().any(|option| option.id == *choice))
                    });
                    if item.trim().is_empty() || item.chars().count() > MAX_ID_CHARS || !accepted {
                        return invalid(format!(
                            "{component_id} verdict {dimension_id} does not accept {value:?}"
                        ));
                    }
                    if items > MAX_ITEMS {
                        return invalid(format!("{component_id} takes at most {MAX_ITEMS} verdicts"));
                    }
                    continue;
                }
                let Some(dimension) = component
                    .dimensions
                    .iter()
                    .find(|dimension| dimension.id == *dimension_id)
                else {
                    return invalid(format!("unknown dimension {component_id}.{dimension_id}"));
                };
                if !self.accepts(dimension, value) {
                    return invalid(format!(
                        "{component_id}.{dimension_id} does not accept {value:?}"
                    ));
                }
            }
        }
        if let Some(overall) = input.overall
            && !self.in_scale(overall)
        {
            return invalid(format!("overall score {overall} is outside the scale"));
        }
        check_text(&input.notes, "review notes")?;
        if input.issues.len() > MAX_ISSUES {
            return invalid(format!("a review holds at most {MAX_ISSUES} issues"));
        }
        let mut issue_ids = BTreeSet::new();
        for issue in &input.issues {
            if issue.id.trim().is_empty()
                || issue.id.chars().count() > MAX_ID_CHARS
                || !issue_ids.insert(issue.id.as_str())
            {
                return invalid(format!(
                    "issue id {:?} is empty, too long or repeated",
                    issue.id
                ));
            }
            if issue.component != GENERAL_COMPONENT && self.component(&issue.component).is_none() {
                return invalid(format!("issue names unknown component {}", issue.component));
            }
            if !self
                .issues
                .severities
                .iter()
                .any(|severity| severity.id == issue.severity)
            {
                return invalid(format!("unknown issue severity {}", issue.severity));
            }
            if let Some(tag) = &issue.tag
                && !self.issues.tags.contains(tag)
            {
                return invalid(format!("unknown issue tag {tag}"));
            }
            let quote = issue.quote.as_deref().unwrap_or_default();
            if issue.note.trim().is_empty() && quote.trim().is_empty() {
                return invalid("an issue needs a note or a quote".to_owned());
            }
            check_text(&issue.note, "issue note")?;
            check_text(quote, "issue quote")?;
        }
        Ok(())
    }

    const fn in_scale(&self, value: u32) -> bool {
        value >= self.scale.min && value <= self.scale.max
    }

    fn accepts(&self, dimension: &RubricDimension, value: &GradeValue) -> bool {
        match (dimension.kind, value) {
            (DimensionKind::Score, GradeValue::Number(score)) => self.in_scale(*score),
            (DimensionKind::Count, GradeValue::Number(count)) => *count <= MAX_COUNT,
            (DimensionKind::PassFail, GradeValue::Flag(_)) => true,
            (DimensionKind::Choice, GradeValue::Choice(choice)) => {
                dimension.options.iter().any(|option| option.id == *choice)
            }
            _ => false,
        }
    }
}

fn check_text(text: &str, what: &str) -> Result<()> {
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(Error::InvalidRequest(format!(
            "{what} is limited to {MAX_TEXT_CHARS} characters"
        )));
    }
    Ok(())
}
