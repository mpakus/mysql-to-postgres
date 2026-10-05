#!/usr/bin/env python3
"""Validate the reviewed fixture inventory against the read-only pinned checkout."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
UPSTREAM = ROOT.parent / "pgloader"
SNAPSHOT = ROOT / "tests/fixtures/upstream"
MANIFEST = ROOT / "tests/fixtures/upstream-manifest.json"


def selected_loads(root=SNAPSHOT):
    files = list((root / "test/mysql").glob("*.load"))
    files += list((root / "clojure/tests/mysql").glob("*/*.load"))
    files += list((root / "clojure/tests/mysql-unit-full").glob("*.load"))
    files += [root / "test" / name for name in ["mysql-collision.load", "sakila.load", "sakila-data.load"]]
    return sorted(str(path.relative_to(root)) for path in files)


def parser_cases(root=SNAPSHOT):
    cases = []
    for file in ["clojure/test/pgloader/load_file/parser_test.clj", "clojure/test/pgloader/load_file/ast_test.clj"]:
        text = (root / file).read_text()
        for match in re.finditer(r"\(deftest\s+([^\s()]+)(.*?)(?=\n\(deftest|\Z)", text, re.S):
            if "mysql://" in match[2]:
                cases.append((file, match[1], text[:match.start()].count("\n") + 1))
    return cases


def validate():
    manifest = json.loads(MANIFEST.read_text())
    if "--upstream" in sys.argv:
        head = subprocess.check_output(["git", "-C", str(UPSTREAM), "rev-parse", "HEAD"], text=True).strip()
        assert head == manifest["upstream_revision"], "upstream pin changed; coordinator review required"
        assert selected_loads(UPSTREAM) == selected_loads(), "new upstream fixture requires classification"
    assert selected_loads() == sorted(entry["path"] for entry in manifest["load_fixtures"]), "selected .load fixture missing or unexpected"
    assert sorted((file, name) for file, name, _ in parser_cases()) == sorted((e["path"], e["symbol"]) for e in manifest["parser_cases"]), "MySQL parser case missing"
    ids = set()
    for entry in manifest["load_fixtures"] + manifest["parser_cases"] + manifest["assertion_cases"] + manifest["supplemental_sql"]:
        assert entry["classification"] in {"ported", "intentional difference", "duplicate", "unsupported"}
        assert entry["rationale"] and entry["case_ids"] and entry["license"]
        assert hashlib.sha256((SNAPSHOT / entry["path"]).read_bytes()).hexdigest() == entry["sha256"], entry["path"]
        if "--upstream" in sys.argv:
            assert hashlib.sha256((UPSTREAM / entry["path"]).read_bytes()).hexdigest() == entry["sha256"], entry["path"]
        for case in entry["case_ids"]:
            ids.add(case)
    assert sorted(path.name for path in (SNAPSHOT / "clojure/tests/mysql/mytest/sql").glob("*.sql")) == sorted(Path(entry["path"]).name for entry in manifest["assertion_cases"])
    baseline = ROOT / "tests/fixtures/baselines/2026-10-01"
    for file, expected in json.loads((baseline / "artifact-hashes.json").read_text()).items():
        assert hashlib.sha256((baseline / file).read_bytes()).hexdigest() == expected, "baseline artifact changed: " + file
    print(f"fixture manifest: {len(manifest['load_fixtures'])} load files, {len(manifest['parser_cases'])} MySQL parser cases, {len(manifest['assertion_cases'])} assertion cases, {len(ids)} case IDs; pin and source hashes verified")


if __name__ == "__main__":
    validate()
