# T23 — local locked build and package reproducibility

Status: same-host repeat-build experiment passed; clean-host and cross-platform reproducibility remain open.

## Baseline and research

- Project HEAD before experiment: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the checkout is dirty, so built inputs include the current uncommitted implementation.
- XERJ project query: `project-my2pg "reproducible Cargo build package checksum toolchain" -k 6 --full 80`. Results at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` pointed to source-archive, license, and dependency-notice audits. Those worklogs separate Cargo.lock download checksums from release artifact checksums.
- XERJ reference query: `ref-dmt-rs "release sha256 checksums build artifact" -k 5 --full 80`, pinned commit `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. I read the original `../references/dmt-rs/.github/workflows/release.yml`: its release job collects build outputs and produces SHA-256 checksums, but it does not establish bit-for-bit repeatability.
- Read current `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, T23 source-archive and dependency-notice worklogs, and `tests/support/test_package.py` (which checks the allowlist, retained provenance/notices, and shipped Markdown links).
- Existing package metadata pins Rust 1.99.0 and registry crates in Cargo.lock; the package verifier previously passed but this checkout's release outputs were not built independently twice. Lock checksums authenticate fetched crate archives; they are not checksums for the project binary or `.crate` archive.
- No production code change is proposed. The experiment will use two empty target directories and the already pinned offline toolchain/dependency set, then compare SHA-256 hashes for the binary and source package.

## Experiment contract

Build twice with the same dirty source snapshot, host/architecture, Rust 1.99.0, release profile, and Cargo.lock, but separate target directories. Produce and retain both hashes. Repeat the source-package build and compare the `.crate` archive too. A matching pair proves same-host/same-source deterministic output only; it does not prove clean checkout, independent host, Linux, macOS x86_64, container, or legal notice acceptance.

## Validation

- Built release binary twice from the same dirty source snapshot with Rust 1.99.0, `--offline --locked`, and distinct empty `CARGO_TARGET_DIR` values. SHA-256 matched: `af1b806434be6c55f621af68598ac067e3341f92e3f35542317b8562f5212093`; `cmp` confirmed byte-identical files.
- Ran `cargo package --locked --offline --allow-dirty` twice with distinct empty target directories. Both source archives were 208 files (2.3 MiB unpacked; 490.6 KiB compressed), SHA-256 `f34a32a7573e7ed3f36fe0d6ad571bb46de0242b3a62a502f47bb8d38b168277`; `cmp` confirmed byte-identical archives.
- These results are limited to this macOS ARM64 host, the current dirty source snapshot, Rust 1.99.0, and the locked locally available dependency set. They do not prove clean-checkout or independent-host builds, other operating systems/architectures, container reproducibility, release checksums, or completion of dependency notices and license review.
- No implementation code changed during this experiment.
