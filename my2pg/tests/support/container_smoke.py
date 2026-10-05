#!/usr/bin/env python3
"""Build and exercise the release image against an owned MySQL/PostgreSQL pair."""
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/support"))
import harness  # noqa: E402


CONFIG = '''version = 1
[source]
url_env = "MY2PG_MYSQL_URL"
consistency = "frozen"
tls_mode = "verify_full"
ca_file = "/tls/ca.pem"
[target]
url_env = "MY2PG_POSTGRES_URL"
schema = "container_smoke"
tls_mode = "verify_full"
ca_file = "/tls/ca.pem"
on_existing = "error"
[migration]
mode = "full"
identifiers = "preserve"
table_workers = 1
readers_per_table = 1
index_workers = 1
batch_rows = 128
batch_bytes = 65536
queue_batches = 2
memory_bytes = 1048576
max_row_bytes = 65536
on_row_error = "stop"
max_rejected_rows = 0
reset_sequences = true
[tables]
include = ["users"]
exclude = []
[verification]
mode = "counts_and_schema"
[report]
directory = "/artifacts"
console = "json"
progress = "never"
'''


def run(args, *, check=True):
    return subprocess.run(args, cwd=ROOT, check=check, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def assert_equal(actual, expected, label):
    if actual != expected:
        raise RuntimeError(f"{label}: expected {expected!r}, got {actual!r}")


def source_state():
    revision = run(["git", "rev-parse", "HEAD"]).stdout.strip()
    dirty = bool(run(["git", "status", "--porcelain=v1", "--untracked-files=all"]).stdout)
    return revision, dirty


def migration_volumes(config, ca_file, report_dir):
    return [
        "--volume", f"{config}:/migration.toml:ro",
        "--volume", f"{ca_file}:/tls/ca.pem:ro",
        "--volume", f"{report_dir}:/artifacts:rw",
    ]


def cleanup(metadata, image, report, artifact):
    errors = []
    if metadata is not None and metadata.get("state") != "stopped":
        try:
            harness.stop(metadata)
        except Exception as error:  # Preserve image cleanup and the result artifact.
            errors.append(f"fixture cleanup failed: {error}")
    try:
        removed = run(["docker", "image", "rm", "--force", image], check=False)
        if removed.returncode:
            errors.append(f"image cleanup failed: {removed.stderr.strip()}")
    except Exception as error:  # Preserve the result artifact if Docker itself is unavailable.
        errors.append(f"image cleanup failed: {error}")
    if errors:
        report["status"] = "failed"
        report["cleanup_errors"] = errors
    if report["status"] == "starting":
        report["status"] = "failed"
    (artifact / "result.json").write_text(json.dumps(report, indent=2) + "\n")


def main():
    run_id = uuid.uuid4().hex[:12]
    image = f"my2pg:container-smoke-{run_id}"
    artifact = ROOT / "target" / "container-smoke" / run_id
    artifact.mkdir(parents=True, mode=0o700)
    metadata = None
    revision, dirty = source_state()
    report = {"image": image, "status": "starting"}
    try:
        build = run(["docker", "build", "--pull", "--tag", image,
                     "--build-arg", f"VCS_REF={revision}",
                     "--file", str(ROOT / "Dockerfile"), str(ROOT)], check=False)
        (artifact / "build.stdout.log").write_text(build.stdout)
        (artifact / "build.stderr.log").write_text(build.stderr)
        if build.returncode:
            raise RuntimeError(f"container image build failed with exit {build.returncode}; see {artifact}")

        image_info = run(["docker", "image", "inspect", "--format",
                          "{{.Id}} {{.Os}} {{.Architecture}} {{.Config.User}}", image]).stdout.strip().split()
        if len(image_info) != 4:
            raise RuntimeError(f"unexpected image inspection result: {image_info!r}")
        image_id, operating_system, architecture, default_user = image_info
        assert_equal(operating_system, "linux", "runtime operating system")
        assert_equal(default_user, "10001:10001", "default runtime user")

        version = run(["docker", "run", "--rm", image, "--version"]).stdout.strip()
        help_text = run(["docker", "run", "--rm", image, "--help"]).stdout
        if "mysql" not in help_text.lower() or "postgresql" not in help_text.lower():
            raise RuntimeError("image CLI help does not expose the supported MySQL/PostgreSQL direction")
        tools = run(["docker", "run", "--rm", "--entrypoint", "/bin/sh", image,
                     "-c", "for tool in java sbcl clojure; do if command -v \"$tool\" >/dev/null 2>&1; then exit 1; fi; done"])

        metadata = harness.start("mysql84", "pg16")
        work = artifact / "migration"
        work.mkdir(mode=0o777)
        work.chmod(0o777)
        config = work / "migration.toml"
        config.write_text(CONFIG)
        report_dir = work / "report"
        report_dir.mkdir(mode=0o777)
        report_dir.chmod(0o777)
        network = f"{metadata['project']}_default"
        with tempfile.NamedTemporaryFile(mode="w", prefix="my2pg-container-env-", delete=False) as env_file:
            env_path = Path(env_file.name)
            env_file.write("MY2PG_MYSQL_URL=mysql://my2pg:integration-only@mysql:3306/source\n")
            env_file.write("MY2PG_POSTGRES_URL=postgresql://my2pg:integration-only@postgres:5432/target\n")
            env_file.write("MY2PG_ARTIFACT_DIR=/artifacts\n")
        env_path.chmod(0o600)
        try:
            result = run([
                "docker", "run", "--rm", "--network", network,
                "--env-file", str(env_path),
                *migration_volumes(config, Path(metadata["artifact_dir"]) / "tls" / "ca.pem", report_dir),
                image, "run", "/migration.toml",
            ], check=False)
        finally:
            env_path.unlink(missing_ok=True)
        (artifact / "migration.stdout.log").write_text(result.stdout)
        (artifact / "migration.stderr.log").write_text(result.stderr)
        if result.returncode:
            raise RuntimeError(f"container migration exited {result.returncode}; see {artifact}")

        report_files = list(report_dir.glob("*/report.json"))
        if len(report_files) != 1:
            raise RuntimeError(f"expected one durable run report, found {len(report_files)}")
        run_report = json.loads(report_files[0].read_text())
        assert_equal(run_report.get("status"), "complete", "run report status")
        assert_equal(run_report.get("verification", {}).get("status"), "complete", "verification status")
        assert_equal(harness.sql(metadata, "postgres", "SELECT COUNT(*) FROM container_smoke.users"), "3", "migrated row count")
        assert_equal(harness.sql(metadata, "postgres", "SELECT encode(payload, 'hex') FROM container_smoke.users WHERE id=1"), "00015c09ff", "binary value")
        assert_equal(harness.sql(metadata, "postgres", "SELECT amount::text FROM container_smoke.users WHERE id=1"), "1234567890123456789012.12345678", "exact decimal")
        assert_equal(harness.sql(metadata, "postgres", "SELECT name FROM container_smoke.users WHERE id=4"), "emoji 😀", "UTF-8 value")
        binary_hash = run(["docker", "run", "--rm", "--entrypoint", "/usr/bin/sha256sum", image,
                           "/usr/local/bin/my2pg"]).stdout.split()[0]
        report.update({
            "status": "passed",
            "source_revision": revision,
            "dirty_source": dirty,
            "source_version": metadata["versions"]["mysql"],
            "target_version": metadata["versions"]["postgres"],
            "architecture": architecture,
            "image_id": image_id,
            "default_user": default_user,
            "binary_version": version,
            "binary_sha256": binary_hash,
            "verification": run_report["verification"],
            "rows_committed": sum(table["committed_rows"] for table in run_report["tables"]),
            "fixture_project": metadata["project"],
            "report_sha256": hashlib.sha256(report_files[0].read_bytes()).hexdigest(),
        })
    finally:
        cleanup(metadata, image, report, artifact)
    print(artifact / "result.json")
    if report["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
