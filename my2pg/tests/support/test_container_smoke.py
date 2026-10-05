import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import call, patch


SCRIPT = Path(__file__).with_name("container_smoke.py")
SPEC = importlib.util.spec_from_file_location("container_smoke", SCRIPT)
container_smoke = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(container_smoke)


class ContainerSmokeInputTests(unittest.TestCase):
    def test_source_state_uses_head_and_reports_clean_or_dirty_tree(self):
        for status, expected_dirty in (("", False), (" M src/main.rs\n?? new.txt\n", True)):
            with self.subTest(dirty=expected_dirty), patch.object(
                    container_smoke, "run",
                    side_effect=[
                        subprocess.CompletedProcess(["git"], 0, "abc123\n", ""),
                        subprocess.CompletedProcess(["git"], 0, status, ""),
                    ]) as run:
                self.assertEqual(container_smoke.source_state(), ("abc123", expected_dirty))

            run.assert_has_calls([
                call(["git", "rev-parse", "HEAD"]),
                call(["git", "status", "--porcelain=v1", "--untracked-files=all"]),
            ])

    def test_migration_mounts_only_read_only_ca_from_tls(self):
        mounts = container_smoke.migration_volumes(
            Path("/fixture/migration.toml"),
            Path("/fixture/tls/ca.pem"),
            Path("/reports"),
        )
        self.assertEqual(mounts, [
            "--volume", "/fixture/migration.toml:/migration.toml:ro",
            "--volume", "/fixture/tls/ca.pem:/tls/ca.pem:ro",
            "--volume", "/reports:/artifacts:rw",
        ])


class ContainerSmokeCleanupTests(unittest.TestCase):
    def test_cleanup_attempts_both_resources_and_records_each_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory)
            report = {"status": "passed"}
            metadata = {"state": "ready"}
            image_remove = subprocess.CompletedProcess(
                ["docker", "image", "rm"], 1, "", "image is still in use"
            )
            with patch.object(container_smoke.harness, "stop", side_effect=RuntimeError("stop failed")) as stop, \
                    patch.object(container_smoke, "run", return_value=image_remove) as run:
                container_smoke.cleanup(metadata, "my2pg:test", report, artifact)

            stop.assert_called_once_with(metadata)
            run.assert_called_once_with(["docker", "image", "rm", "--force", "my2pg:test"], check=False)
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["cleanup_errors"], [
                "fixture cleanup failed: stop failed",
                "image cleanup failed: image is still in use",
            ])
            self.assertEqual(json.loads((artifact / "result.json").read_text()), report)


if __name__ == "__main__":
    unittest.main()
