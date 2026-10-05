import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
FORBIDDEN_DRIVER_CRATES = {
    "csv",
    "duckdb",
    "libsqlite3-sys",
    "mongodb",
    "odbc-api",
    "oracle",
    "rusqlite",
    "sqlx",
    "surrealdb",
    "tiberius",
}
FORBIDDEN_SOURCE_MARKERS = (
    b"sqlite://",
    b"mssql://",
    b"jdbc:",
    b"csv://",
    b"copy://",
    b"fixed://",
    b"dbf://",
    b"ixf://",
    b"redshift://",
    b"s3://",
    b"http://",
)


def run(*args):
    return subprocess.run(
        args,
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )


class SourceScopeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.install = tempfile.TemporaryDirectory(prefix="my2pg-source-scope-")
        cls.binary = Path(cls.install.name) / "bin" / "my2pg"
        cls.addClassCleanup(cls.install.cleanup)
        run(
            str(ROOT / "bin/cargo"),
            "install",
            "--path",
            ".",
            "--root",
            cls.install.name,
            "--offline",
            "--locked",
            "--force",
        )

    def test_runtime_graph_has_expected_transports_and_no_known_alternate_drivers(self):
        result = run(
            str(ROOT / "bin/cargo"),
            "tree",
            "--offline",
            "--locked",
            "--edges",
            "normal",
            "--target",
            "all",
            "--prefix",
            "none",
        )
        packages = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
        self.assertTrue({"mysql_async", "tokio-postgres"} <= packages)
        self.assertFalse(FORBIDDEN_DRIVER_CRATES & packages)

    def test_installed_release_has_no_alternate_source_markers_or_help(self):
        binary = self.binary.read_bytes().lower()
        embedded = [marker.decode() for marker in FORBIDDEN_SOURCE_MARKERS if marker in binary]
        self.assertEqual(embedded, [], f"alternate source markers found in installed binary: {embedded}")

        result = subprocess.run(
            [str(self.binary), "--help"],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        help_text = result.stdout.lower()
        self.assertIn("mysql", help_text)
        self.assertIn("postgresql", help_text)
        self.assertFalse(
            any(marker.decode().split(":")[0] in help_text for marker in FORBIDDEN_SOURCE_MARKERS),
            help_text,
        )


if __name__ == "__main__":
    unittest.main()
