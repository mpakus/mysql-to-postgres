import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("github_release", ROOT / "bin/github-release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
REVISION = "a" * 40


class BuildEntryTests(unittest.TestCase):
    def test_dispatch_arguments_and_failure_status_from_any_directory(self):
        with tempfile.TemporaryDirectory(prefix="build entry ") as temporary:
            root = Path(temporary)
            (root / "bin").mkdir()
            shutil.copy2(ROOT / "bin/buld", root / "bin/buld")
            for name in ("linux", "macos"):
                helper = root / "bin" / f"build-{name}"
                helper.write_text(f'#!/bin/sh\nprintf "%s\\n" "{name}" "$@"\nexit 17\n')
                helper.chmod(0o755)
            uname = root / "bin/uname"
            uname.write_text('#!/bin/sh\necho "$TEST_OS"\n')
            uname.chmod(0o755)
            for host, args, expected in [
                ("Darwin", [], "macos\n"), ("Linux", [], "linux\n"),
                ("Darwin", ["linux", "--out-dir", "a path"], "linux\n--out-dir\na path\n"),
            ]:
                with self.subTest(host=host, args=args):
                    result = subprocess.run([str(root / "bin/buld"), *args], cwd="/",
                                            env=dict(os.environ, PATH=f'{root / "bin"}:{os.environ["PATH"]}', TEST_OS=host),
                                            capture_output=True, text=True)
                    self.assertEqual(result.returncode, 17)
                    self.assertEqual(result.stdout, expected)
            for args in [["windows"], ["bad-platform"]]:
                self.assertEqual(subprocess.run([str(root / "bin/buld"), *args], capture_output=True).returncode, 2)
            self.assertEqual(subprocess.run([str(root / "bin/buld"), "--help"], capture_output=True).returncode, 0)


class ReleaseTests(unittest.TestCase):
    def test_stamp_keeps_locked_dependencies_and_is_repeatable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ("Cargo.toml", "Cargo.lock"):
                shutil.copy2(ROOT / name, root / name)
            original_lock = (root / "Cargo.lock").read_text()
            self.assertEqual(release.stamp("42", root), "0.1.42")
            stamped = (root / "Cargo.toml").read_text()
            self.assertIn('version = "0.1.42"', stamped)
            old_version = original_lock.split('name = "my2pg"\nversion = "', 1)[1].split('"', 1)[0]
            self.assertEqual((root / "Cargo.lock").read_text(), original_lock.replace(
                f'name = "my2pg"\nversion = "{old_version}"', 'name = "my2pg"\nversion = "0.1.42"'))
            self.assertEqual(release.stamp("42", root), "0.1.42")
            self.assertEqual((root / "Cargo.toml").read_text(), stamped)
            self.assertEqual(release.stamp("43", root), "0.1.43")

    def test_invalid_stamp_leaves_both_files_untouched(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = '[package]\nname = "my2pg"\nversion = "0.1.0"\n'
            (root / "Cargo.toml").write_text(manifest)
            (root / "Cargo.lock").write_text("invalid lock")
            for number in ("0", "01", "-1", "1;bad", "42"):
                with self.assertRaises(ValueError):
                    release.stamp(number, root)
                self.assertEqual((root / "Cargo.toml").read_text(), manifest)
                self.assertEqual((root / "Cargo.lock").read_text(), "invalid lock")

    def assets(self, directory):
        for target in release.TARGETS:
            name = f"my2pg-0.1.42-{target}"
            archive = directory / f"{name}.tar.gz"
            info = json.dumps({"version": "0.1.42", "target": target,
                               "source": {"revision": REVISION}}).encode()
            with tarfile.open(archive, "w:gz") as bundle:
                entry = tarfile.TarInfo(f"{name}/BUILD-INFO.json")
                entry.size = len(info)
                bundle.addfile(entry, io.BytesIO(info))
            archive.with_suffix(".gz.sha256").write_text(f"{release.digest(archive)}  {archive.name}\n")

    def test_publish_retries_drafts_keeps_published_assets_and_does_not_downgrade_latest(self):
        existing = {"tag_name": "v0.1.42", "target_commitish": REVISION, "draft": True, "prerelease": False}
        newer = dict(existing, tag_name="v0.1.43", draft=False)
        for releases, create, upload, latest in [
            ([], True, True, "true"), ([existing, newer], False, True, "false"),
            ([dict(existing, draft=False)], False, False, None),
        ]:
            with self.subTest(releases=releases), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.assets(directory)
                def fake_gh(*args):
                    if "matching-refs" in args[-1]:
                        return json.dumps([{"ref": "refs/tags/v0.1.42", "object": {
                            "type": "commit", "sha": REVISION}}] if releases else [])
                    if args[0] == "api":
                        return json.dumps([releases])
                    if args[:2] == ("release", "download"):
                        shutil.copytree(directory, Path(args[args.index("--dir") + 1]), dirs_exist_ok=True)
                    return ""
                with patch.object(release, "gh", side_effect=fake_gh) as gh:
                    release.publish(directory, "0.1.42", REVISION, "test/repo")
                calls = [call.args for call in gh.call_args_list]
                self.assertEqual(any(call[:2] == ("release", "create") for call in calls), create)
                self.assertEqual(any(call[:2] == ("release", "upload") for call in calls), upload)
                edits = [call for call in calls if call[:2] == ("release", "edit")]
                self.assertEqual(len(edits), int(latest is not None))
                if latest is not None:
                    self.assertIn(f"--latest={latest}", edits[0])
                    self.assertLess(next(i for i, call in enumerate(calls) if call[:2] == ("release", "download")), calls.index(edits[0]))

    def test_missing_corrupt_wrong_commit_and_extra_assets_refuse_publication(self):
        for damage in ("missing", "corrupt", "revision", "extra"):
            with self.subTest(damage=damage), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.assets(directory)
                archive = next(directory.glob("*.tar.gz"))
                if damage == "missing":
                    archive.unlink()
                elif damage == "corrupt":
                    archive.write_bytes(b"corrupt")
                elif damage == "extra":
                    (directory / "unexpected").touch()
                with patch.object(release, "gh") as gh, self.assertRaises((ValueError, OSError)):
                    release.publish(directory, "0.1.42", "b" * 40 if damage == "revision" else REVISION, "test/repo")
                gh.assert_not_called()

    def test_download_failure_keeps_draft_unpublished(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.assets(directory)
            def fake_gh(*args):
                if "matching-refs" in args[-1]:
                    return "[]"
                if args[0] == "api":
                    return "[[]]"
                if args[:2] == ("release", "download"):
                    raise subprocess.CalledProcessError(1, ["gh"])
                return ""
            with patch.object(release, "gh", side_effect=fake_gh) as gh:
                with self.assertRaises(subprocess.CalledProcessError):
                    release.publish(directory, "0.1.42", REVISION, "test/repo")
            self.assertFalse(any(call.args[:2] == ("release", "edit") for call in gh.call_args_list))

    def test_conflicting_remote_tag_is_never_reused(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.assets(directory)
            refs = [{"ref": "refs/tags/v0.1.42", "object": {"type": "commit", "sha": "b" * 40}}]
            with patch.object(release, "gh", side_effect=["[[]]", json.dumps(refs)]) as gh:
                with self.assertRaisesRegex(ValueError, "already exists"):
                    release.publish(directory, "0.1.42", REVISION, "test/repo")
                self.assertTrue(all(call.args[0] == "api" for call in gh.call_args_list))


if __name__ == "__main__":
    unittest.main()
