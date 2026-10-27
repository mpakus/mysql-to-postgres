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
    def test_perl_reference_preserves_sources_docs_symbols_and_pin_guard(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            corpus = parent / "ora2pg"
            (corpus / "lib").mkdir(parents=True)
            (corpus / "doc").mkdir()
            (corpus / "scripts").mkdir()
            (corpus / "lib/MySQL.pm").write_text("# café\nsub _column_info { }\n")
            (corpus / "doc/Ora2Pg.pod").write_text("=head1 Validation\n")
            (corpus / "scripts/ora2pg").write_text("#!/usr/bin/perl\n")
            (corpus / "README").write_text("MySQL migration\n")
            subprocess.run(["git", "init", "--quiet", str(corpus)], check=True)
            subprocess.run(["git", "-C", str(corpus), "add", "."], check=True)
            subprocess.run(["git", "-C", str(corpus), "-c", "user.name=Fixture",
                            "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false",
                            "commit", "--quiet", "-m", "fixture"], check=True)
            revision = subprocess.check_output(["git", "-C", str(corpus), "rev-parse", "HEAD"], text=True).strip()
            with patch.object(index, "ROOT", parent):
                documents, evidence = index.documents({"path":"ora2pg", "prefix":"ref-ora2pg", "commit":revision})
                self.assertEqual(set(evidence["included"]), {"lib/MySQL.pm", "doc/Ora2Pg.pod", "scripts/ora2pg", "README"})
                source = next(d for d in documents if d["ax_path"] == "lib/MySQL.pm")
                self.assertEqual(source["code"], "# café\nsub _column_info { }\n")
                self.assertEqual(source["defs"], "_column_info")
                self.assertEqual((source["start_line"], source["end_line"]), (1, 2))
                with self.assertRaisesRegex(RuntimeError, "revision differs"):
                    index.documents({"path":"ora2pg", "prefix":"ref-ora2pg", "commit":"wrong"})

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
