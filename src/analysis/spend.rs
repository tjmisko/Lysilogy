//! Per-substage model spend ledger.
//!
//! Every `OpenRouter` completion appends one JSON line naming the stage, the
//! model, the token counts and the cost, and the running total is capped to a
//! small budget so a runaway pass stops instead of spending. The ledger is a
//! plain file so it can be inspected or cleared without a migration.

use std::{
    collections::BTreeMap,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The default cap, in US dollars.
pub const DEFAULT_BUDGET_USD: f64 = 0.10;

/// The conventional ledger, relative to the working directory.
pub const DEFAULT_LEDGER_PATH: &str = ".lysilogy/model-spend.jsonl";

/// `deepseek/deepseek-v4.1-flash` input price in USD per million tokens.
pub const DEEPSEEK_INPUT_PER_MILLION: f64 = 0.036;

/// `deepseek/deepseek-v4.1-flash` output price in USD per million tokens.
pub const DEEPSEEK_OUTPUT_PER_MILLION: f64 = 0.6435;

/// One completed model call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpendRecord {
    pub at: DateTime<Utc>,
    pub stage: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
}

/// A per-stage total.
#[derive(Clone, Debug, Default, Serialize)]
pub struct StageTotal {
    pub stage: String,
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    budget_usd: f64,
    spent_usd: f64,
}

/// A shared, file-backed spend ledger.
#[derive(Clone, Debug)]
pub struct SpendLedger {
    inner: Arc<Mutex<Inner>>,
}

impl Default for SpendLedger {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl SpendLedger {
    /// A ledger at the conventional path with the configured budget.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(default_ledger_path(), budget_usd())
    }

    /// A ledger at `path` that refuses calls once `budget_usd` is spent.
    #[must_use]
    pub fn new(path: PathBuf, budget_usd: f64) -> Self {
        let spent_usd = read_total(&path);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                path,
                budget_usd,
                spent_usd,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The cap, in US dollars.
    #[must_use]
    pub fn budget_usd(&self) -> f64 {
        self.lock().budget_usd
    }

    /// The total spent so far, in US dollars.
    #[must_use]
    pub fn total_usd(&self) -> f64 {
        self.lock().spent_usd
    }

    /// Error if the budget is already spent.
    pub fn ensure_available(&self) -> Result<()> {
        let (budget_usd, spent_usd) = {
            let inner = self.lock();
            (inner.budget_usd, inner.spent_usd)
        };
        if spent_usd >= budget_usd {
            return Err(Error::BudgetExceeded {
                budget_usd,
                spent_usd,
            });
        }
        Ok(())
    }

    /// Append a record and add its cost to the running total.
    ///
    /// # Errors
    /// Fails if the ledger directory cannot be created or the line cannot be written.
    pub fn record(&self, record: &SpendRecord) -> Result<()> {
        let path = self.lock().path.clone();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
        }
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| Error::io(&path, error))?;
        file.write_all(line.as_bytes())
            .map_err(|error| Error::io(&path, error))?;
        self.lock().spent_usd += record.cost_usd;
        Ok(())
    }
}

/// The ledger path from `LYSILOGY_SPEND_LEDGER`, or the convention.
#[must_use]
pub fn default_ledger_path() -> PathBuf {
    std::env::var_os("LYSILOGY_SPEND_LEDGER")
        .map_or_else(|| PathBuf::from(DEFAULT_LEDGER_PATH), PathBuf::from)
}

/// The budget from `LYSILOGY_MODEL_BUDGET_USD`, or the default.
#[must_use]
pub fn budget_usd() -> f64 {
    std::env::var("LYSILOGY_MODEL_BUDGET_USD")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value > 0.0)
        .unwrap_or(DEFAULT_BUDGET_USD)
}

/// Cost in USD for a completion, falling back to the published prices.
#[must_use]
pub fn completion_cost(prompt_tokens: u64, completion_tokens: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let prompt = prompt_tokens as f64;
    #[allow(clippy::cast_precision_loss)]
    let completion = completion_tokens as f64;
    let input = prompt / 1_000_000.0;
    let output = completion / 1_000_000.0;
    input.mul_add(
        DEEPSEEK_INPUT_PER_MILLION,
        output * DEEPSEEK_OUTPUT_PER_MILLION,
    )
}

/// Read every record in the ledger at `path`; a missing file is empty.
#[must_use]
pub fn read_records(path: &Path) -> Vec<SpendRecord> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<SpendRecord>(line).ok())
        .collect()
}

/// Totals by stage, sorted by stage name.
#[must_use]
pub fn stage_totals(records: &[SpendRecord]) -> Vec<StageTotal> {
    let mut totals: BTreeMap<String, StageTotal> = BTreeMap::new();
    for record in records {
        let total = totals
            .entry(record.stage.clone())
            .or_insert_with(|| StageTotal {
                stage: record.stage.clone(),
                ..StageTotal::default()
            });
        total.calls += 1;
        total.prompt_tokens += record.prompt_tokens;
        total.completion_tokens += record.completion_tokens;
        total.cost_usd += record.cost_usd;
    }
    totals.into_values().collect()
}

fn read_total(path: &Path) -> f64 {
    read_records(path)
        .iter()
        .map(|record| record.cost_usd)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(stage: &str, cost_usd: f64) -> SpendRecord {
        SpendRecord {
            at: Utc::now(),
            stage: stage.to_owned(),
            model: "deepseek/deepseek-v4.1-flash".to_owned(),
            prompt_tokens: 100,
            completion_tokens: 20,
            cost_usd,
        }
    }

    #[test]
    fn should_price_a_completion_from_the_published_rates() {
        let cost = completion_cost(1_000_000, 1_000_000);
        let expected = DEEPSEEK_INPUT_PER_MILLION + DEEPSEEK_OUTPUT_PER_MILLION;
        assert!((cost - expected).abs() < 1e-9, "{cost} != {expected}");
    }

    #[test]
    fn should_record_append_and_total_by_stage() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("spend.jsonl");
        let ledger = SpendLedger::new(path.clone(), 1.0);
        ledger
            .record(&record("orientation", 0.001))
            .expect("record");
        ledger
            .record(&record("orientation", 0.002))
            .expect("record");
        ledger.record(&record("labels", 0.003)).expect("record");
        assert!((ledger.total_usd() - 0.006).abs() < 1e-9);
        let records = read_records(&path);
        assert_eq!(records.len(), 3);
        let totals = stage_totals(&records);
        let orientation = totals
            .iter()
            .find(|total| total.stage == "orientation")
            .expect("orientation total");
        assert_eq!(orientation.calls, 2);
        assert_eq!(orientation.prompt_tokens, 200);
    }

    #[test]
    fn should_refuse_when_the_budget_is_spent() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let ledger = SpendLedger::new(dir.path().join("spend.jsonl"), 0.01);
        ledger.record(&record("structure", 0.02)).expect("record");
        assert!(matches!(
            ledger.ensure_available(),
            Err(Error::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn should_reload_spend_from_an_existing_ledger() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("spend.jsonl");
        SpendLedger::new(path.clone(), 1.0)
            .record(&record("fields", 0.05))
            .expect("record");
        let reloaded = SpendLedger::new(path, 1.0);
        assert!((reloaded.total_usd() - 0.05).abs() < 1e-9);
    }
}
