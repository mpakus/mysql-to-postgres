import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("release_build", ROOT / "bin/build-release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseBuildTests(unittest.TestCase):
    def test_target_selection_refuses_wrong_native_hosts_and_foreign_targets(self):
        self.assertEqual(release.select_target("macos", None, "macos", "arm64", False),
                         "aarch64-apple-darwin")
        self.assertEqual(release.select_target("linux", None, "macos", "arm64", True),
                         "aarch64-unknown-linux-gnu")
        self.assertEqual(release.select_target("windows", None, "windows", "AMD64", False),
                         "x86_64-pc-windows-msvc")
        for values in [("macos", None, "linux", "x86_64", False),
                       ("windows", None, "macos", "arm64", False),
                       ("linux", "x86_64-pc-windows-msvc", "linux", "x86_64", False),
                       ("macos", None, "macos", "arm64", True)]:
            with self.subTest(values=values), self.assertRaises(ValueError):
                release.select_target(*values)

    def test_cargo_build_selects_production_artifact_and_propagates_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "my2pg"
            binary.write_bytes(b"executable")
            artifact = {"reason": "compiler-artifact", "target": {"name": "my2pg", "kind": ["bin"]},
                        "manifest_path": str(ROOT / "Cargo.toml"), "executable": str(binary)}
            worker = dict(artifact, target={"name": "my2pg-artifact-test-worker", "kind": ["bin"]})
            with patch.object(release, "run", return_value=json.dumps(worker) + "\n" + json.dumps(artifact)) as run:
                self.assertEqual(release.build_native("aarch64-apple-darwin", True), binary)
                command = run.call_args.args[0]
                self.assertIn("--locked", command)
                self.assertIn("--release", command)
                self.assertIn("--offline", command)
                self.assertEqual(command[command.index("--bin") + 1], "my2pg")
            with patch.object(release, "run", side_effect=subprocess.CalledProcessError(1, ["cargo"])):
                with self.assertRaises(subprocess.CalledProcessError):
                    release.build_native("aarch64-apple-darwin", False)
            with patch.object(release, "run", return_value=json.dumps(worker)):
                with self.assertRaises(ValueError):
                    release.build_native("aarch64-apple-darwin", False)

    def test_failed_build_preserves_previous_archive_and_never_packages(self):
        with tempfile.TemporaryDirectory() as temporary:
            previous = Path(temporary) / "previous.tar.gz"
            previous.write_bytes(b"previous release")
            with patch.object(release.platform, "system", return_value="Darwin"), \
                    patch.object(release.platform, "machine", return_value="arm64"), \
                    patch.object(release, "build_native", side_effect=ValueError("fixture compilation failed")), \
                    patch.object(release, "package") as package:
                self.assertEqual(release.main(["--platform", "macos", "--out-dir", temporary]), 1)
                package.assert_not_called()
            self.assertEqual(previous.read_bytes(), b"previous release")
            self.assertEqual(list(Path(temporary).iterdir()), [previous])

    def test_unix_and_experimental_windows_archives_have_verified_contents(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            binary = work / "binary"
            binary.write_bytes(b"only the production executable")
            for target in ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"]:
                with self.subTest(target=target), patch.object(release, "source_info", return_value={"revision": "test", "dirty": True}):
                    archive = release.package(binary, target, work / "output", "cargo")
                    sidecar = archive.with_name(archive.name + ".sha256").read_text()
                    self.assertEqual(sidecar, f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
                    if archive.suffix == ".zip":
                        with zipfile.ZipFile(archive) as bundle:
                            contents = {name.split("/", 1)[1]: bundle.read(name)
                                        for name in bundle.namelist() if not name.endswith("/")}
                    else:
                        with tarfile.open(archive) as bundle:
                            contents = {member.name.split("/", 1)[1]: bundle.extractfile(member).read()
                                        for member in bundle.getmembers() if member.isfile()}
                    executable = "my2pg.exe" if "windows" in target else "my2pg"
                    self.assertEqual(contents[executable], binary.read_bytes())
                    self.assertTrue(set(release.BUNDLE_FILES) <= contents.keys())
                    self.assertFalse(any(name.startswith(("src/", "tests/", "worklog/")) for name in contents))
                    info = json.loads(contents["BUILD-INFO.json"])
                    self.assertEqual(info["source"], {"revision": "test", "dirty": True})
                    self.assertEqual(info["target"], target)
                    self.assertEqual(info["binary_sha256"], hashlib.sha256(binary.read_bytes()).hexdigest())
                    if "windows" in target:
                        self.assertIn("Experimental Windows", contents["EXPERIMENTAL-WINDOWS.txt"].decode())
                        self.assertIn("platform_limitation", info)
                    checked = set()
                    for line in contents["SHA256SUMS"].decode().splitlines():
                        expected, name = line.split("  ", 1)
                        self.assertEqual(hashlib.sha256(contents[name]).hexdigest(), expected)
                        checked.add(name)
                    self.assertEqual(checked, contents.keys() - {"SHA256SUMS"})


if __name__ == "__main__":
    unittest.main()
