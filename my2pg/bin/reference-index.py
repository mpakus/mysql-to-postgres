#!/usr/bin/env python3
"""Rebuild the seven local XERJ source indexes; no database dump extraction."""
import argparse
import hashlib
import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "my2pg/docs/reference-repositories.json"
EXTENSIONS = {".rs", ".lisp", ".clj", ".cljs", ".edn", ".asd", ".load", ".sql",
              ".md", ".rst", ".txt", ".toml", ".yaml", ".yml", ".json", ".sh",
              ".py", ".c", ".h", ".ppl", ".out", ".csv", ".stderr", ".patch"}
NAMES = {"Makefile", "Dockerfile", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "COPYING"}
IGNORED = {"target", "build", "dist", "node_modules", "__pycache__", "vendor"}
SYMBOLS = re.compile(r"(?:\b(?:fn|struct|enum|trait|type|mod|const)\s+|"
                     r"\(\s*(?:defn-?|defun|defmethod|defclass|defmacro)\s+)([A-Za-z_][\w!?-]*)")


def request(endpoint, path, data=None, method="GET", content_type="application/json"):
    payload = data if isinstance(data, bytes) else json.dumps(data).encode() if data is not None else None
    req = Request(endpoint + path, data=payload, method=method,
                  headers={"Content-Type": content_type})
    with urlopen(req, timeout=120) as response:
        return json.load(response)


def documents(corpus):
    root = ROOT / corpus["path"]
    revision = None
    git_corpus = (root / ".git").exists()
    if git_corpus:
        revision = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
        if corpus["commit"] and revision != corpus["commit"]:
            raise RuntimeError(f"{root}: revision differs from manifest; review before updating the pin")
        paths = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z", "--cached",
                                         "--others", "--exclude-standard"]).decode().split("\0")
        paths = sorted(set(p for p in paths if p))
    else:
        paths = sorted(str(p.relative_to(root)) for p in root.rglob("*") if p.is_file())
    result, included, skipped, transformed = [], [], [], []
    for relative in paths:
        p = root / relative
        reason = None
        parts = Path(relative).parts
        if any(part.startswith(".") for part in parts) or (
                not git_corpus and any(part in IGNORED for part in parts)):
            reason = "hidden/build path"
        elif p.is_symlink() or not p.is_file():
            reason = "symlink/non-file"
        elif corpus["path"] == "pgloader" and relative.startswith("test/data/"):
            reason = "bulk fixture data (source/DDL tests elsewhere remain indexed)"
        elif p.suffix not in EXTENSIONS and p.name not in NAMES:
            reason = "outside source/document/test formats"
        if reason:
            skipped.append({"path": relative, "reason": reason})
            continue
        try:
            body = p.read_text(encoding="utf-8")
        except UnicodeError:
            skipped.append({"path": relative, "reason": "non-UTF-8"})
            continue
        if "\0" in body:
            body = body.replace("\0", "\\x00")
            transformed.append({"path": relative, "representation": "literal NUL displayed as \\x00"})
        included.append(relative)
        lines = body.splitlines(keepends=True) or [""]
        # Preserve source line numbers; intentional literal NULs get visible escapes.
        for start in range(0, len(lines), 60):
            code = "".join(lines[start:start + 60])
            result.append({"ax_path": relative, "title": relative, "name": p.name,
                           "body": code, "code": code,
                           "defs": "\n".join(SYMBOLS.findall(code)),
                           "start_line": start + 1, "end_line": min(start + 60, len(lines)),
                           "repository": corpus["path"], "revision": revision or "uncommitted"})
    return result, {"prefix": corpus["prefix"], "path": corpus["path"], "revision": revision,
                    "files": len(included), "passages": len(result),
                    "included": included, "skipped": skipped, "transformed": transformed}


def rebuild(endpoint, corpus):
    docs, evidence = documents(corpus)  # Validate/read before mutating the old index.
    if not docs:
        raise RuntimeError(f"{corpus['path']}: empty source corpus")
    index = corpus["prefix"] + "-source"
    try:
        request(endpoint, "/" + index, method="DELETE")
    except HTTPError as error:
        if error.code != 404:
            raise
    props = {field: {"type": "text"} for field in ("body", "code", "defs", "name", "title")}
    props.update({field: {"type": "keyword"} for field in ("ax_path", "repository", "revision")})
    props.update({field: {"type": "long"} for field in ("start_line", "end_line")})
    request(endpoint, "/" + index, {"mappings": {"properties": props}}, "PUT")
    for offset in range(0, len(docs), 100):
        batch = []
        for doc in docs[offset:offset + 100]:
            key = f"{doc['ax_path']}:{doc['start_line']}"
            batch.extend([json.dumps({"index": {"_index": index, "_id": hashlib.sha256(key.encode()).hexdigest()}}),
                          json.dumps(doc)])
        response = request(endpoint, "/_bulk", ("\n".join(batch) + "\n").encode(), "POST", "application/x-ndjson")
        if response.get("errors") or len(response.get("items", [])) != len(batch) // 2:
            raise RuntimeError(f"{index}: bulk refused/omitted documents: {response}")
    request(endpoint, "/" + index + "/_refresh", method="POST")
    actual = request(endpoint, "/" + index + "/_count")["count"]
    if actual != len(docs):
        raise RuntimeError(f"{index}: expected {len(docs)} passages, got {actual}")
    evidence["indexed_passages"] = actual
    print(f"{index}: {len(evidence['included'])} files, {actual} passages; {len(evidence['skipped'])} exclusions", flush=True)
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("prefix", nargs="?", help="One manifest prefix; omit to rebuild all")
    args = parser.parse_args()
    manifest = json.loads(MANIFEST.read_text())
    corpora = [c for c in manifest["corpora"] if not args.prefix or c["prefix"] == args.prefix]
    if not corpora:
        parser.error("unknown prefix; see docs/reference-repositories.json")
    report_dir = ROOT / ".reference-coding/reports"
    report_dir.mkdir(parents=True, exist_ok=True)
    for corpus in corpora:
        evidence = rebuild(manifest["endpoint"], corpus)
        evidence["checked_at"] = datetime.now(timezone.utc).isoformat()
        (report_dir / (corpus["prefix"] + "-coverage.json")).write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
