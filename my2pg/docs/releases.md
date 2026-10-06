# Building release archives

Run these commands from a source checkout. Python 3.9+ and the pinned Rust 1.99.0 toolchain are required for native builds. The scripts use `cargo build --locked --release --bin my2pg`, select the CPU target explicitly, and package only the production executable. Installing a Rust target does not install its system linker or SDK; see [Cargo build](https://doc.rust-lang.org/cargo/commands/cargo-build.html).

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
