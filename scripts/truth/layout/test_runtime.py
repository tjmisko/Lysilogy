"""Trusted runtime discovery cannot execute deposited inputs or follow links."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import runtime
from policy import Refused


class RuntimeTests(unittest.TestCase):
    def should_reject_deposited_executables_when_dependency_discovery_is_requested(self):
        with patch("runtime.subprocess.run") as run, self.assertRaises(Refused):
            runtime.dependency_mounts(Path("/synthetic-untrusted/source-binary"))
        run.assert_not_called()

    def should_hash_symlink_spelling_when_a_runtime_link_points_outside_the_mounted_tree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tree = root / "runtime"
            tree.mkdir()
            outside = root / "synthetic-sentinel"
            outside.write_text("first")
            (tree / "external").symlink_to(outside)
            (tree / "regular").write_text("inside")
            original = runtime.tree_inventory(tree)
            outside.write_text("second")
            self.assertEqual(runtime.tree_inventory(tree), original)
            self.assertEqual(original[0], {"name": "external", "kind": "symlink", "target": str(outside)})
            (tree / "regular").write_text("changed")
            self.assertNotEqual(runtime.tree_inventory(tree), original)

    def should_refuse_private_named_entries_when_inventorying_a_runtime_tree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            # Simulate the forbidden name; never create or open such a file.
            with patch("runtime.os.walk", return_value=[(str(root), [], [".env"])]), patch("runtime.binding") as bind, self.assertRaises(Refused):
                runtime.tree_inventory(root)
            bind.assert_not_called()


if __name__ == "__main__":
    loader = unittest.TestLoader()
    loader.testMethodPrefix = "should_"
    unittest.main(testLoader=loader, verbosity=2)
