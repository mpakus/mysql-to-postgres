# 2026-10-02 — T23 — project license notice

Status: partial release preparation; third-party notices and release artifacts remain open.

## Reference research before edits

- Read `AGENTS.md`, `docs/reference-coding.md`, `docs/implementation-plan.md`, `docs/agent-tasks.md` T23, `docs/checklists.md` release gates, `docs/scope-and-compatibility.md`, and the current `Cargo.toml`/README.
- Current source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the working tree contains active T12/T13/T21 work.
- XERJ `project-my2pg` query: `T23 release package dependency license provenance binary checksums`. It returned the implementation plan and task ownership; the plan requires all compatibility and performance gates before a release recommendation.
- XERJ `ref-dmt-rs` query: `cargo install release binary Dockerfile license package`, pinned commit `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Read its original `README.md:395-423`: it shows a small multi-stage binary image and declares MIT. I use the short packaging shape only as a design reference and reject its license because my2pg's `Cargo.toml` already declares `PostgreSQL`.
- XERJ `project-pgloader` query: `SQLite PostgreSQL-as-source unsupported source excluded drivers dependencies`, pinned commit `231ab86778ca5ffd7de40878714760c8b4860cdf`. Results confirm the broad legacy source matrix that this release must exclude. Read the upstream `LICENSE` to avoid attributing its copyright to my2pg; no source code or fixture is copied by this license change.
- Direct project evidence: `src/config/validation.rs` performs offline configuration checks; `tests/importer.rs:340-370` rejects SQLite, MSSQL, PostgreSQL-as-source, JDBC URLs and malformed target schemes. `tests/fixtures/upstream/LICENSE` and `DATASET-NOTICES.md` retain separate provenance for reviewed legacy fixtures.
- Primary license text: the [official PostgreSQL license page](https://www.postgresql.org/about/licence/) supplies the PostgreSQL-license permission and disclaimer text. The package declares `license = "PostgreSQL"` but has no root LICENSE file.

## Decision and acceptance

Add a root license notice using the license already declared by Cargo, with a project copyright notice and no third-party copyright attribution. Link the notice from the README. Do not change package license metadata, dependency versions, or the scope of the source adapters. This closes only the project's base license notice; it does not close third-party license collection, fixture attribution review, reproducible builds, binaries, containers or checksums.

Acceptance: the notice must retain all three license paragraphs, the package metadata must continue to declare PostgreSQL, Cargo's package file listing must include LICENSE, README must link it, and the license text must match the official permission/disclaimer wording. Existing fixture notices remain separate.

## Ponytail and RTK

Read Ponytail at `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`. Reuse the existing standard Cargo license field, add only the absent notice, and add no package or build dependency. All repository inspection uses `rtk run`.

## Post-edit validation and packaging finding

- `bin/cargo package --list --allow-dirty --no-verify --locked --offline` exited 0 and listed `LICENSE`. It warned that documentation, homepage and repository package metadata are absent.
- The default Cargo package file set also includes project-internal task/worklog material. Cargo's current guidance confirms `include`/`exclude` control published files and recommends reviewing `cargo package --list`; see [Cargo manifest packaging](https://doc.rust-lang.org/cargo/reference/manifest.html#the-exclude-and-include-fields). I have not changed the archive boundary yet because the operator-facing docs contain links to the worklogs and need a deliberate source-package decision in T23.
- Correction after populating Cargo's target-specific cache: `bin/cargo metadata --locked --format-version 1` completed successfully. It reported 223 packages and no package missing both a license expression and `license_file`; expressions span PostgreSQL, MIT/Apache/BSD/ISC, Unicode, CDLA, Zlib, Unlicense, BSL and an optional LGPL alternative. This is metadata coverage only, not a complete notice bundle or compatibility review. `cargo-about` is not installed, so no license texts or rendered third-party notices have been assembled. The release checklist remains unchecked.
