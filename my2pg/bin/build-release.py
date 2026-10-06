#!/usr/bin/env python3
"""Build one production target and bundle its operator docs and license notices."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "linux": ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"),
    "macos": ("x86_64-apple-darwin", "aarch64-apple-darwin"),
    "windows": ("x86_64-pc-windows-msvc",),
}
HOSTS = {"Linux": "linux", "Darwin": "macos", "Windows": "windows"}
WINDOWS_LIMIT = (
    "Experimental Windows build: migration durability/private-file guarantees are "
    "not supported or tested on Windows. Use Linux (including WSL2) or macOS for migrations."
)
BUNDLE_FILES = (
    "README.md", "LICENSE", "THIRD-PARTY-NOTICES.md",
    "docs/architecture.md", "docs/config-and-cli.md",
    "docs/scope-and-compatibility.md", "docs/releases.md",
    "docs/examples/mysql-to-postgres.toml", "docs/examples/pgloader-subset.load",
)


def select_target(family, target, host, machine, docker):
    if docker and family != "linux":
        raise ValueError("--docker builds Linux only")
    if not docker and family != host:
        raise ValueError(f"{family} requires a native {family} host; Linux can also use --docker")
    if target is None:
        arch = {"x86_64": "x86_64", "amd64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(machine.lower())
        target = next((item for item in TARGETS[family] if item.startswith(f"{arch}-")), None)
    if target not in TARGETS[family]:
        raise ValueError(f"choose a {family} target: {', '.join(TARGETS[family])}")
    return target


def run(command, capture=False, env=None):
    return subprocess.run(command, cwd=ROOT, check=True, text=True, env=env,
                          stdout=subprocess.PIPE if capture else None).stdout


def build_native(target, offline):
    env = os.environ.copy()
    cargo = str(ROOT / "bin/cargo")
    if os.name == "nt":
        tools = ROOT.parent / ".toolchains"
        local = tools / "cargo/bin/cargo.exe"
        cargo = str(local) if local.is_file() else "cargo"
        if local.is_file():
            env.update(CARGO_HOME=str(tools / "cargo"), RUSTUP_HOME=str(tools / "rustup"))
            env["PATH"] = str(local.parent) + os.pathsep + env.get("PATH", "")
    command = [cargo, "build", "--locked", "--release", "--bin", "my2pg",
               "--target", target, "--message-format=json-render-diagnostics"]
    if offline:
        command.append("--offline")
    output = run(command, capture=True, env=env)
    artifacts = []
    for line in output.splitlines():
        item = json.loads(line)
        if (item.get("reason") == "compiler-artifact"
                and item.get("target", {}).get("name") == "my2pg"
                and item.get("target", {}).get("kind") == ["bin"]
                and item.get("executable")
                and Path(item["manifest_path"]).resolve() == ROOT / "Cargo.toml"):
            artifacts.append(Path(item["executable"]))
    if len(artifacts) != 1 or not artifacts[0].is_file():
        raise ValueError("Cargo did not report exactly one existing my2pg executable")
    return artifacts[0]


def build_docker(target, destination):
    docker_platform = "linux/amd64" if target.startswith("x86_64-") else "linux/arm64"
    run(["docker", "buildx", "build", "--platform", docker_platform,
         "--target", "release-binary", "--output", f"type=local,dest={destination}", "."])
    binary = destination / "my2pg"
    if not binary.is_file():
        raise ValueError("Docker did not export the Linux executable")
    return binary


def digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def source_info():
    try:
        return {"revision": run(["git", "rev-parse", "HEAD"], True).strip(),
                "dirty": bool(run(["git", "status", "--porcelain"], True).strip())}
    except (OSError, subprocess.CalledProcessError):
        return {"revision": None, "dirty": None}


def package(binary, target, output, method):
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8").split("[dependencies]", 1)[0]
    version = re.search(r'^version\s*=\s*"([0-9A-Za-z.+-]+)"\s*$', manifest, re.MULTILINE)
    if version is None:
        raise ValueError("Cargo.toml has no explicit package version")
    name = f"my2pg-{version.group(1)}-{target}"
    windows = "windows" in target
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".my2pg-release-", dir=output) as temporary:
        work = Path(temporary)
        bundle = work / name
        bundle.mkdir()
        executable = bundle / ("my2pg.exe" if windows else "my2pg")
        shutil.copy2(binary, executable)
        executable.chmod(0o755)
        for relative in BUNDLE_FILES:
            destination = bundle / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / relative, destination)
        info = {"version": version.group(1), "target": target, "build_method": method,
                "source": source_info(), "binary_sha256": digest(executable),
                "toolchain": (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"),
                "migration_validation": "not performed by the build script"}
        if windows:
            info["platform_limitation"] = WINDOWS_LIMIT
            (bundle / "EXPERIMENTAL-WINDOWS.txt").write_text(WINDOWS_LIMIT + "\n", encoding="utf-8")
        (bundle / "BUILD-INFO.json").write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")
        files = sorted(path for path in bundle.rglob("*") if path.is_file())
        (bundle / "SHA256SUMS").write_text("".join(
            f"{digest(path)}  {path.relative_to(bundle).as_posix()}\n" for path in files), encoding="utf-8")
        archive = Path(shutil.make_archive(str(work / name), "zip" if windows else "gztar",
                                           root_dir=work, base_dir=name))
        checksum = digest(archive)
        destination = output / archive.name
        os.replace(archive, destination)
        destination.with_name(destination.name + ".sha256").write_text(
            f"{checksum}  {destination.name}\n", encoding="utf-8")
    return destination


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=TARGETS, required=True)
    parser.add_argument("--target", help="Rust target triple; defaults to the current CPU architecture")
    parser.add_argument("--out-dir", type=Path, default=ROOT / "target/dist")
    parser.add_argument("--docker", action="store_true", help="use the pinned Linux Docker builder")
    parser.add_argument("--offline", action="store_true", help="native Cargo only; use cached dependencies")
    args = parser.parse_args(argv)
    host = HOSTS.get(platform.system())
    docker = args.docker or (args.platform == "linux" and host != "linux")
    try:
        target = select_target(args.platform, args.target, host, platform.machine(), docker)
        if docker and args.offline:
            raise ValueError("--offline is for native Cargo builds; Docker controls its own cache/network")
        if args.platform == "windows":
            print(WINDOWS_LIMIT, file=sys.stderr)
        with tempfile.TemporaryDirectory(prefix="my2pg-linux-build-") as temporary:
            binary = build_docker(target, Path(temporary)) if docker else build_native(target, args.offline)
            result = package(binary, target, args.out_dir.resolve(), "docker" if docker else "cargo")
        print(result)
        print(result.with_name(result.name + ".sha256"))
        return 0
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"release build failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
