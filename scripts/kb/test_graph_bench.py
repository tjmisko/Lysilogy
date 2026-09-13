import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("graph_bench", Path(__file__).with_name("graph-bench.py"))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class GraphCollectorTests(unittest.TestCase):
    def report(self):
        return {"works": 500000, "edges": 3000000, "queries": 200, "fts5": True,
                "samples": [{"stratum": "hub" if index % 5 == 0 else "uniform", "correct": True,
                             "nodes_truncated": False, "edges_truncated": False, "nodes": 100,
                             "milliseconds": index + 1.0} for index in range(200)]}

    def should_accept_full_observations_when_every_query_matches_truth(self):
        report = self.report()
        self.assertEqual(200, len(bench.validate(report, report)))

    def should_reject_smaller_graph_when_a_fixture_is_presented_as_scale(self):
        report = self.report()
        truth = dict(report)
        report["works"] = 1000
        with self.assertRaises(ValueError):
            bench.validate(report, truth)

    def should_reject_truncated_results_when_a_fast_query_drops_neighbors(self):
        report = self.report()
        report["samples"][4]["nodes_truncated"] = True
        with self.assertRaises(ValueError):
            bench.validate(report, report)

    def should_reject_missing_hubs_when_the_query_mix_is_changed(self):
        report = self.report()
        report["samples"][0]["stratum"] = "uniform"
        with self.assertRaises(ValueError):
            bench.validate(report, report)


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.TextTestRunner(verbosity=2).run(loader.loadTestsFromTestCase(GraphCollectorTests))
