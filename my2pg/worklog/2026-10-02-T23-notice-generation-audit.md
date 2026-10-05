# 2026-10-02 — T23 — dependency notice generation and source distribution audit

Status: audit complete; dependency notices and license review remain open. This audit changed only this worklog.

## Reference research and revision

- Project HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the working tree is dirty, so all package observations below refer to the current tree, not that committed snapshot.
- Read `docs/reference-coding.md:1-32, 80-90`, `docs/implementation-plan.md:13, 25, 41-43`, `docs/agent-tasks.md:64, 161`, `docs/checklists.md:92, 103`, `Cargo.toml:1-42`, `Cargo.lock:1-100`, `README.md:11`, and `tests/support/test_package.py:1-85`.
- Existing successful project XERJ queries in [the earlier package audit](2026-10-02-T23-notices-boundary-audit.md): `project-my2pg "Cargo source package include exclude license notices fixture provenance" -k 5 --full 30` and `project-my2pg "T23 cargo package worklog include exclude archive" -k 8 --full 40`. That report identifies the index revision as `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` and warns it did not represent the full dirty tree.
- Current-attempt XERJ queries: `project-my2pg "dependency notices Cargo package archive license" -k 8 --full 120` and `ref "Cargo license notice attribution bundle" -k 6 --full 100`. Both failed before querying because the sandbox denied loopback access to the configured XERJ endpoint (`PermissionError: [Errno 1] Operation not permitted`). I did not rebuild or mutate shared indexes.
- The earlier audit searched `ref-dmt-rs "Cargo.toml license package metadata Dockerfile" -k 8 --full 40`, pinned revision `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`, and read the original workspace/crate manifests. The pinned manifest is also recorded in `docs/reference-repositories.json:8`. Its workspace-level license metadata is an organizational example only; it does not answer which dependency texts or attributions my2pg must ship.
- Ponytail guidance was read at `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`; this is a read-only audit, so no code/dependency change was made.

## Current package and dependency evidence

- `Cargo.toml:1-20` declares a PostgreSQL-licensed package and an explicit source archive allowlist. It includes the project source/tests, operator docs and examples, root license and pinned toolchain; Cargo adds its manifest and lockfile. `Cargo.toml:24-42` declares MySQL/PostgreSQL runtime crates and platform-specific Unix dependencies.
- Command `bin/cargo package --list --allow-dirty --no-verify --locked --offline` exited 0 and returned 207 paths. Of those, 42 are under `tests/fixtures/upstream/`: 40 retained upstream source/example files plus `LICENSE` and `DATASET-NOTICES.md`. The package also contains `tests/fixtures/upstream-manifest.json`. The allowlist therefore preserves the notices and provenance alongside copied fixture inputs. This path listing does not establish that each notice is complete or legally applicable.
- `tests/fixtures/README.md:5-9` says the pgloader source fixtures were copied from a pinned upstream snapshot with its license and dataset notices; it also says the Sakila/F1DB datasets themselves are not copied. `tests/fixtures/upstream-manifest.json:3-5, 682-694` pins fixture provenance and flags external dataset status, including F1DB attribution review before redistribution. `tests/fixtures/upstream/LICENSE:1-7` and `tests/fixtures/upstream/DATASET-NOTICES.md:1-76` are preserved in the archive. A human must still review whether these notices correctly cover the exact selected material and whether any inherited notice is required in executable distributions.
- `bin/cargo metadata --locked --format-version 1 --offline` succeeded. A local JSON summary found 223 package records, none missing both a `license` expression and `license_file`; the records include compound/alternative expressions such as `MIT OR Apache-2.0`, an optional LGPL alternative, Unicode, CDLA, PostgreSQL, BSD, ISC, Zlib, Unlicense and BSL. Metadata presence is not verification of the underlying full license text, correct expression, selected license alternative, copyright attribution, obligations, compatibility, or actual link/runtime inclusion.
- `Cargo.lock` pins exact registry versions and checksums, but it is not a notice bundle. Metadata's 223-package universe also should not automatically be treated as the shipped executable's graph: distinguish normal runtime dependencies from dev/test dependencies, optional features, target-specific dependencies and build dependencies, then generate/check the distribution notice for each actual binary target and shipped feature set.
- Local tooling check: Cargo is `1.99.0 (5f94df478 2026-08-27)` through `bin/cargo`; the root `rustc` command is not on this shell's PATH. `bin/cargo about --version`, `bin/cargo license --version`, and `bin/cargo deny --version` each report no such Cargo subcommand. `cargo-about`, `cargo-license`, and `cargo-deny` are not installed. No license bundle was generated.
- Current README says only that the project is distributed under the PostgreSQL license and that reviewed upstream fixtures retain separate terms (`README.md:11`). There is no generated third-party dependency notice in the 207-path package. The release checklist explicitly leaves dependency review/artifact checksums open at `docs/checklists.md:103`.

## Recommendation and acceptance evidence

The minimal implementation path is to pin a dedicated notice generator in the release tooling (not as a runtime dependency), commit its reviewed configuration/template, and make generation deterministic in CI from `Cargo.lock`. `cargo-about` is a plausible candidate because its official project describes dependency-license aggregation and its documented command is `cargo about generate <template>`; the project also supports explicit license clarification/configuration. A separate policy checker such as `cargo-deny` can enforce the reviewed policy, but it does not replace the rendered attribution texts. Official primary references: [cargo-about README](https://github.com/EmbarkStudios/cargo-about/blob/main/README.md), [cargo-about generation configuration](https://embarkstudios.github.io/cargo-about/cli/generate/config.html), and [Cargo package license metadata](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields).

Before enabling a green release gate, the owner should:

1. Select the exact runtime targets/features and dependency edge set being distributed; separately decide whether source archives need third-party notices for dev/test-only crates.
2. Pin the generator version and configuration/template, generate a plain-text `THIRD-PARTY-NOTICES` artifact into the source archive and every binary/container distribution, and fail CI if regeneration differs or any included crate is unresolved.
3. Review every `OR`/`WITH`/compound license and any clarification against the pinned crate's actual license and attribution files. Have a qualified human review legal compatibility and fixture attribution; neither Cargo metadata nor a green tool result is a legal conclusion.
4. Verify the generated notice names and texts against the exact release's `Cargo.lock`, binary features, platform and container contents; retain the generated files, tool versions, lockfile, build inputs and checksums with the release evidence.

`cargo-about` documentation itself disclaims legal advice. Tool output can automate collection and policy checks, but this audit does not determine compliance. The F1DB attribution-review gate remains relevant if that corpus is ever copied or provisioned for distribution.

## Checks and limits

- Package listing: passed, 207 paths; no package created or published by this audit.
- Offline Cargo metadata parse and 223-package summary: passed.
- Notice-generator, license-summary and policy-checker availability: all three Cargo subcommands unavailable; no installation was attempted.
- XERJ current queries: blocked by sandbox loopback permission; prior successful search evidence and pinned revision are recorded above and in the earlier T23 audit.
- No code, Cargo configuration, package contents, CI/release configuration, fixture set, binary/container, generated notice or shared index was changed. No legal compliance opinion is offered.
