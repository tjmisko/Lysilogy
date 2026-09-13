#!/usr/bin/env python3
"""Measure O30 from the production Rust admission trace, without network/models."""
import hashlib
import json
from pathlib import Path
import os
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
TRUTH = "eval/truth/provider-budget-batch.json"
TRACE = "eval/inputs/provider-budget-observations.json"


def evidence(path, version):
    return {"path": path, "version": version, "sha256": hashlib.sha256((ROOT / path).read_bytes()).hexdigest()}


def validate_trace(events, fixture):
    """Independent checks against observed grants, not the Rust decision labels."""
    policy = fixture["policy"]
    seen = set()
    states = {provider: {"last": None, "window": None, "count": 0, "cooldown": 0} for provider in fixture["providers"]}
    admitted_by_provider = {provider: 0 for provider in fixture["providers"]}
    violations = 0
    deferrals = cooldowns = restarts = 0
    for event in events:
        state = states[event["provider"]]
        timestamp = event["at_ms"]
        if event["event"] == "cooldown":
            state["cooldown"] = max(state["cooldown"], timestamp + fixture["cooldown_seconds"] * 1000)
            cooldowns += 1
        elif event["event"] == "restart":
            restarts += 1
        elif event["event"] in ("wait", "deferred"):
            deferrals += event["event"] == "deferred"
        elif event["event"] == "granted":
            reference = event["reference"]
            if reference in seen or not 0 <= reference < fixture["references"]:
                violations += 1
            seen.add(reference)
            admitted_by_provider[event["provider"]] += 1
            if state["last"] is not None and timestamp - state["last"] < policy["spacing_ms"]:
                violations += 1
            if timestamp < state["cooldown"]:
                violations += 1
            if state["window"] is None or timestamp >= state["window"] + policy["window_ms"]:
                state["window"], state["count"] = timestamp, 0
            state["count"] += 1
            if state["count"] > policy["requests_per_window"]:
                violations += 1
            state["last"] = timestamp
        else:
            raise ValueError("Unknown trace event")
    if len(seen) != fixture["references"] or not deferrals or not cooldowns or not restarts:
        raise ValueError("Incomplete trace: every reference must run, including exhausted windows, cooldowns and restarts")
    if "cooldown_every" in fixture and cooldowns != sum(count // fixture["cooldown_every"] for count in admitted_by_provider.values()):
        raise ValueError("Cooldown coverage differs from the configured fixture")
    if "restart_every" in fixture and restarts != sum(count // fixture["restart_every"] for count in admitted_by_provider.values()):
        raise ValueError("Restart coverage differs from the configured fixture")
    if any(count == 0 for count in admitted_by_provider.values()):
        raise ValueError("Every configured provider must receive reference requests")
    return {"cases": len(seen), "violations": violations, "deferrals": deferrals, "cooldowns": cooldowns, "restarts": restarts, "admitted_by_provider": admitted_by_provider}


def main():
    started = time.monotonic()
    fixture = json.loads((ROOT / TRUTH).read_text())
    environment = dict(os.environ, CARGO_BUILD_JOBS="1", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0", CARGO_NET_OFFLINE="true")
    run = subprocess.run(["cargo", "run", "--quiet", "--offline", "--example", "provider_budget_eval", "--", TRUTH], cwd=ROOT, env=environment, check=True, capture_output=True, text=True)
    events = json.loads(run.stdout)
    result = validate_trace(events, fixture)
    (ROOT / TRACE).parent.mkdir(parents=True, exist_ok=True)
    (ROOT / TRACE).write_text(json.dumps({"summary": result, "events": events}, sort_keys=True) + "\n")
    path = ROOT / "eval/inputs/scale/provider-budgets.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    value = {"schema_version": 1, "suite": "scale", "collector": "provider-budgets-v1",
             "implementation": [evidence(file, "e7.1-v1") for file in
                                ("src/citation_graph/budget.rs", "src/citation_graph/cache.rs", "src/citation_graph/http.rs", "src/citation_graph/mod.rs",
                                 "examples/provider_budget_eval.rs", "scripts/eval/provider-budgets.py", "Cargo.toml", "Cargo.lock")],
             "truth_sets": {"recorded fixtures": evidence(TRUTH, "provider-budget-batch-v1")},
             "metrics": {"O30": {"sample": {"method": "value", "value": result["violations"]}, "cases": result["cases"], "evidence": [evidence(TRACE, "provider-budget-observations-v1")]}},
             "cost_usd": 0, "wall_seconds": time.monotonic() - started}
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
