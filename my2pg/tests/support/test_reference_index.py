"""Internal XERJ importer regression; excluded with its tool from source packages."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("reference_index", ROOT / "bin/reference-index.py")
index = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(index)


class ReferenceIndexTests(unittest.TestCase):
    def test_parent_git_retains_revision_scripts_and_excludes_build_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            corpus = parent / "nested"
            (corpus / "bin").mkdir(parents=True)
            (corpus / "target").mkdir()
            (parent / ".gitignore").write_text("nested/target/\n")
            (corpus / "source.rs").write_text("pub fn source() {}\n")
            (corpus / "bin/build-linux").write_text("#!/bin/sh\nexit 0\n")
            (corpus / "bin/build-windows.ps1").write_text("exit 0\n")
            (corpus / "target/ignored.rs").write_text("must not be indexed\n")

            def git(*args):
                return subprocess.run(["git", "-C", str(parent), *args], check=True,
                                      capture_output=True, text=True).stdout.strip()

            git("init", "--quiet")
            git("add", ".")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture")
            with patch.object(index, "ROOT", parent):
                documents, evidence = index.documents({"path": "nested", "prefix": "fixture", "commit": None})
            self.assertEqual(evidence["revision"], git("rev-parse", "HEAD"))
            self.assertEqual(set(evidence["included"]), {"source.rs", "bin/build-linux", "bin/build-windows.ps1"})
            self.assertTrue(all(item["revision"] == evidence["revision"] for item in documents))


if __name__ == "__main__":
    unittest.main()
