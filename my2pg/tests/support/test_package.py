import re
import subprocess
import unittest
from pathlib import Path, PurePosixPath
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[2]


def package_paths():
    result = subprocess.run(
        [str(ROOT / "bin/cargo"), "package", "--list", "--allow-dirty", "--no-verify", "--locked", "--offline"],
        cwd=ROOT,
        check=True,
        text=True,
        capture_output=True,
    )
    return {line.strip().removeprefix("./") for line in result.stdout.splitlines() if line.strip()}


class PackageBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.paths = package_paths()

    def test_archive_contains_runtime_tests_and_fixture_provenance(self):
        required = {
            "Cargo.toml",
            "Dockerfile",
            ".dockerignore",
            "README.md",
            "LICENSE",
            "rust-toolchain.toml",
            "src/main.rs",
            "tests/integration.rs",
            "tests/support/test_package.py",
            "tests/fixtures/README.md",
            "tests/fixtures/upstream-manifest.json",
            "tests/fixtures/upstream/LICENSE",
            "tests/fixtures/upstream/DATASET-NOTICES.md",
            "docs/architecture.md",
            "docs/config-and-cli.md",
            "docs/scope-and-compatibility.md",
            "docs/examples/mysql-to-postgres.toml",
        }
        self.assertTrue(required <= self.paths, f"missing package paths: {sorted(required - self.paths)}")

    def test_archive_excludes_internal_and_generated_material(self):
        forbidden = {
            "AGENTS.md",
            "docs/implementation-plan.md",
            "docs/agent-tasks.md",
            "docs/checklists.md",
            "docs/reference-coding.md",
            "docs/README.md",
        }
        leaked = {path for path in self.paths if path.startswith("worklog/") or path.startswith("tests/fixtures/baselines/")}
        self.assertFalse(forbidden & self.paths, f"internal paths leaked: {sorted(forbidden & self.paths)}")
        self.assertFalse(leaked, f"historical or generated paths leaked: {sorted(leaked)}")

    def test_shipped_markdown_local_links_resolve_in_archive(self):
        markdown = {path for path in self.paths if path.lower().endswith(".md")}
        missing = []
        link_pattern = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")
        for source in sorted(markdown):
            source_path = ROOT / source
            content = source_path.read_text(encoding="utf-8")
            for raw in link_pattern.findall(content):
                target = raw.strip().split(maxsplit=1)[0].strip("<>")
                if not target or target.startswith(("https://", "http://", "mailto:", "#")):
                    continue
                path_part = unquote(target.split("#", 1)[0])
                if not path_part:
                    continue
                resolved = PurePosixPath(source).parent.joinpath(path_part)
                normalized = str(resolved)
                stack = []
                for segment in normalized.split("/"):
                    if segment == "..":
                        if stack:
                            stack.pop()
                    elif segment not in ("", "."):
                        stack.append(segment)
                normalized = "/".join(stack)
                if normalized not in self.paths:
                    missing.append(f"{source}: {target} -> {normalized}")
        self.assertEqual(missing, [], "broken or unpublished local Markdown links:\n" + "\n".join(missing))


if __name__ == "__main__":
    unittest.main()
