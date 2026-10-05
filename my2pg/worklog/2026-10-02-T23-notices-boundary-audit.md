# 2026-10-02 — T23 — notices and Cargo source-package boundary audit

Status: audit complete; T23 remains open. This entry makes no source or packaging changes.

## Evidence and reference research

- Project revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. The checkout was dirty during this audit, so this SHA identifies HEAD only; the package list reflects the current working tree.
- Read `docs/reference-coding.md:1-32` for the required XERJ workflow and `docs/implementation-plan.md:13,25,41-43` for T23's release/package/provenance gates.
- XERJ project query: `project-my2pg "Cargo source package include exclude license notices fixture provenance" -k 5 --full 30`. It returned `tests/fixtures/upstream-manifest.json`, `tests/fixtures/README.md`, and the implementation plan, all at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. The index is a search aid and does not reflect this entire dirty snapshot.
- XERJ project follow-up: `project-my2pg "T23 cargo package worklog include exclude archive" -k 8 --full 40`. It returned project README, implementation/task docs and worklogs, but no specific Cargo include/exclude decision.
- XERJ pinned-reference query: `ref-dmt-rs "Cargo.toml license package metadata Dockerfile" -k 8 --full 40`, pinned revision `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Read the original workspace and crate manifests at `references/dmt-rs/Cargo.toml:5-11` and `references/dmt-rs/crates/dmt-rs/Cargo.toml:1-10`: the reference puts MIT and repository metadata in workspace package fields inherited by its crate. This is metadata organization only; it does not settle my2pg's archive contents or notice obligations. The earlier T23 worklog records the separate read of its README packaging example.
- Project manifest: `Cargo.toml:1-7` declares a single `my2pg` package and `license = "PostgreSQL"`; it has no `include` or `exclude` field. `Cargo.toml:24-42` declares runtime dependencies, including the MySQL and PostgreSQL clients. The root `LICENSE` and the README license link were added in the separate T23 project-license slice; `README.md:13` says reviewed fixtures keep their own notices.
- Fixture provenance: `tests/fixtures/README.md:5-9` describes the pinned pgloader source snapshot, its manifest, the copied upstream `LICENSE` and `DATASET-NOTICES.md`, and says Sakila/F1DB datasets are not copied. `tests/fixtures/upstream-manifest.json:3-5` records pgloader URL/revision and fixture selection; lines `682-694` state external Sakila is not provisioned and F1DB redistribution requires attribution review. The included notice text is in `tests/fixtures/upstream/LICENSE:1-7` and dataset terms in `tests/fixtures/upstream/DATASET-NOTICES.md:6-76`.

## Cargo source-package observation

Command: `bin/cargo package --list --allow-dirty --no-verify --locked --offline` from `my2pg/`. It exited 0 and emitted 377 paths. It includes 143 `worklog/` files, 14 `docs/` files (including internal task, plan, checklist, source-audit and reference-workflow documents), 149 `tests/` files, and 58 `src/` files. The set also includes `AGENTS.md`, `.github`, Cargo lock/manifest, toolchain file, README and root LICENSE. Among the 61 `tests/fixtures/` paths are 42 under `tests/fixtures/upstream/`, plus the upstream manifest, the two notice files, synthetic fixtures and baseline logs. No sibling `references/` checkout appears in the package list.

This proves Cargo currently selects a broad source archive and that the copied fixture notices are present beside the retained upstream inputs. It does not establish whether worklogs and planning documents are intended public package content, whether every fixture has a correct applicable notice, whether third-party dependency notice texts are assembled, or what release binaries/container images contain. `cargo metadata` reported 223 packages with license metadata, but that metadata inventory is not a rendered notice bundle or a legal compatibility review; `cargo-about` was unavailable during the prior T23 check.

## Recommended bounded next step and acceptance

Have the T23 packaging owner make and record the intended source-archive audience, then encode a reviewed allowlist (or equivalent exclusion policy) in Cargo metadata. Keep all source/test inputs required by the packaged test targets, and keep `tests/fixtures/upstream/LICENSE`, `tests/fixtures/upstream/DATASET-NOTICES.md`, and the provenance manifest whenever upstream fixture files remain in the archive. Decide explicitly whether to publish the internal task board, agent instructions, historical worklogs, and XERJ/reference setup docs; the current README and development docs link into those materials, so changing the boundary also requires checking for broken links in the package-facing documentation.

Acceptance should include a saved `cargo package --list` result with path-based assertions for the chosen allowlist and denylist; a successful `cargo package --locked` (without `--no-verify`); a test of the unpacked package or packaged test targets proving required fixtures remain available; and a check that every retained upstream fixture has its matching provenance/notice files. Dependency license collection, binary/container contents and checksums remain separate release gates and need their own artifacts and checks.

## Checks and limits

- `bin/cargo package --list --allow-dirty --no-verify --locked --offline`: passed; warning reports missing package documentation, homepage and repository metadata.
- No database fixtures were started. No source, manifest, license, README, task board, checklist, or shared XERJ index/pin was changed by this audit.
- The package-file result is a snapshot of the dirty working tree, not a verified publish artifact. Full license terms and source/binary/container distribution have not been independently reviewed here.
