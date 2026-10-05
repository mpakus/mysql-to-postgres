# 2026-10-05 — T22 — SingleSnapshot nullable processlist row

- Status: nullable processlist test oracle fixed; six current-source ARM64 base cells pass
- Agent/role: coordinator / integrator (temporary lease of `tests/cases/t13_snapshot_mdl.rs`)
- Task and specification links: [T22 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release), [T13 SingleSnapshot cancellation contract](2026-10-02-T13-single-snapshot-control-cancel.md)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty

## Failure evidence

The current-source serial matrix passed 155 cases on MySQL 8.4.11/PostgreSQL 18.6 and failed `t13_snapshot_mdl::cancellation_terminates_single_snapshot_reader_blocked_during_preflight`. Its panic is in the pinned `mysql_common` row conversion: `Couldn't convert Row { STATE: Null } to type alloc::string::String`. The fixture project is recorded stopped. The other five current-source base cells passed 156 cases each; this sixth cell is not accepted until repaired and rerun.

The query in `tests/cases/t13_snapshot_mdl.rs::blocked_reader` selects `ID, STATE, INFO` from `information_schema.processlist`. `INFO` is optional, but `STATE` is decoded as `String`. The same helper is shared by two cancellation cases. A processlist row whose worker is disappearing can have `STATE = NULL`; this is an observation race and must not panic before test cleanup.

## Reference research before code

- XERJ project query: `SingleSnapshot blocked_reader STATE nullable processlist`, prefix `project-my2pg`; retrieved `tests/cases/t13_snapshot_mdl.rs` and the prior nullable processlist analysis in `worklog/2026-10-02-T13-external-metadata-lock.md`.
- XERJ driver query: `from_row Option nullable column Row conversion`, prefix `ref-mysql_async`, pinned revision `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`.
- Project evidence: `worklog/2026-10-02-T13-external-metadata-lock.md` records the exact failure mode and adaptation: nullable processlist values are decoded as `Option`, and disappearance polling selects only stable connection identity.
- Driver evidence: `references/mysql_async/src/queryable/mod.rs:355-364` documents that `exec_map` converts through `FromRow`; incompatible non-optional types panic. `references/mysql_async/src/queryable/query_result/mod.rs:251-263` offers `try_collect` for fallible conversion. `references/mysql_async/src/conn/mod.rs:1847-1856` has adjacent typed-row tests with `Option<String>` and SQL NULL values. The pinned public API reexports `FromRow` and `from_row_opt` at `references/mysql_async/src/lib.rs:565`.
- RTK is available and used for all commands. The configured skill roots contain no standalone `ponytail` or `rtk` skill file; follow this repository's required RTK wrappers and Ponytail's smallest-complete-solution guidance recorded in the existing T13 worklog. This is a test-only oracle repair, not a driver or migration behavior change.

## Chosen adaptation and acceptance

Change `BlockedReader.state` to `Option<String>` and decode the selected column accordingly. Treat NULL as “not observed in the required metadata-lock state yet”; the polling deadline and exact query/worker assertions remain the acceptance oracle. Keep the existing optional `INFO` and disappearance-by-ID checks. Run formatting, the focused `t13_snapshot_mdl` cases, then rerun the failed full `mysql84 pg18` lane. Rerun all six base lanes against the resulting source executable if the repair changes the lane's binary hash; do not count the failed lane as passing.

## Resolution and verification

Changed the single shared `blocked_reader` helper to decode `STATE` as `Option<String>` and both observing tests now wait until a non-null value reports a metadata-lock wait. This preserves the original 15-second observation deadline, SELECT-shape assertion, exact backend-disappearance checks, and cleanup path. No production migration code changed.

The first post-fix compile found two final assertions that still treated the new optional state as a string; those were changed to use the same checked optional predicate. Formatting, integration-target compilation, and strict integration-target Clippy then passed. The complete MySQL 8.4.11→PostgreSQL 18.6 native lane passed all 156 cases, including the repaired cancellation case, with zero failed, unmet-required, or unclassified IDs. The runner stopped the owned fixture project. The five other base lanes were rerun after this test change; see the [current-source six-cell matrix refresh](2026-10-05-T22-current-source-matrix-refresh.md).

The observed initial failure and its raw artifacts remain under `target/integration/my2pg-mysql84-pg18-105650d0abaa/`; that ledger is not counted as a pass. The accepted rerun is `target/integration/my2pg-mysql84-pg18-3a85373dda7a/`. This resolves the local nullable processlist test-oracle issue; MySQL 5.7 AMD64, hosted CI, native Linux, optional platform lanes, and broad soak/fault acceptance remain open.
