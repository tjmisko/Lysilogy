import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run
from vault import generate


def phases(count=8):
    backend = dict(papers=count, catalog_initial_seconds=.5,
                   catalog_populated_no_change_seconds=[.1, .2, .3], production_ingest_workers=1,
                   catalog_discovered_no_change_seconds=[.01, .02, .03], population_seconds=2,
                   populated_papers=count, catalog_populated_initial_seconds=.4,
                   extraction=[dict(workers=worker, papers=count, seconds=seconds, output_sha256="same")
                               for worker, seconds in [(1, 4), (4, 2), (4, 2), (1, 4)]])
    browser = dict(papers=count, home_first_render_ms=[100, 200, 300],
                   search=[dict(milliseconds=value) for value in range(1, 21)])
    return backend, browser


class BenchmarkTests(unittest.TestCase):
    def should_report_all_phase_timings_when_observations_complete(self):
        result = run.summarize(*phases(), 8)
        self.assertEqual(result["catalog_initial_seconds"], .5)
        self.assertEqual(result["catalog_populated_no_change_median_seconds"], .2)
        self.assertEqual(result["home_first_render_median_ms"], 200)
        self.assertEqual(result["search_p95_ms"], 19)
        self.assertEqual(result["extractor_parallel_efficiency"], .5)
        self.assertTrue(result["O27"].startswith("unavailable"))

    def should_reject_missing_invalid_and_mismatched_phases_when_summarizing(self):
        for change in (lambda back, _: back.update(papers=9),
                       lambda _, browser: browser.update(home_first_render_ms=[]),
                       lambda back, _: back.update(catalog_initial_seconds=float("nan")),
                       lambda back, _: back["extraction"][0].update(output_sha256="different"),
                       lambda back, _: back["extraction"][0].update(papers=7)):
            backend, browser = phases()
            change(backend, browser)
            with self.assertRaises(ValueError):
                run.summarize(backend, browser, 8)

    def should_verify_exact_generated_truth_when_the_vault_is_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "vault"
            expected = generate(root, count=3)
            self.assertEqual(run.verify_vault(root), expected)
            (root / "papers/extra.pdf").write_bytes(b"extra")
            with self.assertRaises(ValueError):
                run.verify_vault(root)

    def should_reject_modified_pdf_and_manifest_when_truth_was_tampered(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "vault"
            manifest = generate(root, count=1)
            paper = root / "papers" / manifest["papers"][0]["path"]
            paper.write_bytes(b"different")
            with self.assertRaises(ValueError):
                run.verify_vault(root)
            manifest["papers"][0]["sha256"] = "forged"
            (root / "manifest.json").write_text(json.dumps(manifest))
            with self.assertRaises(ValueError):
                run.verify_vault(root)

    def should_refuse_truth_symlinks_when_a_manifest_has_matching_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "vault"
            generate(root, count=1)
            original = root / "manifest.json"
            outside = Path(temporary) / "manifest.json"
            original.rename(outside)
            original.symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "symlink"):
                run.verify_vault(root)

    def should_keep_atomic_reports_in_the_opened_directory_when_its_path_is_swapped(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            safe, parked, outside = (root / name for name in ("safe", "parked", "outside"))
            safe.mkdir()
            outside.mkdir()
            replace = os.replace
            def swap(*args, **kwargs):
                safe.rename(parked)
                safe.symlink_to(outside, target_is_directory=True)
                return replace(*args, **kwargs)
            with patch("vault.os.replace", side_effect=swap):
                run.atomic_json(safe / "result.json", {"measured": True})
            self.assertEqual(json.loads((parked / "result.json").read_text()), {"measured": True})
            self.assertEqual(list(outside.iterdir()), [])

    def should_refuse_scale_publication_when_only_a_tiny_fixture_was_measured(self):
        backend, browser = phases()
        with self.assertRaisesRegex(ValueError, "10000"):
            run.publish(dict(count=8), backend, browser, [], 1)

    def should_preserve_provider_evidence_when_a_disjoint_scale_collector_publishes(self):
        backend, browser = phases(10000)
        with tempfile.TemporaryDirectory() as temporary, patch.object(run, "ROOT", Path(temporary)), patch.object(run, "implementation", return_value=[]):
            other = Path(temporary) / "eval/inputs/scale/provider-budgets.json"
            other.parent.mkdir(parents=True)
            other.write_text("existing provider evidence")
            run.publish(dict(count=10000), backend, browser, [], 1)
            self.assertEqual(other.read_text(), "existing provider evidence")
            value = json.loads((Path(temporary) / run.COLLECTOR).read_text())
            self.assertEqual(set(value["metrics"]), {"O25", "O26.render", "O26.search"})
            self.assertEqual(value["cost_usd"], 0)

    def should_refuse_publication_when_implementation_changed_during_measurement(self):
        backend, browser = phases(10000)
        with patch.object(run, "implementation", return_value=["new hash"]):
            with self.assertRaisesRegex(ValueError, "implementation changed"):
                run.publish(dict(count=10000), backend, browser, ["old hash"], 1)

    def should_refuse_repository_and_tmp_roots_when_the_cli_validates_storage(self):
        for path in (run.ROOT / "benchmark", Path("/tmp/benchmark"), run.DESIGNATED / "../escape"):
            with self.assertRaises(ValueError):
                run.designated_root(path)


if __name__ == "__main__":
    unittest.main(testLoader=type("ShouldLoader", (unittest.TestLoader,),
                               {"testMethodPrefix": "should_"})())
