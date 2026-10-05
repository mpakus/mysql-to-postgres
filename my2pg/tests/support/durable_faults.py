#!/usr/bin/env python3
"""Native faults on one validated, owned 64 MiB image; never a host filler."""
import argparse
import errno
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
LIMIT = 64 * 1024 * 1024


def command(args):
    result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=45)
    if result.returncode:
        raise RuntimeError(f"{args[0:2]} failed ({result.returncode}): {result.stderr.decode()}")
    return result.stdout


def save(info):
    Path(info["root"]).joinpath("ownership.json").write_text(json.dumps(info, indent=2) + "\n")


def load(root):
    root = Path(root).resolve()
    if root.parent != ROOT.joinpath("target").resolve() or not re.fullmatch(r"t12-durable-[0-9a-f]{32}", root.name):
        raise ValueError("not an owned durability-fault root")
    info = json.loads(root.joinpath("ownership.json").read_text())
    for key, value in [("root", root), ("image", root / "volume.dmg"), ("mount", root / "volume")]:
        if info[key] != str(value) or value.is_symlink():
            raise ValueError("ownership path mismatch or symlink")
    return info


def verify_attached(info):
    images = plistlib.loads(command(["hdiutil", "info", "-plist"]))["images"]
    for image in images:
        if Path(image.get("image-path", "")).resolve() != Path(info["image"]):
            continue
        for entity in image.get("system-entities", []):
            if entity.get("mount-point") == info["mount"] and entity.get("dev-entry") == info["device"]:
                if os.stat(info["mount"]).st_dev == os.stat(info["root"]).st_dev:
                    raise ValueError("mountpoint is on host filesystem")
                return
    raise ValueError("owned image/device/mount identity is not attached")


def attach(info):
    result = plistlib.loads(command([
        "hdiutil", "attach", info["image"], "-mountpoint", info["mount"],
        "-nobrowse", "-noautoopen", "-plist",
    ]))
    entities = [entry for entry in result["system-entities"] if entry.get("mount-point") == info["mount"]]
    if len(entities) != 1:
        raise RuntimeError("image did not mount at its unique owned mountpoint")
    info["device"] = entities[0]["dev-entry"]
    info["attached"] = True
    save(info)
    verify_attached(info)
    return info


def create():
    if sys.platform != "darwin":
        raise RuntimeError("native disk-image lane requires macOS; do not count a skip")
    root = ROOT / "target" / f"t12-durable-{uuid.uuid4().hex}"
    root.mkdir(mode=0o700)
    mount = root / "volume"
    mount.mkdir(mode=0o700)
    info = {"root": str(root), "image": str(root / "volume.dmg"), "mount": str(mount), "attached": False}
    save(info)
    command(["hdiutil", "create", "-size", "64m", "-fs", "HFS+", "-type", "UDIF",
             "-volname", "MY2PG_T12_FAULT", info["image"]])
    return attach(info)


def detach(info, force=False):
    verify_attached(info)
    args = ["hdiutil", "detach", info["mount"]]
    if force:
        args.append("-force")
    command(args)
    info["attached"] = False
    save(info)
    return info


def fill(info):
    verify_attached(info)
    path = Path(info["mount"]) / "filler.bin"
    allocated = 0
    with path.open("xb", buffering=0) as filler:
        block = b"x" * (1024 * 1024)
        while allocated < LIMIT:
            try:
                allocated += os.write(filler.fileno(), block[:min(len(block), LIMIT - allocated)])
            except OSError as error:
                if error.errno != errno.ENOSPC:
                    raise
                if len(block) > 1:
                    block = block[:max(1, len(block) // 16)]
                    continue
                return {"errno": error.errno, "kind": "ENOSPC", "allocated": allocated,
                        "failed_write_bytes": 1, "root": info["root"]}
        raise RuntimeError("fixed-image fill reached limit without physical ENOSPC")


def probe():
    info = create()
    result = {"ownership": info.copy()}
    try:
        result["physical_fill"] = fill(info)
        Path(info["mount"]).joinpath("filler.bin").unlink()
        with Path(info["mount"]).joinpath("probe.bin").open("xb", buffering=0) as file:
            file.write(b"native sync probe\n")
            os.fsync(file.fileno())
            detach(info, force=True)
            try:
                os.fsync(file.fileno())
                result["detached_fsync"] = {"result": "success", "errno": 0}
            except OSError as error:
                result["detached_fsync"] = {"result": "failure", "errno": error.errno, "message": error.strerror}
        attach(info)
        result["reattached_bytes"] = Path(info["mount"]).joinpath("probe.bin").read_text()
    finally:
        if info["attached"]:
            detach(info)
        result["final_ownership"] = info
        Path(info["root"]).joinpath("probe-result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["create", "attach", "detach", "force-detach", "fill", "free", "probe"])
    parser.add_argument("root", nargs="?")
    args = parser.parse_args()
    if args.operation == "probe":
        result = probe()
    elif args.operation == "create":
        result = create()
    else:
        if not args.root:
            parser.error("owned root is required")
        info = load(args.root)
        if args.operation == "attach":
            result = attach(info)
        elif args.operation in ["detach", "force-detach"]:
            result = detach(info, args.operation == "force-detach")
        elif args.operation == "fill":
            result = fill(info)
        else:
            verify_attached(info)
            Path(info["mount"]).joinpath("filler.bin").unlink()
            result = info
    print(json.dumps(result))


if __name__ == "__main__":
    main()
