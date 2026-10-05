# 2026-10-02 — T23 — source archive boundary

Status: in progress.

## Reference research (recorded before changes)

- Task: define a reviewable public Cargo source archive for the MySQL-to-PostgreSQL CLI while retaining the source, tests, examples, documentation, license and every retained upstream fixture's notice and provenance.
- Project HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree is dirty, so this identifies the base commit only.
- Project XERJ query: `project-my2pg "Cargo package source distribution boundary package include docs worklog" -k 5 --full 60`. The source index is pinned to `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; useful hits were the existing T23 archive audit, T23 license worklog, and current release checklist. Read originals rather than treating passages as complete.
- Pinned-reference XERJ query: `ref-dmt-rs "Cargo package include manifest exclude" -k 5 --full 60`; pinned revision `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Search did not find package-boundary guidance, so no packaging behavior is borrowed from this reference.
- Read `Cargo.toml:1-42`; its package currently has no file allowlist. Read `README.md:1-16`; it links to internal plan, task board, worklogs and XERJ setup that should not ship to CLI users. Read `docs/config-and-cli.md:93` and `docs/scope-and-compatibility.md:73,79`; these contain internal worklog links that would break if worklogs are excluded.
- Read `worklog/2026-10-02-T23-notices-boundary-audit.md`: the current `cargo package --list` selected 377 paths including 143 worklogs and internal docs. `tests/fixtures/README.md:5-9`, `tests/fixtures/upstream-manifest.json:3-5,682-694`, `tests/fixtures/upstream/LICENSE:1-7`, and `tests/fixtures/upstream/DATASET-NOTICES.md:6-76` establish required fixture provenance and notices.
- Official Cargo manifest reference: https://doc.rust-lang.org/cargo/reference/manifest.html?highlight=exclude . `include` provides an explicit packaged-file allowlist; `cargo package --list` is the inspection seam and Cargo.toml / minimized Cargo.lock are handled by Cargo.
- Guidance applied: repository-required RTK and Ponytail workflow; TDD skill (`/Users/renatibragimov/.agents/skills/tdd/SKILL.md`) for an observable packaging seam. Add a package-list acceptance test before changing the manifest or package-facing docs.

## Decision and acceptance

Use Cargo's `include` allowlist for public runtime source, tests and their non-baseline fixtures, user/operator documentation, examples, README, license and toolchain pin. Exclude internal agent directions, historical worklogs, planning/reference setup, source audits and generated baseline artifacts. Preserve upstream fixture license, dataset notice and provenance manifest. Rewrite package-facing links so they resolve inside the archive. Test required and forbidden paths plus all local Markdown links in the package list, then run a real locked offline `cargo package` verification.

## Result

- Added the package allowlist in `Cargo.toml`; excluded `tests/fixtures/baselines/**`, worklogs and all internal planning/reference docs by omission.
- Rewrote the README's package-facing documentation links and removed worklog/source-audit links from the included configuration and scope docs.
- Added `tests/support/test_package.py`. It checks runtime/test/docs/fixture notice and provenance paths, excludes internal/generated material, and resolves relative Markdown links against the actual archive path set.
- Red phase: the first list showed all six explicitly forbidden internal paths and 143 worklogs; after allowlisting, the test found an unpublished `source-audit.md` link. Both findings were fixed.
- `PYTHONPYCACHEPREFIX=/private/tmp/t23-pycache python3 -m unittest discover -s tests/support -p test_package.py -v`: 3 passed.
- `bin/cargo package --allow-dirty --locked --offline`: passed Cargo's package verification build, 207 files, 2.2 MiB (482.7 KiB compressed). Cargo still warns that homepage/repository/documentation metadata is absent; these are package metadata polish and do not affect this boundary check.
- `bin/cargo package --list --allow-dirty --no-verify --locked --offline`: shows only allowed runtime/test/docs/example/material and includes `tests/fixtures/upstream/LICENSE`, `tests/fixtures/upstream/DATASET-NOTICES.md`, and `tests/fixtures/upstream-manifest.json`.
- `PYTHONPYCACHEPREFIX=/private/tmp/pg-support-pycache python3 -m unittest discover -s tests/support -p "test_*.py" -v`: all 22 support tests passed, including package boundary, harness registry and matrix tests.
- `git diff --check`: passed after final worklog/checklist edits.

This closes the source archive boundary slice, not T23. Dependency notice rendering and review, dataset attribution decisions, reproducible builds, binaries/container and checksums remain open.
