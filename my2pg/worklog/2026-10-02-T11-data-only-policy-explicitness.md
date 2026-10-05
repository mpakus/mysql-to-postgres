# 2026-10-02 — T11 — make data-only target policy explicit

Status: contract resolution and implementation slice in progress; T11 remains open.

## Reference research before code

- Reviewed `docs/scope-and-compatibility.md` structure/mode contract, `docs/config-and-cli.md` target policy and `.load` importer contract, `docs/architecture.md` target preparation, T11 in `docs/agent-tasks.md`, and the delegated [policy/mode audit](2026-10-02-T11-policy-mode-audit.md).
- Project XERJ query: `project-my2pg "DataOnly on_existing error implicit append explicit append truncate validation planner" -k 10 --full 100`; indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Findings pointed to `src/config/validation.rs:250-270`, `src/plan/mod.rs:994-1002`, `src/config/import.rs:1193-1205`, and `tests/importer.rs:111-131`; read those exact sources and neighboring tests.
- Pinned pgloader XERJ query: `project-pgloader "data only include no drop truncate append existing PostgreSQL target" -k 8 --full 90`; revision `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `src/load/migrate-database.lisp:61-100` and `clojure/src/pgloader/ddl/common.clj:635-674` with `clojure/test/pgloader/ddl_test.clj:461-490`. The legacy loader treats “not creating tables” as a distinct write path and its reset derives from MAX; preserve our safer target checks and monotonic sequences rather than copying those mechanics.
- Pinned Paganel query: `ref-paganel "existing target write mode append replace data only" -k 8 --full 90`; revision `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`. Read `crates/engine-planner/src/plan/pipeline/destination.rs:1-80`: it represents destination write mode explicitly. Adapt the explicitness principle only; do not adopt generic replace/upsert modes.
- Current behavior: config validation accepts `mode=data_only` with `on_existing=error`; planner skips `TARGET_EXISTS` for that combination, so it acts as implicit append. Import's `--append-data-only` explicitly converts `error` to `append`, but without that option it currently relies on the same implicit behavior. The architecture says append is explicit; make ambiguity fail closed.
- RTK was used for all local reads/queries. Ponytail executable/skill remains unavailable in the checked roots; no Ponytail execution is claimed.

## Chosen behavior and acceptance

Reject `data_only` paired with `on_existing=error` during offline configuration validation. Require callers to choose `append` or `truncate`; this keeps the default `error` policy meaningful, makes plans/reports state the intended effect, and removes a hidden alias. Preserve importer opt-in: `--append-data-only` becomes required for a legacy data-only/create-no-tables load unless the source config explicitly asks for truncate; importer errors should identify the missing choice rather than emit invalid TOML. Add config/planner/importer regressions and an actual `check` refusal before database access. Do not change runtime DDL or data handling.
