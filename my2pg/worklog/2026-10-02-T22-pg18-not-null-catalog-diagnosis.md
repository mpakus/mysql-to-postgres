# T22 — diagnose PG18 DataOnly constraint preflight failures

- Status: read-only diagnosis; no source, test, harness, image, task-board, or shared-index changes.
- Runtime: saved lane `target/integration/my2pg-mysql84-pg18-98d457d19752`; raw output `/private/tmp/my2pg-T22-pg18-native.stdout` and `.stderr`.

## Evidence and cause

Both failing tests completed their schema-only setup and report publication, then panicked at `src/pipeline/mod.rs:2291` while unwrapping `plan(&config, &credentials)` for the subsequent DataOnly + Append operation. The panic reports four `TARGET_CONSTRAINT_UNSUPPORTED` codes before either worker-fault scenario starts. Harness logs record actual MySQL 8.4.11 and PostgreSQL 18.6. The unrelated native target-catalog repeatable-snapshot test passed.

The saved schema-only `plan.json` files identify two tables per run. Each has two non-null columns (`id`, `value`); the generated DDL creates those columns `NOT NULL` and adds a primary key on `id`. Schema-only ran against absent destination tables, so it did not bind preexisting target constraints. The following DataOnly plan inspects the now-created PG18 tables: `pipeline::plan` calls `inspect` then `build`, and `plan/existing.rs` calls `bind_constraints` for each existing table.

The catalog query in `src/postgres/observed_constraints.sql` returns every `pg_constraint` row and places `con.contype` in the observed `kind` field. `plan/existing.rs::bind_constraints` accepts table kinds `p`, `u`, `f`, and `c` (with separate handling for `t` and deferred keys); any other kind emits `TARGET_CONSTRAINT_UNSUPPORTED`. PostgreSQL 18 newly stores column `NOT NULL` specifications in `pg_constraint`, with `contype = 'n'`. Its official release notes describe this change, and the PG18 catalog docs list `n` as a not-null kind; PostgreSQL 17's catalog docs say relation NOT NULL state was represented in `pg_attribute` and `n` applied to domains only. Therefore the four new rows are the two non-null column constraints on each of the two existing target tables. Each hits the planner's unknown-kind guard, explaining the exact count of four.

The cause is strongly established by the runtime count, generated schemas, exhaustive catalog query, finite planner classifier, and PG18 catalog change. The stopped fixture did not preserve raw observed `TargetConstraintPart` rows, so the four constraint names/OIDs were not captured independently. A read-only confirmation on a fresh PG18 reproduction is:

```sql
SELECT n.nspname, t.relname, con.conname, con.contype, a.attname
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class t ON t.oid = con.conrelid
JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
LEFT JOIN LATERAL pg_catalog.unnest(con.conkey) WITH ORDINALITY k(attnum, ordinal) ON true
LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
WHERE n.nspname = $1 AND t.relname IN ($2, $3)
ORDER BY t.relname, con.contype, con.conname;
```

This identifies the compatibility work: explicitly classify/validate relation `n` rows as the equivalent of observed `ColumnPlan.nullable == false`, rather than treating these expected catalog rows as unsupported. Preserve rejection for unsupported constraint kinds and unvalidated/deferrable semantics.

## Reference research and source reads

- XERJ `project-my2pg` query `T13 panic DataOnly TARGET_CONSTRAINT_UNSUPPORTED target constraints`: snapshot `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; returned panic worklogs and target-constraint tests.
- XERJ `project-my2pg` query `TARGET_CONSTRAINT_UNSUPPORTED`: same snapshot; returned `src/plan/existing.rs` and tests for unsupported target constraints.
- XERJ `project-my2pg` query `target constraint unsupported exclusion constraint validation`: same snapshot; returned verifier/planner coverage including existing-policy tests.
- XERJ `ref-paganel` query `postgres target constraints catalog introspection check constraint supported`: pinned commit `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`; its results were not needed to explain this diagnostic.
- XERJ `project-pgloader` query `postgres constraint target table foreign key catalog`: pinned commit `231ab86778ca5ffd7de40878714760c8b4860cdf`; it did not surface the PG18 catalog change.
- Read local originals: `src/postgres/observed_constraints.sql`; `src/postgres/observed.rs` constraint decoding; `src/plan/existing.rs::bind_constraints`; `src/pipeline/mod.rs::plan`; and the failing setup in `src/pipeline/mod.rs::worker_fault_tests::native_runner_panic`.
- Adjacent tests read: `tests/cases/t11_existing_policies.rs` (a `NOT VALID` extra CHECK must remain unsupported); `tests/cases/t11_deferred_keys.rs` (supported linked deferred keys are retained while incompatible required-key policy is reported); and the T13 panic worker tests. These show the unknown/unvalidated guard is intentional; PG18's newly cataloged, ordinary not-null rows need a precise exception/representation.
- Official PostgreSQL evidence: [18 release notes, Constraints](https://www.postgresql.org/docs/18/release-18.html#RELEASE-18-2-1-2) state that column `NOT NULL` specifications are now stored in `pg_constraint`; [18 `pg_constraint`](https://www.postgresql.org/docs/18/catalog-pg-constraint.html) documents table not-null rows and `contype='n'`; [17 `pg_constraint`](https://www.postgresql.org/docs/17/catalog-pg-constraint.html) says relation not-null state is in `pg_attribute` and `n` applies to domains only.

## Run artifact accounting

`rust-tests.json` records one pass and exactly the two worker-panic failures; the latter stopped before their intended panic path. `rust-tests.log` records 1/3 filtered library cases passed. No integration/driver suites ran because Cargo stopped at the failed library target. The schema-only `report.json` files are Complete with two tables each and zero rows, confirming setup, not DataOnly compatibility. Fixture teardown reports stopped state; no rerun or mutation was performed during this diagnosis.
