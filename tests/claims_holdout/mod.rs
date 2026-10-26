//! The held-out claim verifier evaluation: replays the recorded provider responses of E1,
//! E2 and E3 through the frozen verifier (optionally with one check ablated) and computes
//! the metrics in `eval/claims/results.json`.
//!
//! Replay only: no network, and an empty graph and a cold response cache for every claim
//! (the scratch database is truncated between claims), so the local-match shortcut cannot
//! help and calls per claim are cold-cache counts. Included by
//! `examples/claims_holdout_eval.rs` and `tests/claims_holdout_eval.rs`, which declare
//! `mod claims_support;` beside it.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lysilogy_core::{Ablation, Claim};
use lysilogy_db::claim::ClaimRepository;
use lysilogy_db::source_record::SourceRecordRepository;
use lysilogy_db::work::WorkRepository;
use lysilogy_ingest::openalex::{JsonTransport, OpenAlexClient, ProviderError, ProviderFuture};
use lysilogy_ingest::verify_claim::fixtures::{ReplayTransport, Responses};
use lysilogy_ingest::verify_claim::{ClaimResolution, VerifyContext, verify_claim_with};
use serde_json::{Value, json};

use crate::claims_support::{Failure, Outcome, ScratchDatabase};

/// z for a 95% interval.
const Z95: f64 = 1.96;

/// A count out of a total, as the numbers in `results.json` report it.
#[allow(clippy::cast_precision_loss)]
fn fraction(part: usize, whole: usize) -> f64 {
    part as f64 / whole as f64
}

/// The 95% Wilson score interval, `None` for an empty total.
#[must_use]
pub fn wilson(k: usize, n: usize) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let total = fraction(n, 1);
    let p = fraction(k, n);
    let z2 = Z95 * Z95;
    let denominator = 1.0 + z2 / total;
    let centre = (p + z2 / (2.0 * total)) / denominator;
    let half = Z95 * (p * (1.0 - p) / total + z2 / (4.0 * total * total)).sqrt() / denominator;
    Some(((centre - half).max(0.0), (centre + half).min(1.0)))
}

/// The rule-of-three 95% upper bound on a rate seen zero times in `n` trials.
#[must_use]
pub fn rule_of_three(n: usize) -> Option<f64> {
    (n > 0).then(|| (3.0 / fraction(n, 1)).min(1.0))
}

fn round(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

/// `{k, n, rate, wilson95, rule_of_three_upper}`; the interval and rate are null for n = 0
/// and the rule-of-three bound appears only when k = 0.
#[must_use]
pub fn rate_json(k: usize, n: usize) -> Value {
    let interval = wilson(k, n).map(|(low, high)| json!([round(low), round(high)]));
    json!({
        "k": k,
        "n": n,
        "rate": (n > 0).then(|| round(fraction(k, n))),
        "wilson95": interval,
        "rule_of_three_upper": if k == 0 { rule_of_three(n).map(round) } else { None },
    })
}

/// What a claim is supposed to come to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A real paper: should resolve to its Work.
    Real,
    /// A real paper with a typo, a dropped subtitle or abbreviated authors.
    NearMiss,
    /// A fake, a `does_not_exist` or a must-not-accept case: must not become a Work.
    MustNot,
    /// A real paper with wrong details (`exists_wrong_details`). Reported apart.
    Flawed,
    /// Left out of the headline numbers (labellers disagreed, or no label).
    Excluded,
}

/// One claim to run, with how to score it.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    /// `e1`, `e2` or `e3`.
    pub set: &'static str,
    /// The breakdown: E1 and E3 the case kind, E2 the model.
    pub group: String,
    pub class: Class,
    pub claim: Claim,
    /// `OpenAlex` IDs that count as the right Work; empty when unknown.
    pub truths: Vec<String>,
    pub detail: Value,
}

/// How one item came out.
#[derive(Clone, Debug)]
pub struct Row {
    pub outcome: Outcome,
    pub candidates: Vec<String>,
    pub calls: usize,
    pub searches: usize,
    /// The verifier asked for a response the recording does not hold (it was recorded with
    /// other Works already in the graph), so this claim cannot be replayed honestly.
    pub unreplayable: bool,
}

/// A transport that counts the requests that reach it.
#[derive(Clone)]
struct Counting {
    inner: ReplayTransport,
    calls: Arc<AtomicUsize>,
    searches: Arc<AtomicUsize>,
    unrecorded: Arc<AtomicUsize>,
}

impl JsonTransport for Counting {
    fn get_json<'a>(&'a self, url: &'a str) -> ProviderFuture<'a, Result<Value, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if url.contains("title.search") {
            self.searches.fetch_add(1, Ordering::SeqCst);
        }
        Box::pin(async move {
            let result = self.inner.get_json(url).await;
            // The replay answers an unrecorded URL with a request error; nothing else does.
            if matches!(result, Err(ProviderError::Request(_))) {
                self.unrecorded.fetch_add(1, Ordering::SeqCst);
            }
            result
        })
    }
}

/// The rows of one run, in item order, and the URLs the verifier asked for that were not
/// recorded (those answered as transport errors).
pub struct Run {
    pub rows: Vec<Row>,
    pub misses: Vec<String>,
}

/// Run every item through the verifier with this ablation, replaying `responses`.
pub async fn run_items(
    database_url: &str,
    responses: &Responses,
    items: &[Item],
    ablation: Ablation,
) -> Result<Run, Failure> {
    let replay = ReplayTransport::new(responses.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let searches = Arc::new(AtomicUsize::new(0));
    let unrecorded = Arc::new(AtomicUsize::new(0));
    let client = OpenAlexClient::new(Counting {
        inner: replay.clone(),
        calls: Arc::clone(&calls),
        searches: Arc::clone(&searches),
        unrecorded: Arc::clone(&unrecorded),
    });
    let scratch = ScratchDatabase::create(database_url).await?;
    let works = WorkRepository::new(scratch.pool.clone());
    let claims = ClaimRepository::new(scratch.pool.clone());
    let records = SourceRecordRepository::new(scratch.pool.clone());
    let context = VerifyContext {
        works: &works,
        claims: &claims,
        records: &records,
        client: &client,
    };
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        // An empty graph and a cold response cache for every claim, so no earlier claim's
        // Work or cached search can decide this one.
        sqlx::query("TRUNCATE graph.work, source.source_record CASCADE")
            .execute(&scratch.pool)
            .await?;
        let (calls_before, searches_before, unrecorded_before) = (
            calls.load(Ordering::SeqCst),
            searches.load(Ordering::SeqCst),
            unrecorded.load(Ordering::SeqCst),
        );
        let resolution = verify_claim_with(&context, &item.claim, ablation).await?;
        let (outcome, candidates) = match resolution {
            ClaimResolution::Work { work, verified, .. } => {
                let ids = works.openalex_ids_for(&[work.id]).await?;
                (
                    Outcome::Work {
                        openalex_id: ids.into_iter().next().map(|(_, id)| id),
                        verified,
                    },
                    Vec::new(),
                )
            }
            ClaimResolution::Unresolved {
                state, candidates, ..
            } => (
                Outcome::Unresolved(state),
                candidates.into_iter().map(|c| c.provider_id).collect(),
            ),
        };
        rows.push(Row {
            outcome,
            candidates,
            calls: calls.load(Ordering::SeqCst) - calls_before,
            searches: searches.load(Ordering::SeqCst) - searches_before,
            unreplayable: unrecorded.load(Ordering::SeqCst) > unrecorded_before,
        });
    }
    scratch.drop_database().await?;
    Ok(Run {
        rows,
        misses: replay.misses(),
    })
}

/// `verified`, `matched`, or the unresolved state.
#[must_use]
pub fn outcome_label(outcome: &Outcome) -> String {
    outcome
        .describe()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Judgement {
    Correct,
    WrongWork,
    Unadjudicated,
    NotWork,
}

fn judge(item: &Item, outcome: &Outcome) -> Judgement {
    let Outcome::Work { openalex_id, .. } = outcome else {
        return Judgement::NotWork;
    };
    if item.truths.is_empty() {
        return Judgement::Unadjudicated;
    }
    let found = openalex_id.as_deref().is_some_and(|id| {
        item.truths
            .iter()
            .any(|truth| truth.eq_ignore_ascii_case(id))
    });
    if found {
        Judgement::Correct
    } else {
        Judgement::WrongWork
    }
}

/// Counts over a group of items; the rates in `results.json` are derived from these.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub n: usize,
    pub outcomes: BTreeMap<String, usize>,
    pub must_not_n: usize,
    pub must_not_accepted: usize,
    pub real_n: usize,
    pub real_correct: usize,
    pub real_wrong: usize,
    pub real_unadjudicated: usize,
    pub near_n: usize,
    pub near_correct: usize,
    pub near_wrong: usize,
    pub near_unadjudicated: usize,
    pub flawed_n: usize,
    pub flawed_correct: usize,
    pub flawed_wrong: usize,
    pub flawed_unadjudicated: usize,
    /// Ambiguous claims among those that name a real paper (real, near-miss, flawed).
    pub expected_ambiguous: usize,
    pub ambiguous: usize,
    pub pending: usize,
    pub calls: usize,
    pub searches: usize,
    /// Claims left out because the recording cannot replay them.
    pub unreplayable: usize,
}

/// Count `rows` (paired with their items); excluded items are skipped.
#[must_use]
pub fn summarize(pairs: &[(&Item, &Row)]) -> Summary {
    let mut summary = Summary::default();
    for (item, row) in pairs {
        if item.class == Class::Excluded {
            continue;
        }
        if row.unreplayable {
            summary.unreplayable += 1;
            continue;
        }
        let label = outcome_label(&row.outcome);
        summary.n += 1;
        summary.calls += row.calls;
        summary.searches += row.searches;
        match label.as_str() {
            "ambiguous" => summary.ambiguous += 1,
            "pending" => summary.pending += 1,
            _ => {}
        }
        *summary.outcomes.entry(label.clone()).or_default() += 1;
        let judgement = judge(item, &row.outcome);
        let accepted = matches!(row.outcome, Outcome::Work { .. });
        match item.class {
            Class::MustNot => {
                summary.must_not_n += 1;
                summary.must_not_accepted += usize::from(accepted);
            }
            Class::Real => {
                summary.real_n += 1;
                match judgement {
                    Judgement::Correct => summary.real_correct += 1,
                    Judgement::WrongWork => summary.real_wrong += 1,
                    Judgement::Unadjudicated => summary.real_unadjudicated += 1,
                    Judgement::NotWork => {}
                }
            }
            Class::NearMiss => {
                summary.near_n += 1;
                match judgement {
                    Judgement::Correct => summary.near_correct += 1,
                    Judgement::WrongWork => summary.near_wrong += 1,
                    Judgement::Unadjudicated => summary.near_unadjudicated += 1,
                    Judgement::NotWork => {}
                }
            }
            Class::Flawed => {
                summary.flawed_n += 1;
                match judgement {
                    Judgement::Correct => summary.flawed_correct += 1,
                    Judgement::WrongWork => summary.flawed_wrong += 1,
                    Judgement::Unadjudicated => summary.flawed_unadjudicated += 1,
                    Judgement::NotWork => {}
                }
            }
            Class::Excluded => {}
        }
        if label == "ambiguous"
            && matches!(item.class, Class::Real | Class::NearMiss | Class::Flawed)
        {
            summary.expected_ambiguous += 1;
        }
    }
    summary
}

impl Summary {
    const fn expected_n(&self) -> usize {
        self.real_n + self.near_n
    }

    fn per_claim(&self, total: usize) -> Option<f64> {
        (self.n > 0).then(|| round(fraction(total, self.n)))
    }

    /// Every metric: counts with Wilson intervals, and the rule-of-three bound at zero.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "n": self.n,
            "unreplayable_excluded": self.unreplayable,
            "outcomes": self.outcomes,
            "false_accept": rate_json(self.must_not_accepted, self.must_not_n),
            "wrong_work": rate_json(self.real_wrong + self.near_wrong, self.expected_n()),
            "real_recall": rate_json(self.real_correct, self.real_n),
            "real_recall_counting_unadjudicated_as_correct":
                rate_json(self.real_correct + self.real_unadjudicated, self.real_n),
            "real_unadjudicated": self.real_unadjudicated,
            "near_miss_recall": rate_json(self.near_correct, self.near_n),
            "exists_wrong_details_accepted_to_right_work":
                rate_json(self.flawed_correct, self.flawed_n),
            "exists_wrong_details_accepted_to_wrong_work":
                rate_json(self.flawed_wrong, self.flawed_n),
            "exists_wrong_details_unadjudicated": self.flawed_unadjudicated,
            "ambiguity_burden": rate_json(self.ambiguous, self.n),
            "ambiguity_burden_among_real_claims":
                rate_json(self.expected_ambiguous, self.expected_n() + self.flawed_n),
            "pending_rate": rate_json(self.pending, self.n),
            "provider_calls_per_claim": self.per_claim(self.calls),
            "title_searches_per_claim": self.per_claim(self.searches),
        })
    }

    /// Ablated minus default, for the numbers an ablation can move.
    #[must_use]
    pub fn delta_json(&self, default: &Self) -> Value {
        let signed = |now: usize, before: usize| {
            i64::try_from(now).unwrap_or(i64::MAX) - i64::try_from(before).unwrap_or(i64::MAX)
        };
        json!({
            "false_accepts": signed(self.must_not_accepted, default.must_not_accepted),
            "wrong_work": signed(self.real_wrong + self.near_wrong,
                default.real_wrong + default.near_wrong),
            "real_correct": signed(self.real_correct, default.real_correct),
            "near_miss_correct": signed(self.near_correct, default.near_correct),
            "exists_wrong_details_accepted":
                signed(self.flawed_correct + self.flawed_wrong + self.flawed_unadjudicated,
                    default.flawed_correct + default.flawed_wrong + default.flawed_unadjudicated),
            "ambiguous": signed(self.ambiguous, default.ambiguous),
            "pending": signed(self.pending, default.pending),
            "provider_calls": signed(self.calls, default.calls),
            "title_searches": signed(self.searches, default.searches),
            "real_recall_rate": self.real_rate_delta(default),
        })
    }

    fn real_rate_delta(&self, default: &Self) -> Option<f64> {
        (self.real_n > 0 && default.real_n > 0).then(|| {
            round(
                fraction(self.real_correct, self.real_n)
                    - fraction(default.real_correct, default.real_n),
            )
        })
    }
}

fn pairs<'a>(items: &'a [Item], rows: &'a [Row]) -> Vec<(&'a Item, &'a Row)> {
    items.iter().zip(rows).collect()
}

fn group_summaries(items: &[Item], rows: &[Row]) -> BTreeMap<String, Summary> {
    let mut groups: BTreeMap<String, Vec<(&Item, &Row)>> = BTreeMap::new();
    for pair in pairs(items, rows) {
        if pair.0.class != Class::Excluded {
            groups.entry(pair.0.group.clone()).or_default().push(pair);
        }
    }
    groups
        .into_iter()
        .map(|(group, members)| (group, summarize(&members)))
        .collect()
}

fn changed(items: &[Item], before: &[Row], after: &[Row]) -> Vec<Value> {
    items
        .iter()
        .zip(before.iter().zip(after))
        .filter(|(_, (a, b))| a.outcome != b.outcome)
        .map(|(item, (a, b))| {
            json!({"id": item.id, "class": format!("{:?}", item.class),
                "default": a.outcome.describe(), "ablated": b.outcome.describe()})
        })
        .collect()
}

/// The ablations scored: default plus each single switch.
#[must_use]
pub fn ablations() -> Vec<(&'static str, Ablation)> {
    Ablation::singles().to_vec()
}

/// Score one set: the default run, a repeat of it (to show nondeterminism), and one run per
/// ablation when `with_ablations`. Returns the metrics and the default run's rows.
pub async fn score_set(
    database_url: &str,
    responses: &Responses,
    items: &[Item],
    with_ablations: bool,
) -> Result<(Value, Vec<Row>), Failure> {
    let default = run_items(database_url, responses, items, Ablation::default()).await?;
    let repeat = run_items(database_url, responses, items, Ablation::default()).await?;
    let headline = summarize(&pairs(items, &default.rows));
    let excluded: Vec<(&Item, &Row)> = items
        .iter()
        .zip(&default.rows)
        .filter(|(item, _)| item.class == Class::Excluded)
        .collect();
    let mut excluded_outcomes: BTreeMap<String, usize> = BTreeMap::new();
    for (_, row) in &excluded {
        *excluded_outcomes
            .entry(outcome_label(&row.outcome))
            .or_default() += 1;
    }
    let unreplayable_ids: Vec<&str> = items
        .iter()
        .zip(&default.rows)
        .filter(|(item, row)| row.unreplayable && item.class != Class::Excluded)
        .map(|(item, _)| item.id.as_str())
        .collect();
    let groups: BTreeMap<String, Value> = group_summaries(items, &default.rows)
        .into_iter()
        .map(|(group, summary)| (group, summary.to_json()))
        .collect();
    let mut ablation_results = serde_json::Map::new();
    if with_ablations {
        for (name, ablation) in ablations() {
            let run = run_items(database_url, responses, items, ablation).await?;
            // Compare on the claims both arms could replay.
            let common: Vec<usize> = (0..items.len())
                .filter(|&i| !default.rows[i].unreplayable && !run.rows[i].unreplayable)
                .collect();
            let default_common: Vec<(&Item, &Row)> = common
                .iter()
                .map(|&i| (&items[i], &default.rows[i]))
                .collect();
            let ablated_common: Vec<(&Item, &Row)> =
                common.iter().map(|&i| (&items[i], &run.rows[i])).collect();
            let (before, after) = (summarize(&default_common), summarize(&ablated_common));
            let flipped: Vec<Value> = changed(items, &default.rows, &run.rows)
                .into_iter()
                .filter(|entry| {
                    common
                        .iter()
                        .any(|&i| entry["id"].as_str() == Some(items[i].id.as_str()))
                })
                .collect();
            ablation_results.insert(
                name.to_owned(),
                json!({
                    "claims_compared": common.len(),
                    "unreplayable_in_either_arm": items.len() - common.len(),
                    "default_metrics_on_compared_claims": before.to_json(),
                    "metrics": after.to_json(),
                    "delta_vs_default": after.delta_json(&before),
                    "changed_outcomes": flipped,
                }),
            );
        }
    }
    let scored = json!({
        "metrics": headline.to_json(),
        "by_group": groups,
        "excluded": {"n": excluded.len(), "outcomes": excluded_outcomes},
        "unreplayable_claims": unreplayable_ids,
        "unrecorded_urls": default.misses,
        "default_repeat_changed_outcomes": changed(items, &default.rows, &repeat.rows),
        "ablations": ablation_results,
    });
    Ok((scored, default.rows))
}

/// Build the items of a held-out set from its `cases.json` (E1 or E3).
pub fn load_set(directory: &Path, set: &'static str) -> Result<(Vec<Item>, Value), Failure> {
    let cases: Value = serde_json::from_slice(&std::fs::read(directory.join("cases.json"))?)?;
    let mut items = Vec::new();
    for (list, class) in [
        ("real", Class::Real),
        ("near_miss", Class::NearMiss),
        ("fake", Class::MustNot),
    ] {
        for case in cases[list].as_array().map_or(&[][..], Vec::as_slice) {
            let mut truths: Vec<String> = case["truth"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .collect();
            truths.extend(
                case["alt_truth"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_owned)),
            );
            items.push(Item {
                id: case["id"].as_str().unwrap_or_default().to_owned(),
                set,
                group: case["kind"].as_str().unwrap_or("?").to_owned(),
                class,
                claim: serde_json::from_value(case["claim"].clone())?,
                truths,
                detail: Value::Null,
            });
        }
    }
    Ok((items, cases["status"].clone()))
}

pub fn read_responses(directory: &Path) -> Result<Responses, Failure> {
    Ok(serde_json::from_slice(&std::fs::read(
        directory.join("responses.json"),
    )?)?)
}

/// The two labellers' labels for one claim, reconciled.
#[derive(Clone, Debug)]
pub enum Reconciled {
    Agreed { label: String, truths: Vec<String> },
    SpotCheck { reason: &'static str },
    Unlabelled,
}

fn label_of(entry: &Value) -> &str {
    entry["label"].as_str().unwrap_or("")
}

fn id_of(entry: &Value) -> Option<String> {
    entry["openalex_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// Agreed labels are ground truth; a disagreement, an `unsure`, a label only one labeller
/// gave, or a label outside the known set goes to the spot check.
#[must_use]
pub fn reconcile(a: Option<&Value>, b: Option<&Value>) -> Reconciled {
    let (a, b) = match (a, b) {
        (None, None) => return Reconciled::Unlabelled,
        (Some(a), Some(b)) => (a, b),
        _ => {
            return Reconciled::SpotCheck {
                reason: "only_one_labeller",
            };
        }
    };
    let known = ["exists", "exists_wrong_details", "does_not_exist"];
    if label_of(a) == "unsure" || label_of(b) == "unsure" {
        return Reconciled::SpotCheck { reason: "unsure" };
    }
    if label_of(a) != label_of(b) {
        return Reconciled::SpotCheck {
            reason: "label_disagreement",
        };
    }
    if !known.contains(&label_of(a)) {
        return Reconciled::SpotCheck {
            reason: "unrecognised_label",
        };
    }
    let mut truths: Vec<String> = Vec::new();
    for id in [id_of(a), id_of(b)].into_iter().flatten() {
        if !truths.iter().any(|seen| seen.eq_ignore_ascii_case(&id)) {
            truths.push(id);
        }
    }
    Reconciled::Agreed {
        label: label_of(a).to_owned(),
        truths,
    }
}

/// The labels file: a list of `{batch, A: [...], B: [...]}`, by claim ID.
fn labels_by_claim(labels: &Value) -> (BTreeMap<String, Value>, BTreeMap<String, Value>) {
    let (mut a, mut b) = (BTreeMap::new(), BTreeMap::new());
    for batch in labels.as_array().into_iter().flatten() {
        for (side, target) in [("A", &mut a), ("B", &mut b)] {
            for entry in batch[side].as_array().into_iter().flatten() {
                if let Some(id) = entry["claim_id"].as_str() {
                    target.insert(id.to_owned(), entry.clone());
                }
            }
        }
    }
    (a, b)
}

#[allow(clippy::cast_possible_truncation)] // a publication year
fn claim_of(row: &Value) -> Claim {
    let text = |key: &str| row[key].as_str().map(str::to_owned);
    Claim {
        title: text("title").unwrap_or_default(),
        authors: row["authors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|author| author.as_str().map(str::to_owned))
            .collect(),
        year: row["year"].as_i64().map(|year| year as i32),
        venue: text("venue"),
        doi: text("doi"),
        arxiv: text("arxiv"),
        raw: None,
    }
}

/// E2 items from `claims.json` and `labels.json`, plus the spot-check list.
pub struct E2Items {
    pub items: Vec<Item>,
    pub spot_check: Vec<Value>,
    pub unlabelled: Vec<String>,
}

pub fn load_e2(directory: &Path) -> Result<E2Items, Failure> {
    let claims: Vec<Value> =
        serde_json::from_slice(&std::fs::read(directory.join("claims.json"))?)?;
    let labels: Value = serde_json::from_slice(&std::fs::read(directory.join("labels.json"))?)?;
    let (a, b) = labels_by_claim(&labels);
    let mut result = E2Items {
        items: Vec::new(),
        spot_check: Vec::new(),
        unlabelled: Vec::new(),
    };
    for row in &claims {
        let id = row["claim_id"].as_str().unwrap_or_default().to_owned();
        let model = row["model"].as_str().unwrap_or("?").to_owned();
        let reconciled = reconcile(a.get(&id), b.get(&id));
        let (class, truths, detail) = match &reconciled {
            Reconciled::Agreed { label, truths } => {
                let class = match label.as_str() {
                    "exists" => Class::Real,
                    "exists_wrong_details" => Class::Flawed,
                    _ => Class::MustNot,
                };
                let fields = |entry: Option<&Value>| entry.map(|e| e["wrong_fields"].clone());
                (
                    class,
                    truths.clone(),
                    json!({"label": label,
                        "wrong_fields_A": fields(a.get(&id)),
                        "wrong_fields_B": fields(b.get(&id))}),
                )
            }
            Reconciled::SpotCheck { reason } => {
                result.spot_check.push(json!({
                    "claim_id": id,
                    "model": model,
                    "reason": reason,
                    "claim": {"title": row["title"], "authors": row["authors"],
                        "year": row["year"], "doi": row["doi"], "arxiv": row["arxiv"],
                        "venue": row["venue"]},
                    "label_A": a.get(&id),
                    "label_B": b.get(&id),
                    "evidence_A": a.get(&id).map(|e| e["evidence"].clone()),
                    "evidence_B": b.get(&id).map(|e| e["evidence"].clone()),
                    "verifier_verdict": null,
                }));
                (Class::Excluded, Vec::new(), Value::Null)
            }
            Reconciled::Unlabelled => {
                result.unlabelled.push(id.clone());
                (Class::Excluded, Vec::new(), Value::Null)
            }
        };
        result.items.push(Item {
            id,
            set: "e2",
            group: model,
            class,
            claim: claim_of(row),
            truths,
            detail,
        });
    }
    Ok(result)
}

/// Fill each spot-check entry's `verifier_verdict` from a default run of the same items.
pub fn attach_verdicts(spot_check: &mut [Value], items: &[Item], rows: &[Row]) {
    let by_id: BTreeMap<&str, &Row> = items
        .iter()
        .zip(rows)
        .map(|(item, row)| (item.id.as_str(), row))
        .collect();
    for entry in spot_check {
        let Some(row) = entry["claim_id"].as_str().and_then(|id| by_id.get(id)) else {
            continue;
        };
        entry["verifier_verdict"] = json!({
            "outcome": outcome_label(&row.outcome),
            "detail": row.outcome.describe(),
            "candidates": row.candidates,
        });
    }
}
