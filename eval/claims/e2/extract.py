"""Extract every claim from raw/<model>/<goal>.json into claims.json (E2)."""
import json
import pathlib

HERE = pathlib.Path(__file__).parent
MODELS = {"deepseek": "ds", "fable": "fa"}


def plan_of(model, record):
    if model == "fable":
        plan = record.get("structured_output")
        if plan is None:
            plan = json.loads(record["result"])
        return plan
    return json.loads(record["content"])


goals = [g["id"] for g in json.loads((HERE / "goals.json").read_text())]
claims, dropped, missing = [], [], []
for model, prefix in MODELS.items():
    seen = {}
    for goal_number, goal in enumerate(goals, 1):
        path = HERE / "raw" / model / f"{goal}.json"
        if not path.exists():
            missing.append(f"{model}/{goal}")
            continue
        plan = plan_of(model, json.loads(path.read_text()))
        index = 0
        for topic in plan["topics"]:
            for claim in topic["claims"]:
                index += 1
                claim_id = f"{prefix}-{goal_number:02d}-{index:02d}"
                key = json.dumps(
                    [
                        " ".join(claim["title"].lower().split()),
                        claim["authors"],
                        claim["year"],
                        claim["venue"],
                        claim["doi"],
                        claim["arxiv"],
                    ]
                )
                if key in seen:
                    seen[key]["also_in"].append(claim_id)
                    dropped.append(claim_id)
                    continue
                row = {
                    "claim_id": claim_id,
                    "model": model,
                    "goal": goal,
                    "topic": topic["label"],
                    "title": claim["title"],
                    "authors": claim["authors"],
                    "year": claim["year"],
                    "venue": claim["venue"],
                    "doi": claim["doi"],
                    "arxiv": claim["arxiv"],
                    "role": claim["role"],
                    "also_in": [],
                }
                seen[key] = row
                claims.append(row)
(HERE / "claims.json").write_text(json.dumps(claims, indent=2, ensure_ascii=False) + "\n")
by_model = {m: sum(c["model"] == m for c in claims) for m in MODELS}
print(by_model, "exact duplicates dropped:", dropped, "missing:", missing)
