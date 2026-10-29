#!/usr/bin/env python3
"""Stamp CI versions and publish verified macOS/Linux development builds."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile


ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("x86_64-apple-darwin", "aarch64-apple-darwin",
           "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")
VERSION = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"


def stamp(run_number, root=ROOT):
    if not re.fullmatch(r"[1-9][0-9]*", run_number):
        raise ValueError("run number must be a positive integer")
    manifest = root / "Cargo.toml"
    lock = root / "Cargo.lock"
    text = manifest.read_text(encoding="utf-8")
    pattern = r'(?m)(^version = ")(' + VERSION + r')("$)'
    current = re.search(pattern, text.split("[dependencies]", 1)[0])
    if current is None:
        raise ValueError("Cargo.toml needs an explicit stable package version")
    major, minor, _ = current.group(2).split(".")
    version = f"{major}.{minor}.{run_number}"
    updated_manifest = text[:current.start(2)] + version + text[current.end(2):]
    updated_lock, count = re.subn(
        r'(?m)(^name = "my2pg"\nversion = ")[^"]+("$)',
        lambda match: match.group(1) + version + match.group(2),
        lock.read_text(encoding="utf-8"),
    )
    if count != 1:
        raise ValueError("Cargo.lock must contain exactly one my2pg package")
    manifest.write_text(updated_manifest, encoding="utf-8")
    lock.write_text(updated_lock, encoding="utf-8")
    return version


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True)


def digest(path):
    with path.open("rb") as source:
        checksum = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def verify_assets(directory, version, revision):
    expected = set()
    for target in TARGETS:
        name = f"my2pg-{version}-{target}"
        archive = directory / f"{name}.tar.gz"
        sidecar = directory / f"{name}.tar.gz.sha256"
        expected.update((archive.name, sidecar.name))
        if sidecar.read_text(encoding="utf-8") != f"{digest(archive)}  {archive.name}\n":
            raise ValueError(f"checksum mismatch: {archive.name}")
        with tarfile.open(archive) as bundle:
            info = json.load(bundle.extractfile(f"{name}/BUILD-INFO.json"))
        if (info["version"], info["target"], info["source"]["revision"]) != (version, target, revision):
            raise ValueError(f"build identity mismatch: {archive.name}")
    if {path.name for path in directory.iterdir()} != expected:
        raise ValueError("release must contain exactly four archives and their checksums")
    return sorted(directory / name for name in expected)


def publish(directory, version, revision, repo):
    if not re.fullmatch(VERSION, version) or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("expected a stable version and full source commit SHA")
    files = verify_assets(directory, version, revision)
    pages = json.loads(gh("api", "--paginate", "--slurp", f"repos/{repo}/releases"))
    releases = [release for page in pages for release in page]
    tag = f"v{version}"
    existing = next((release for release in releases if release["tag_name"] == tag), None)
    if existing and existing["target_commitish"] != revision:
        raise ValueError(f"{tag} already belongs to another commit")
    refs = json.loads(gh("api", f"repos/{repo}/git/matching-refs/tags/{tag}"))
    for ref in refs:
        target = ref["object"]
        if ref["ref"] == f"refs/tags/{tag}" and (target["type"] != "commit" or target["sha"] != revision):
            raise ValueError(f"{tag} already exists and is not this build's lightweight commit tag")
    published = existing is not None and not existing["draft"]
    with tempfile.TemporaryDirectory(prefix="my2pg-publish-") as temporary:
        work = Path(temporary)
        if existing is None:
            notes = work / "notes.md"
            notes.write_text(
                f"Automatic development build from `{revision}`.\n\n"
                "Includes native macOS and Linux binaries for Intel/AMD64 and ARM64, "
                "documentation, license notices and SHA-256 checksums.\n\n"
                "Linux requires glibc 2.39+ and OpenSSL 3. Builds are unsigned; "
                "macOS builds are not notarized. Build and CLI checks do not certify "
                "the complete database migration compatibility/recovery matrix.\n",
                encoding="utf-8",
            )
            gh("release", "create", tag, "--repo", repo, "--draft", "--target", revision,
               "--title", f"my2pg {tag}", "--notes-file", str(notes))
        if not published:
            gh("release", "upload", tag, "--repo", repo, "--clobber", *map(str, files))
        downloaded = work / "downloaded"
        downloaded.mkdir()
        gh("release", "download", tag, "--repo", repo, "--dir", str(downloaded))
        verify_assets(downloaded, version, revision)
        if not published:
            for path in files:
                if digest(path) != digest(downloaded / path.name):
                    raise ValueError(f"download differs from uploaded asset: {path.name}")
            newer = any(
                not release["draft"] and not release["prerelease"]
                and re.fullmatch("v" + VERSION, release["tag_name"])
                and tuple(map(int, release["tag_name"][1:].split("."))) > tuple(map(int, version.split(".")))
                for release in releases
            )
            gh("release", "edit", tag, "--repo", repo, "--draft=false",
               f"--latest={'false' if newer else 'true'}")
    print(f"https://github.com/{repo}/releases/tag/{tag}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("stamp").add_argument("run_number")
    release = commands.add_parser("publish")
    release.add_argument("version")
    release.add_argument("revision")
    release.add_argument("directory", type=Path)
    args = parser.parse_args()
    if args.command == "stamp":
        print(stamp(args.run_number))
    else:
        publish(args.directory, args.version, args.revision, os.environ["GH_REPO"])


if __name__ == "__main__":
    main()
