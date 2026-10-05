# 2026-10-02 — coordinator — T13, source archive and XERJ closeout

Status: T13 characterization and T23 source-package boundary are verified; larger release gates remain open.

## Project and reference search

- Followed `docs/reference-coding.md` before integrating the required T13 test and before changing package boundaries.
- Project search: `project-my2pg "SingleSnapshot external metadata lock cancellation later table"`; pinned searches: `ref-mysql_async "query result cancellation drop stream"` and `ref-dmt-rs "cancellation bounded producer signal"`. The final diagnosis and source locations are in `worklog/2026-10-02-T13-snapshot-mdl-registration.md`.
- Package-boundary project search: `project-my2pg "Cargo package source distribution boundary package include docs worklog"`; pinned search: `ref-dmt-rs "Cargo package include manifest exclude"`. The Cargo allowlist and archive acceptance results are in `worklog/2026-10-02-T23-source-archive-boundary.md`.
- Project HEAD is `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the checkout is dirty, so it does not identify the full working snapshot. Immutable references remain pinned and read-only.

## Verification

- T13 exact-case discovery listed the new test among 158 ignored integration tests. The focused MySQL 8.4/PostgreSQL 16 case passed. The full fresh lane passed 155 cases; all 25 applicable required IDs ran exactly once; no failed or unmet IDs. See `worklog/2026-10-02-T13-single-snapshot-mdl.md` for fixture ID and artifact hashes.
- The T13 test confirms a real server-side limitation: the CLI reports cancellation with zero acknowledged rows while MySQL retains the blocked server thread until its external metadata lock is released. The required test records this behavior and ensures fixture cleanup; a product fix and broader shutdown acceptance remain open.
- Locked offline Cargo source-package verification passed for 207 files. The 22 database-free Python support tests passed, including package contents, local links, registry validation and matrix checks. The include allowlist preserves upstream notices and provenance while excluding internal plans, worklogs and generated baselines.
- `git diff --check` passed. The other five base matrix cells and separate PostGIS lane need reruns against the newly expanded required-case registry. T12 physical fsync fault proof, MySQL 5.7 platform availability, NLS, fuzz/soak, CI, dependency notice review and T24 independent release audit remain open.

## Shared XERJ project index

The coordinator refreshed only `project-my2pg`; reference pins and indexes were not changed. Refresh and representative-search results will be recorded here after the rebuild.

- Final refresh output: `project-my2pg-source`, 374 files, 1,311 passages, 11 recorded exclusions. Coverage report revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` (HEAD; sources also include the dirty working tree).
- Representative query `project-my2pg "cancellation_leaves_single_snapshot_select_waiting_until_external_mdl_releases" -k 2 --full 35` returned the current `tests/cases/t13_snapshot_mdl.rs` definition.
- Representative query `project-my2pg "Cargo include source archive local links fixture notices provenance" -k 3 --full 35` returned the T23 source-archive worklog and the current coordinator closeout.
- The project index remains lexical retrieval, not semantic/vector search. Every hit was checked against original files. One last refresh follows this evidence update so the final worklog text itself is indexed.
