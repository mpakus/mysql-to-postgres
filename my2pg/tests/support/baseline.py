#!/usr/bin/env python3
"""Run an audited pgloader build on one owned fixture; never call this a benchmark."""
import argparse
import hashlib
import json
from pathlib import Path
import time
import harness


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", choices=["v3", "v4"])
    parser.add_argument("connections", type=Path)
    args = parser.parse_args()
    metadata = json.loads(args.connections.read_text())
    harness.validate_owned(metadata)
    if metadata["state"] != "ready":
        raise ValueError("baseline needs a currently ready owned fixture")
    revision = "231ab86778ca5ffd7de40878714760c8b4860cdf"
    head = harness.command(["git", "-C", str(harness.ROOT.parent / "pgloader"), "rev-parse", "HEAD"]).stdout.strip()
    if head != revision:
        raise ValueError("pgloader revision differs from audited baseline")
    image = f"my2pg-reference-{args.version}:231ab867"
    image_info = json.loads(harness.command(["docker", "image", "inspect", image]).stdout)[0]
    artifact = Path(metadata["artifact_dir"])
    schema = f"baseline_{args.version}"
    common = ["docker", "run", "--rm", "--label", "org.my2pg.integration=true", "--network", metadata["project"] + "_default", "-v", str(artifact) + ":/suite:ro"]
    if args.version == "v4":
        # Import only our generated CA into a disposable JVM truststore.
        harness.command(["docker", "run", "--rm", "--label", "org.my2pg.integration=true", "--user", "0", "--entrypoint", "keytool", "-v", str(artifact) + ":/suite", image,
                         "-importcert", "-noprompt", "-alias", "my2pg-ca", "-file", "/suite/tls/ca.pem", "-keystore", "/suite/tls/truststore.p12", "-storetype", "PKCS12", "-storepass", "integration-only"])
        source_params = "sslMode=VERIFY_CA&trustCertificateKeyStoreUrl=file:/suite/tls/truststore.p12&trustCertificateKeyStorePassword=integration-only&trustCertificateKeyStoreType=PKCS12"
    else:
        # Preserve the server's modern auth and request encryption using the
        # syntax the Lisp parser supports; do not infer capability from old docs.
        source_params = "useSSL=true"
    target_params = "sslmode=require" if args.version == "v3" else "sslmode=verify-ca&sslrootcert=/suite/tls/ca.pem"
    load = f"""LOAD DATABASE
 FROM mysql://my2pg:integration-only@mysql:3306/source?{source_params}
 INTO postgresql://my2pg:integration-only@postgres:5432/target?{target_params}
 WITH concurrency = 1, workers = 1, include no drop, create tables, create indexes, reset sequences, quote identifiers
 INCLUDING ONLY TABLE NAMES MATCHING 'users'
 ALTER SCHEMA 'source' RENAME TO '{schema}'
 CAST type datetime to timestamp;
"""
    file = artifact / f"baseline-{args.version}.load"
    file.write_text(load)
    if args.version == "v3":
        command = common + ["-e", "SSL_CERT_FILE=/suite/tls/ca.pem", image, "/opt/src/pgloader/build/bin/pgloader", "/suite/" + file.name]
    else:
        command = common + [image, "/suite/" + file.name]
    started = time.monotonic()
    result = harness.command(command, check=False)
    elapsed = time.monotonic() - started
    (artifact / f"baseline-{args.version}.log").write_text(result.stdout + result.stderr)
    report = {"upstream_revision": revision, "loader": args.version, "image_id": image_info["Id"], "architecture": image_info["Architecture"], "source_target": metadata["versions"],
              "fixture_sha256": metadata["fixture_sha256"], "load_sha256": hashlib.sha256(file.read_bytes()).hexdigest(), "exit_code": result.returncode, "elapsed_seconds_informational": elapsed,
              "benchmark": False, "correctness": "not established", "command": command}
    if result.returncode == 0:
        checks = [
            (f'SELECT count(*) FROM "{schema}".users', "3"),
            (f'SELECT amount::text FROM "{schema}".users WHERE id=1', "1234567890123456789012.12345678"),
            (f"SELECT encode(payload, 'hex') FROM \"{schema}\".users WHERE id=1", "00015c09ff"),
            (f'SELECT note IS NULL FROM "{schema}".users WHERE id=1', "t"),
            (f'SELECT note = \'\' FROM "{schema}".users WHERE id=4', "t"),
            ("SELECT sentinel FROM public.users WHERE id=999", "must-survive"),
        ]
        report["checks"] = [{"query": query, "expected": expected, "actual": harness.sql(metadata, "postgres", query)} for query, expected in checks]
        report["correctness"] = "six checks passed" if all(c["expected"] == c["actual"] for c in report["checks"]) else "failed"
    path = artifact / f"baseline-{args.version}.json"
    path.write_text(json.dumps(report, indent=2) + "\n")
    print(path)
    print(f"{args.version}: exit {result.returncode}; correctness {report['correctness']}")


if __name__ == "__main__":
    main()
