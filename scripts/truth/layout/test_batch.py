"""Population, deadlines and fail-closed aggregate outcomes without source execution."""
from contextlib import ExitStack
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import batch
from policy import Refused


class BatchTests(unittest.TestCase):
    def prepare(self, root):
        selection = {"papers": [{"arxiv_id": str(i), "version": 1, "page_count": 1,
                                 "frozen_artifacts": {"pdf": {"path": "synthetic.pdf"}}} for i in range(10)],
                     "go_no_go": {"selected": 10, "minimum_builds": 8,
                                  "minimum_exact_whole_documents": 5,
                                  "required_dpi_for_every_page": [96, 192]}}
        (root / "selection.json").write_text(json.dumps(selection))
        (root / "runtime.json").write_text("{}")
        return {"output_parent": str(root), "confinement_tools": {"synthetic": "pin"}}

    def doubles(self, stack, *, fail_build=None, fail_compare=None):
        stack.enter_context(patch("batch.runtime.verify"))
        stack.enter_context(patch("batch.checked_artifacts"))

        def build(paper, runtime, directory, *, deadline):
            self.assertEqual(runtime["confinement_tools"], {"synthetic": "pin"})
            self.assertEqual(deadline, 900)
            directory.mkdir()
            result = {"status": "unsupported_or_refused" if paper["arxiv_id"] == fail_build else "built",
                      "pdf": {"path": "synthetic-built.pdf"}}
            (directory / "receipt.json").write_text(json.dumps(result))
            return result

        def compare(paper, rebuilt, runtime, directory, count, *, deadline):
            self.assertEqual(deadline, 900)
            if paper["arxiv_id"] == fail_compare:
                raise Refused("synthetic unavailable raster")
            directory.mkdir()
            result = {"status": "compared", "exact_whole_document": True}
            (directory / "receipt.json").write_text(json.dumps(result))
            return result

        builds = stack.enter_context(patch("batch.build_paper", side_effect=build))
        counts = stack.enter_context(patch("batch.page_count", return_value={"count": 1}))
        stack.enter_context(patch("batch.compare_document", side_effect=compare))
        return builds, counts

    def should_keep_failed_papers_in_the_denominator_when_builds_or_rasters_fail(self):
        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            root = Path(temporary)
            config = self.prepare(root)
            builds, counts = self.doubles(stack, fail_build="2", fail_compare="4")
            stack.enter_context(patch("batch.time.monotonic", return_value=0))
            result = batch.run(config, root)
            self.assertEqual([p["arxiv_id"] for p in result["papers"]], [str(i) for i in range(10)])
            self.assertEqual(result["builds"], 9)
            self.assertEqual(result["exact_whole_documents"], 8)
            self.assertEqual(result["papers"][2]["status"], "unsupported_or_refused")
            self.assertEqual(result["papers"][4]["status"], "correspondence_unavailable")
            self.assertEqual(builds.call_count, 10)
            self.assertTrue(all(call.kwargs["deadline"] == 900 for call in counts.call_args_list))
            self.assertTrue(result["exploratory_go"])
            self.assertFalse(result["truth_admission"])

    def should_stop_starting_builds_when_the_batch_has_less_than_one_build_budget(self):
        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            root = Path(temporary)
            config = self.prepare(root)
            builds, _ = self.doubles(stack)
            stack.enter_context(patch("batch.time.monotonic", side_effect=[0, 0, *([811] * 10)]))
            result = batch.run(config, root)
            self.assertEqual(builds.call_count, 1)
            self.assertEqual(len(result["papers"]), 10)
            self.assertTrue(all(p["status"] == "batch_deadline_not_started" for p in result["papers"][1:]))
            self.assertEqual(result["builds"], 1)
            self.assertFalse(result["exploratory_go"])

    def should_retain_all_unstarted_papers_when_runtime_verification_aborts_the_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = self.prepare(root)
            with patch("batch.runtime.verify", side_effect=Refused("changed runtime")), self.assertRaises(Refused):
                batch.run(config, root)
            result = json.loads((root / "experiment/receipt.json").read_text())
            self.assertEqual(len(result["papers"]), 10)
            self.assertTrue(all(p["status"] == "not_started_after_failure" for p in result["papers"]))
            self.assertEqual(result["status"], "failed")
            self.assertFalse(result["exploratory_go"])

    def should_refuse_an_altered_population_or_target_when_the_snapshot_is_invalid(self):
        for change in ("duplicate", "target", "dpi"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                config = self.prepare(root)
                path = root / "selection.json"
                selection = json.loads(path.read_text())
                if change == "duplicate":
                    selection["papers"][-1] = selection["papers"][0]
                elif change == "target":
                    selection["go_no_go"]["minimum_exact_whole_documents"] = 1
                else:
                    selection["go_no_go"]["required_dpi_for_every_page"] = [96]
                path.write_text(json.dumps(selection))
                with patch("batch.build_paper") as build, self.assertRaises(Refused):
                    batch.run(config, root)
                build.assert_not_called()
                self.assertFalse((root / "experiment").exists())


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
