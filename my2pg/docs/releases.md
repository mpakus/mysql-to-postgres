# Building release archives

Run these commands from a source checkout. Python 3.9+ and the pinned Rust 1.99.0 toolchain are required for native builds. The scripts use `cargo build --locked --release --bin my2pg`, select the CPU target explicitly, and package only the production executable. Installing a Rust target does not install its system linker or SDK; see [Cargo build](https://doc.rust-lang.org/cargo/commands/cargo-build.html).

## One build command

`bin/buld` is a Bash script available at both the repository root and inside `my2pg/`. With no platform argument it builds for the current OS and CPU. Explicit `macos` or `linux` selects the existing platform builder; all remaining options pass through unchanged.

```sh
./bin/buld
./bin/buld --offline
./bin/buld macos --target x86_64-apple-darwin
./bin/buld linux --docker --target x86_64-unknown-linux-gnu
./bin/buld linux --help
```

macOS compilation requires a Mac. Linux can compile natively on Linux or through Docker on a Mac. Outputs go to `my2pg/target/dist/` relative to the repository root, regardless of the directory from which the script is invoked.

## macOS

Install Xcode Command Line Tools and the selected Rust target. Both Apple Silicon and Intel are supported build targets; a build for the other CPU requires its Rust standard library and the matching Apple linker/SDK support. Cross-compilation is not a native migration test.

```sh
./bin/build-macos
./bin/build-macos --target x86_64-apple-darwin
./bin/build-macos --target aarch64-apple-darwin --offline
```

## Linux

On Linux, use the pinned Rust toolchain, a C compiler, `pkg-config` and OpenSSL development headers (on Debian/Ubuntu: `build-essential pkg-config libssl-dev`). Native builds default to the current CPU. Building another CPU natively requires its Rust target, C toolchain/linker and matching OpenSSL development libraries.

```sh
./bin/build-linux
./bin/build-linux --offline
```

The Docker path uses the pinned Debian Bookworm Rust builder from the project's Dockerfile and needs Docker with Buildx. It is selected automatically on macOS/Windows, or explicitly with `--docker` on Linux. Python is needed on the host; Rust is supplied by the container. Foreign CPU builds need Docker emulation or a matching builder. Docker's image/dependency cache and networking are separate from native Cargo's `--offline` option.

```sh
./bin/build-linux --docker --target x86_64-unknown-linux-gnu
./bin/build-linux --docker --target aarch64-unknown-linux-gnu
```

From Windows, invoke the shared script with `py -3 bin/build-release.py --platform linux --target x86_64-unknown-linux-gnu`. Exported Docker binaries require a compatible glibc Linux environment and OpenSSL 3 runtime libraries; they are not static musl binaries. The existing application container supplies those libraries.

## Experimental Windows compilation

On Windows, install Python, the pinned Rust toolchain and Visual Studio C++ Build Tools/Windows SDK for the MSVC target. Run in Developer PowerShell:

```powershell
./bin/build-windows.ps1
./bin/build-windows.ps1 --target x86_64-pc-windows-msvc --offline
```

The Windows ZIP is **experimental and unverified**. Current artifact directory syncing and private file modes are Unix-specific, and replacing existing report files needs Windows-specific review/tests. Use Linux (including WSL2) or macOS for migrations until these paths and database/failure tests are accepted on Windows. The build script prints this limitation and includes it in the ZIP and build metadata. Windows cross-compilation from macOS is not provided.

## Outputs and verification

Default output: `target/dist/my2pg-0.1.0-<target>.tar.gz` for Linux/macOS and `.zip` for Windows, each with an adjacent `.sha256` file. Use `--out-dir /path/to/output` to change the destination. Failed builds return nonzero and do not package a stale executable. The archive contains the binary, [operator docs](config-and-cli.md), [scope](scope-and-compatibility.md), examples, project/dependency notices, `BUILD-INFO.json` and a per-file `SHA256SUMS`. Source revision and dirty state are recorded when Git is available; dependency/toolchain inputs are pinned, but no independent-host reproducibility claim follows from this script.

Verify the archive on Linux with `sha256sum --check <archive>.sha256`, or on macOS with `shasum -a 256 --check <archive>.sha256`. On Windows, compare `Get-FileHash <archive> -Algorithm SHA256` with its sidecar. After extraction, verify `SHA256SUMS` from inside the bundle, then run `./my2pg --version` and `./my2pg --help` (Windows: `./my2pg.exe`).

These scripts compile/package locally. They do not publish, sign, notarize, run database tests, certify platform compatibility, or complete license/fixture review. Review the source checkout's release checklist before distributing artifacts.

## Automatic GitHub releases

The repository-root `.github/workflows/release.yml` runs on every branch push and can also be started manually from Actions. Tag pushes do not trigger it. It builds and tests native macOS Intel/Apple Silicon and Linux AMD64/ARM64 binaries using the existing packagers. All four builds, ordinary Rust tests, support tests, archive checksums and extracted native CLI checks must pass before publication. Database-dependent tests that the ordinary suite ignores remain separate acceptance gates.

Versions use the package's major/minor and the release workflow's incrementing run number as the patch: `v0.1.1`, `v0.1.2`, etc. Failed runs can leave gaps; reruns retain their version. The workflow stamps Cargo.toml and the root package entry in Cargo.lock only in its checkout, so the binary's `--version`, archive names and release tag agree without version-bump commits. Local builds retain the checked-in Cargo version. CI build metadata records the original source commit and a dirty tree because of that version stamp. Keep the workflow identity and its patch-number allocation when changing the version scheme.

Every successful run gets its own release on [GitHub Releases](https://github.com/mpakus/mysql-to-postgres/releases), including pushes to feature branches. The highest published version becomes Latest. Publication is serialized using GitHub's `queue: max` (at most 100 pending publications); older runs finishing late cannot replace a newer Latest release. The publisher creates a draft, uploads all four archives and their SHA-256 sidecars, downloads them and verifies checksums/source identity before publishing. Reruns recover drafts or verify an already published release without replacing its assets.

GitHub-hosted Linux builds use Ubuntu 24.04 and require glibc 2.39+ and OpenSSL 3. Local Docker builds retain the existing Debian Bookworm baseline. Builds are unsigned, and macOS builds are not notarized. These are automatic development builds; publication does not complete the broader migration/recovery, license or platform acceptance checklist. The workflow uses the repository's built-in `GITHUB_TOKEN` with `contents: write` only for publication; repository policy must permit that permission.
