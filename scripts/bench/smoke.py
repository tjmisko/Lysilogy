#!/usr/bin/env python3
"""Explicit eight-paper, loopback-only Playwright verification; never publishes scale metrics."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import run
from vault import generate


def main():
    repo = run.ROOT
    environment = {**os.environ, "CARGO_BUILD_JOBS": "1", "CARGO_PROFILE_DEV_DEBUG": "0",
                   "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_INCREMENTAL": "0", "CARGO_TARGET_DIR": str(repo / "target")}
    subprocess.run(["cargo", "build", "--offline", "--bin", "lysilogy", "--example", "scale_bench"],
                   cwd=repo, env=environment, check=True)
    subprocess.run(["npm", "run", "build"], cwd=repo / "web", check=True)
    # This bounded test is the sole temporary-PDF exception: exactly eight tiny
    # specimens, removed with their isolated artifacts when the test completes.
    with tempfile.TemporaryDirectory(prefix="lysilogy-bench-tiny-") as directory:
        root = Path(directory) / "vault"
        generate(root, count=8)
        run.verify_vault(root)
        output = root / "tiny-run"
        output.mkdir()
        backend = json.loads(subprocess.check_output([
            str(repo / "target/debug/examples/scale_bench"), "--library", str(root / "papers"),
            "--data", str(output / "catalog-data"), "--expected-count", "8", "--repeats", "3", "--extraction-limit", "8"], text=True))
        (output / "browser-tmp").mkdir()
        with run.serve(repo / "target/debug/lysilogy", root, output, repo / "web/dist") as url:
            subprocess.run(["node", str(repo / "web/scripts/scale-bench.mjs"), url, "8", "3",
                            str(output / "browser.json"), str(output / "home.png")], check=True,
                           env={**os.environ, "TMPDIR": str(output / "browser-tmp")})
        browser = json.loads((output / "browser.json").read_text())
        summary = run.summarize(backend, browser, 8)
        run.verify_vault(root)
        run.atomic_json(repo / "target/bench-fixture-report.json",
                        dict(kind="tiny 8-paper fixture; not a scale metric", profile="debug",
                             backend=backend, browser=browser, summary=summary))
        shutil.copyfile(output / "home.png", repo / "target/bench-fixture-home.png")
        print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
