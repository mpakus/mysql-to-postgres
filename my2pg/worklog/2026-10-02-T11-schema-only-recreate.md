# 2026-10-02 — T11 — schema-only recreate regression

Status: native regression passed on MySQL 8.4.11/PostgreSQL 16.15; broader T11 acceptance remains open. Ownership was limited to `tests/cases/t11_schema_only_recreate.rs` and this worklog. Coordinator retained registration, task status, and shared indexes.

## Reference research before coding

- Reviewed `docs/reference-coding.md`, T11 in `docs/agent-tasks.md`, schema-only and identity requirements in `docs/scope-and-compatibility.md:83-93`, and target policy rules in `docs/architecture.md:68-76`.
- XERJ query: `project-my2pg "schema_only recreate existing selected table zero rows identity recreate RESTRICT native test"`; current indexed working revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It returned `src/plan/mod.rs:1141`, T11 existing-policy/structure tests, and the reviewed RESTRICT policy. Read original planner logic in `src/plan/mod.rs:938-1003,1130-1174,1605-1619` and runner sequence-finalization ordering in `src/pipeline/mod.rs:1403-1422`.
- XERJ query: `project-pgloader "create tables false schema only existing destination recreate drop table"`; pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `pgloader/src/load/migrate-database.lisp:15-105`: its table-creation branch prepares SQL types, sequences, then tables; its no-create branch instead merges target catalog constraints and may drop keys/indexes before load. That legacy data-loading path is not the schema-only recreate contract and is not copied.
- XERJ query: `ref-paganel "schema only existing destination replace table recreate DDL plan"`; pin `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`. Read `crates/engine-planner/src/plan/pipeline/destination.rs:5-78`; its explicit generic write-mode/data-impact model is useful context, but its replace/append abstractions are not imported into my2pg.
- Adjacent project evidence read directly: `tests/cases/t11_existing_policies.rs:243-289,1035-1040` executes full-mode recreate and only planner-checks schema-only recreate privilege; `tests/cases/t11_structures.rs:9-52,150-205` runs full/schema-only on fresh schemas and verifies schema-only row count, constraints, and generated identity. These leave the existing-object schema-only recreate runtime combination uncovered.
- RTK wrappers were used for source/reference inspection. Ponytail guidance or executable remains unavailable in the checked skill/cache roots; no Ponytail run is claimed. No test code was edited before this pre-code record.

## Chosen behavioral proof

Add one ignored native CLI case for `migration.mode = schema_only` and `target.on_existing = recreate`. Seed the selected PostgreSQL table with rows and deliberately stale structure, retain its relation OID, and seed an unrelated table in a separate schema while saving its OID/data. Create an InnoDB MySQL source table with a primary key, AUTO_INCREMENT, an additional unique key, source rows, and a known next identity. Run the real binary. Assert the selected table now has a different relation OID, exactly the source-planned columns/PK/unique structure and BY DEFAULT identity, none of the stale target columns/constraints remain, zero rows were copied, and the new owned sequence exists with the expected next value. Assert the unrelated sentinel table keeps its OID and contents. Drop only test-owned MySQL/target objects during cleanup.

The case is ignored like neighboring TLS-dependent acceptance tests. Compile/discovery and formatting can be checked without opening a database pair; module registration stays with the coordinator.

## Implementation and verification

Added the ignored native CLI regression in `tests/cases/t11_schema_only_recreate.rs`. It uses distinct process-scoped MySQL and PostgreSQL objects, verifies MySQL reports next identity 42, recreates the pre-populated selected table, and checks new relation OID, exact columns/types/nullability/identity, PK/unique constraints, absence of old constraint/index, zero target rows, and next sequence value 42. It also checks the outside sentinel's OID and row values are unchanged.

- `my2pg/bin/cargo fmt --all -- --check tests/cases/t11_schema_only_recreate.rs` passed.
- `my2pg/bin/cargo test --offline --locked --test integration --no-run` passed for the currently registered integration suite. The new module is not registered yet because `tests/integration.rs` is coordinator-owned; the new source therefore still needs its focused compile/discovery check after registration.
- The first native DB run exposed the incorrect catalog oracle documented below. The corrected case passed on the coordinator's subsequent fresh serial lane.

## Follow-up research before correcting the structural oracle

- Saved lane `target/integration/my2pg-mysql84-pg16-2912c263fbbc/rust-tests.json` records 143 passed cases and only this case failed; `rust-tests.log:691-693` shows the assertion expected two `pg_constraint` rows and observed one. The binary preparation in that lane exited 0. No DB rerun is authorized yet.
- Fresh project XERJ query: `project-my2pg "T11 source UNIQUE key planner emits unique index pg_constraint pg_index schema_only CREATE"`; indexed working revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It returned `src/plan/mod.rs:1397-1437` and `tests/cases/t11_structures.rs:101-127`. Read the original planner: primary indexes become `ALTER TABLE ... ADD CONSTRAINT ... PRIMARY KEY`, while non-primary unique source indexes become `CREATE UNIQUE INDEX` (`src/plan/mod.rs:1397-1410`). The adjacent native case inspects PostgreSQL `pg_index` and asserts `indisunique` and ordered key columns (`t11_structures.rs:101-127`).
- Pinned pgloader XERJ query: `project-pgloader "MySQL unique key PostgreSQL CREATE UNIQUE INDEX constraint conversion"`; pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `src/pgsql/pgsql-ddl.lisp:207-263`: a primary index and an index with a constraint definition are separately upgraded through `ALTER TABLE ... ADD CONSTRAINT`; ordinary unique indexes use `CREATE UNIQUE INDEX`. The adjacent Clojure DDL test `clojure/test/pgloader/ddl_test.clj:89-96` expects the unique non-primary definition to contain `UNIQUE` in the generated index SQL.
- Pinned Paganel XERJ query: `ref-paganel "unique index unique constraint PostgreSQL catalog pg_index schema create"`; pin `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`. It returned schema-plan and destination files; the my2pg local planner and native `pg_index` case are the more direct implementation/test match, so no Paganel behavior is being adapted.
- Chosen correction: query `pg_index` for the recreated relation and assert exactly one primary unique index on `id` plus one non-primary unique index on `value`, following `t11_structures.rs:101-127`. Keep the separate obsolete-constraint/index absence checks and all zero-row, sequence, new-OID, and outside-sentinel assertions. This matches emitted DDL instead of requiring an unsupported catalog representation.

## Correction and verification

Updated only this case's index assertion to inspect `pg_index.indisprimary`, `indisunique`, and ordered `indkey` columns. The expected fresh-lane result is two unique indexes: primary=true/unique=true/columns=`[id]`, then primary=false/unique=true/columns=`[value]`. All other structural, zero-row, identity, and outside-sentinel assertions remain.

- Direct pinned-toolchain `rustfmt --edition 2024 --check tests/cases/t11_schema_only_recreate.rs` passed after formatting the test file.
- Focused integration compile: `my2pg/bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration t11_schema_only_recreate --no-run` passed.
- Discovery: the same focused test target with `-- --list` lists exactly `t11_schema_only_recreate::native_schema_only_recreate_replaces_selected_table_without_copying_or_touching_sentinel`.
- Strict static check: `my2pg/bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings` passed.
- `cargo fmt --all -- --check` initially exposed a module-order diff in coordinator-owned `tests/integration.rs`; the coordinator applied workspace formatting before native acceptance.

## Coordinator native follow-up

After formatting module registration, the corrected case passed **1/1** on fresh MySQL 8.4.11/PostgreSQL 16.15 fixture `my2pg-mysql84-pg16-d2b1a5236243`. The exact fixture was stopped. It now runs as a mandatory native acceptance case; the subsequent complete serial MySQL84/PG16 suite also passed (144 total invoked tests, zero failures), with all 13 mandatory native IDs executed exactly once. See [coordinator integration evidence](2026-10-02-T11-T12-test-registration.md).
