#!/usr/bin/env python3
"""Search pinned local XERJ sources using its verified REST query path."""
import argparse
import json
from pathlib import Path
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("prefix", help="Manifest prefix, ref (all references), or project")
parser.add_argument("query")
parser.add_argument("--symbol", action="store_true", help="Match a heuristic definition name")
parser.add_argument("-k", type=int, default=5)
parser.add_argument("--full", type=int, default=2000)
parser.add_argument("--json", action="store_true")
args = parser.parse_args()
manifest = json.loads((ROOT / "my2pg/docs/reference-repositories.json").read_text())
valid = {c["prefix"] for c in manifest["corpora"]} | {"ref", "project"}
if args.prefix not in valid or not 1 <= args.k <= 50 or args.full < 0:
    parser.error("use a listed prefix, 1..50 results, and nonnegative --full")
query = {"match_phrase": {"defs": args.query}} if args.symbol else {
    "multi_match": {"query": args.query, "fields": ["code", "defs", "title"]}}
payload = {"query": query, "size": args.k,
           "_source": ["ax_path", "code", "start_line", "end_line", "repository", "revision"]}
req = Request(manifest["endpoint"] + "/" + args.prefix + "-*/_search",
              data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"})
with urlopen(req, timeout=30) as response:
    result = json.load(response)
if args.json:
    print(json.dumps(result, indent=2))
else:
    hits = result["hits"]["hits"]
    for hit in hits:
        source = hit["_source"]
        path = ROOT / source["repository"] / source["ax_path"]
        print(f"\n{path}:{source['start_line']} (score {hit['_score']:.2f}, revision {source['revision']})")
        print(source["code"][:args.full])
    print(f"\n{len(hits)} result(s). Read the original file and adjacent tests before relying on a passage.")
