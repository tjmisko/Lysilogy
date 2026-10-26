//! Scores the frozen claim verifier on the held-out sets (E1, E2, E3) from recorded
//! provider responses, with the E4 ablations, and writes `eval/claims/results.json`.
//! Replay only: no network, and an empty graph and cold cache for every claim.
//!
//! `LYSILOGY_TEST_DATABASE_URL=... cargo run --example claims_holdout_eval [-- --dir eval/claims]`

#[allow(dead_code)]
#[path = "../tests/claims_holdout/mod.rs"]
mod claims_holdout;
#[allow(dead_code)]
#[path = "../tests/claims_support/mod.rs"]
mod claims_support;

use std::path::{Path, PathBuf};

use claims_holdout::{Class, attach_verdicts, load_e2, load_set, read_responses, score_set};
use claims_support::Failure;
use serde_json::{Value, json};

fn write_json(path: &Path, value: &Value) -> Result<(), Failure> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}

fn rate_of(metrics: &Value, name: &str) -> (Option<f64>, usize, usize) {
    let entry = &metrics[name];
    (
        entry["rate"].as_f64(),
        usize::try_from(entry["k"].as_u64().unwrap_or(0)).unwrap_or(0),
        usize::try_from(entry["n"].as_u64().unwrap_or(0)).unwrap_or(0),
    )
}

/// One pass-bar line: `pass`, `fail` or `not_evaluable`, with what was observed.
fn bar(requirement: &str, status: &str, observed: &Value, note: &str) -> Value {
    json!({"requirement": requirement, "status": status, "observed": observed, "note": note})
}

fn false_accept_bar(set: &str, complete: bool, scored: Option<&Value>) -> Value {
    let requirement = format!("0 false accepts on {set}");
    let Some(scored) = scored else {
        return bar(
            &requirement,
            "not_evaluable",
            &Value::Null,
            "the set was not scored",
        );
    };
    let (_, k, n) = rate_of(&scored["metrics"], "false_accept");
    let observed = scored["metrics"]["false_accept"].clone();
    if n == 0 {
        return bar(
            &requirement,
            "not_evaluable",
            &observed,
            "no claim in the set is labelled a fake or does_not_exist, so the bar is vacuous",
        );
    }
    if !complete {
        return bar(
            &requirement,
            "not_evaluable",
            &observed,
            "the set is partial; this is the count so far, not a result",
        );
    }
    bar(
        &requirement,
        if k == 0 { "pass" } else { "fail" },
        &observed,
        "",
    )
}

fn recall_bar(set: &str, minimum: f64, complete: bool, scored: Option<&Value>) -> Value {
    let requirement = format!("{set} real recall >= {:.0}%", minimum * 100.0);
    let Some(scored) = scored else {
        return bar(
            &requirement,
            "not_evaluable",
            &Value::Null,
            "the set was not scored",
        );
    };
    let observed = scored["metrics"]["real_recall"].clone();
    let (rate, _, n) = rate_of(&scored["metrics"], "real_recall");
    let Some(rate) = rate.filter(|_| n > 0) else {
        return bar(&requirement, "not_evaluable", &observed, "no real claims");
    };
    if !complete {
        return bar(
            &requirement,
            "not_evaluable",
            &observed,
            "the set is partial; this is the rate so far, not a result",
        );
    }
    bar(
        &requirement,
        if rate >= minimum { "pass" } else { "fail" },
        &observed,
        "",
    )
}

#[allow(clippy::too_many_lines)] // one pass over the sets, read top to bottom
#[tokio::main]
async fn main() -> Result<(), Failure> {
    let mut root = PathBuf::from("eval/claims");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--dir" => root = PathBuf::from(args.next().ok_or("--dir needs a value")?),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let url = std::env::var("LYSILOGY_TEST_DATABASE_URL").unwrap_or_else(|_| {
        format!(
            "postgresql://{}@127.0.0.1:5433/lysilogy_test",
            std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned())
        )
    });

    let mut sets = serde_json::Map::new();
    let mut scored_sets: Vec<(&str, bool, Option<Value>)> = Vec::new();
    for set in ["e1", "e3"] {
        let directory = root.join(set);
        let (items, status) = load_set(&directory, if set == "e1" { "e1" } else { "e3" })?;
        let complete = status["complete"].as_bool().unwrap_or(false);
        let responses = read_responses(&directory)?;
        eprintln!("{set}: {} cases, complete={complete}", items.len());
        let (scored, _) = score_set(&url, &responses, &items, true).await?;
        sets.insert(
            set.to_owned(),
            json!({"status": status, "complete": complete, "scored": scored,
                "headline": if complete { "final" } else { "PARTIAL: informational only, not a result" }}),
        );
        scored_sets.push((set, complete, Some(scored)));
    }

    // E2: labels are reconciled whether or not the verifier has been run.
    let e2_dir = root.join("e2");
    let mut e2_scored: Option<Value> = None;
    if e2_dir.join("labels.json").exists() && e2_dir.join("claims.json").exists() {
        let mut loaded = load_e2(&e2_dir)?;
        let mut label_counts = serde_json::Map::new();
        for item in &loaded.items {
            let key = format!("{}:{:?}", item.group, item.class);
            let count = label_counts.entry(key).or_insert(json!(0));
            *count = json!(count.as_u64().unwrap_or(0) + 1);
        }
        let mut entry = json!({
            "claims": loaded.items.len(),
            "agreed": loaded.items.iter().filter(|i| i.class != Class::Excluded).count(),
            "spot_check": loaded.spot_check.len(),
            "unlabelled": loaded.unlabelled.len(),
            "claims_by_model_and_class": label_counts,
        });
        if e2_dir.join("responses.json").exists() {
            let responses = read_responses(&e2_dir)?;
            let (scored, rows) = score_set(&url, &responses, &loaded.items, true).await?;
            attach_verdicts(&mut loaded.spot_check, &loaded.items, &rows);
            entry["status"] = json!("scored");
            entry["scored"] = scored.clone();
            e2_scored = Some(scored);
        } else {
            entry["status"] = json!("blocked");
            entry["blocked_reason"] = json!(
                "eval/claims/e2/responses.json does not exist: the E2 provider responses could \
                 not be recorded because the OpenAlex free daily budget was exhausted. Run \
                 `cargo run --example e2_verify -- --dir eval/claims/e2` once the budget resets \
                 or with LYSILOGY_OPENALEX_API_KEY set, then rerun this example."
            );
        }
        write_json(
            &e2_dir.join("spot_check.json"),
            &Value::Array(loaded.spot_check),
        )?;
        sets.insert("e2".to_owned(), entry);
    }

    let e1 = scored_sets.iter().find(|s| s.0 == "e1");
    let pass_bar = json!([
        false_accept_bar("E2", true, e2_scored.as_ref()),
        false_accept_bar("E1", e1.is_some_and(|s| s.1), e1.and_then(|s| s.2.as_ref()),),
        recall_bar("E2", 0.85, true, e2_scored.as_ref()),
        recall_bar(
            "E1",
            0.90,
            e1.is_some_and(|s| s.1),
            e1.and_then(|s| s.2.as_ref())
        ),
    ]);
    let results = json!({
        "verifier_reference": "decide_claim and verify_claim frozen at 0913e7f; the only change since is the Ablation switch, whose default reproduces the shipped behaviour",
        "method": "offline replay of recorded provider responses; graph and response cache emptied before every claim, so the local-match shortcut finds nothing and provider_calls_per_claim is a cold-cache count; wilson95 is the 95% Wilson score interval; rule_of_three_upper is the 95% upper bound on a rate seen zero times",
        "ablation_definitions": {
            "split_title_retry": "no retry of an empty title search on the two halves of the title",
            "duplicate_collapsing": "every OpenAlex record of a paper is kept instead of merging duplicates",
            "year_window": "years must be equal instead of within one (every year comparison in decide_claim)",
            "surname_check": "author surnames are never compared in decide_claim",
            "identifier_checks": "an identifier's record verifies whatever its year and authors (title must still agree)",
        },
        "notes": [
            "E1 is partial (status.complete is false): its numbers are informational and the E1 pass-bar lines are not_evaluable until the set is recorded.",
            "E2 has no verifier verdicts yet because eval/claims/e2/responses.json does not exist (OpenAlex free budget exhausted); labels are reconciled and spot_check.json is written, and the E2 metrics, per-model breakdown and ablations appear here once responses.json exists and this example is rerun.",
            "No E2 claim is labelled does_not_exist, so the E2 false-accept bar is vacuous; exists_wrong_details claims (conflations, wrong authors or identifiers) are reported separately as accepted to the right Work, the wrong Work, or not accepted.",
            "Claims whose replay needs a response the recording lacks (it was recorded with other Works in the graph, so the local-match shortcut answered) are excluded as unreplayable and listed; ablation deltas compare only claims replayable in both arms.",
            "Empty-graph scores differ from the committed README baseline (recorded with an accumulating graph), by design.",
            "default_repeat_changed_outcomes lists claims whose outcome changed between two identical default runs (nondeterminism).",
        ],
        "sets": Value::Object(sets),
        "pass_bar": pass_bar,
    });
    write_json(&root.join("results.json"), &results)?;
    eprintln!("wrote {}", root.join("results.json").display());
    Ok(())
}
