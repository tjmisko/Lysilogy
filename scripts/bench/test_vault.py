import hashlib
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from vault import generate, specimen


class VaultTests(unittest.TestCase):
    def should_generate_identical_vaults_when_given_the_same_seed(self):
        with tempfile.TemporaryDirectory() as directory:
            first = generate(Path(directory) / "first", count=8, seed=19)
            second = generate(Path(directory) / "second", count=8, seed=19)
            self.assertEqual(first, second)
            self.assertEqual(len({row["path"] for row in first["papers"]}), 8)
            for row in first["papers"]:
                for name in ("first", "second"):
                    content = (Path(directory) / name / "papers" / row["path"]).read_bytes()
                    self.assertEqual(hashlib.sha256(content).hexdigest(), row["sha256"])

    def should_change_pdf_content_when_the_seed_changes(self):
        self.assertNotEqual(specimen(19, 0)[1], specimen(20, 0)[1])

    def should_resume_exact_files_when_generation_is_repeated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "vault"
            first = generate(root, count=2)
            self.assertEqual(generate(root, count=2), first)
            with self.assertRaises(ValueError):
                generate(root, count=3)

    def should_preserve_existing_content_when_a_generated_file_was_changed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "vault"
            first = generate(root, count=1)
            path = root / "papers" / first["papers"][0]["path"]
            path.write_bytes(b"changed file")
            with self.assertRaises(ValueError):
                generate(root, count=1)
            self.assertEqual(path.read_bytes(), b"changed file")

    def should_refuse_existing_directories_when_no_ownership_marker_exists(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                generate(Path(directory), count=1)

    def should_refuse_symlink_components_when_the_output_escapes_its_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "outside").mkdir()
            (root / "link").symlink_to(root / "outside", target_is_directory=True)
            with self.assertRaises(ValueError):
                generate(root / "link" / "vault", count=1)
            self.assertFalse((root / "outside/vault").exists())

    def should_extract_valid_text_and_metadata_when_poppler_reads_a_specimen(self):
        self.assertIsNotNone(shutil.which("pdftotext"))
        self.assertIsNotNone(shutil.which("pdfinfo"))
        with tempfile.TemporaryDirectory() as directory:
            _, pdf, metadata = specimen(19, 0)
            path = Path(directory) / "sample.pdf"
            path.write_bytes(pdf)
            text = subprocess.check_output(["pdftotext", str(path), "-"], text=True)
            info = subprocess.check_output(["pdfinfo", str(path)], text=True)
            self.assertIn(metadata["title"], text)
            self.assertIn(metadata["authors"][0], info)
            self.assertIn("Pages:", info)


if __name__ == "__main__":
    unittest.main(testLoader=type("ShouldLoader", (unittest.TestLoader,),
                               {"testMethodPrefix": "should_"})())
