# 2026-10-02 — T11/T12 — coordinator test registration

Status: coordinator registration and validation complete; broader T11/T12 task gates remain open.

## Reference research before edits

- Read `docs/reference-coding.md`, coordinator ownership and native-lane policy in `docs/agent-tasks.md`, and current `tests/integration.rs`, `tests/support/harness.py`, `tests/support/matrix.py`, and their adjacent Python acceptance tests.
- XERJ query: `project-my2pg "REQUIRED_CASES native integration registration schema_only recreate data_only on_existing explicit append" -k 8 --full 120`; index revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Results identified the T11 policy audit, config validation, T11 worklogs, and current native acceptance machinery. Read the current originals directly because the indexed snapshot predates the concurrent new cases.
- Pinned reference query: `project-pgloader "schema only recreate target existing table drop" -k 5 --full 90`; pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. Search was inconclusive for this edge. The new T11 case worklog records a more targeted search and direct reads of `src/load/migrate-database.lisp:15-105`, T11 policy tests, and the alternative Rust destination planner.
- The T11 case's own worklog records its schema-only/recreate queries, pinned source reads, selected-table/sentinel oracle, and exact unregistered module boundary. The T12 case's worklog records protocol/COPY/commit source and adjacent test reads, the deterministic lost-ack test, and its fresh native MySQL 8.4.11/PostgreSQL 16.15 evidence.
- RTK was used for local searches and reads. Ponytail guidance/executable remains unavailable in the checked roots; no execution is claimed. The coordinator changes are limited to root module registration, native required-case policy/tests, this worklog, and the task board/checklist after evidence is reviewed.

## Adaptation and proof

Register the two isolated acceptance modules without changing production behavior. Require the lost-commit-ack case in every native harness lane because it runs on supported host platforms and has a deterministic fresh MySQL/PostgreSQL test pair; matrix discovery must therefore also discover it. Keep schema-only recreate discoverable but not yet mandatory until its runtime case passes a coordinator-owned fresh lane. Separately, validate the explicit data-only policy with the full offline suite and a `check`-level refusal before target access. Update task/checklist prose only to the evidenced states and leave broader T11/T12 acceptance open.

## Integration and validation

- Registered `t11_schema_only_recreate` in `tests/integration.rs`; `t12_lost_commit_ack` was already registered. Added the lost-ack test to platform-generic mandatory native cases and its platform-policy unit test. Updated the mode scope and checklist with bounded claims.
- `bin/cargo test --offline --locked` passed: 107 library, 15 contract, 13 integration-unit, and 18 importer tests; 125 ignored native cases were correctly not run. An initial sandboxed run failed three existing loopback tests at socket creation (`PermissionDenied`); rerun with the authorized loopback capability passed all 13 non-ignored integration tests.
- `python3 -m unittest discover -s tests/support -p 'test_*.py'` passed 19 tests.
- `python3 tests/support/matrix.py --check --all` performed offline compilation/list discovery and exited 1 for the already documented missing `mysql57` and `mysql80` pins. Database tests were not run by this preflight.
- Fresh owned lane `my2pg-mysql84-pg16-2912c263fbbc` ran the full serial native suite: 129 passed, one failed, zero ignored among invoked tests. Every mandatory ID passed exactly once, including `t12_lost_commit_ack::completed_commit_with_dropped_ack_is_indeterminate_and_never_replayed`; `rust-tests.json` reports no unmet mandatory IDs. The new schema-only test alone failed because its assertion expected the source `UNIQUE KEY` to appear in `pg_constraint`; the existing mapping emits it as a unique index. This is a test-oracle defect, not a production failure. The exact test artifact and assertion are retained under `target/integration/my2pg-mysql84-pg16-2912c263fbbc/`. A focused correction and fresh-lane rerun are pending; T11 remains open until then.
- The first harness run stopped its exact owned fixture pair after the run; its 129-pass/one-oracle-failure result is retained as diagnostic evidence, not final acceptance.
- Follow-up: the worker corrected the T11 target-key oracle to inspect `pg_index` before rerun. The focused test then passed **1/1** on a fresh owned MySQL 8.4.11/PostgreSQL 16.15 pair (`my2pg-mysql84-pg16-d2b1a5236243`); its exact owned fixture was stopped successfully. The relation's primary and unique indexes, zero copied rows, generated identity state, recreated OID, and outside sentinel are all asserted.
- With that runtime proof in hand, promoted `t11_schema_only_recreate::native_schema_only_recreate_replaces_selected_table_without_copying_or_touching_sentinel` into generic `REQUIRED_CASES`. The final full harness run executed it exactly once alongside the mandatory T12 lost-ack proof.

## Final coordinator acceptance

- `bin/cargo fmt --all -- --check` passed.
- `bin/cargo test --offline --locked` passed; library, contracts, importer and 13 non-ignored integration tests passed, with 125 native cases ignored in the ordinary suite.
- `bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings` passed.
- `python3 -m unittest discover -s tests/support -p 'test_*.py'` passed 19 tests.
- `tests/run-integration.sh mysql84 pg16` passed **144 invoked tests, zero failures** on Darwin/ARM64 against pinned MySQL 8.4.11 and PostgreSQL 16.15. All **13 mandatory IDs** executed exactly once with `ok`, including schema-only recreate, lost-COMMIT-ack and the platform-required uncertain-storage case. Harness evidence: `target/integration/my2pg-mysql84-pg16-9aa50734f97c/rust-tests.json`; fixture metadata reports `state=stopped`.
- `tests/support/matrix.py --check --all` still reports missing MySQL 5.7 and 8.0 pins; it performs list-only discovery and does not certify those database lanes.
- Remaining limits: the native full pass covers this pinned host pair only. It does not close full T11/T12 acceptance, physical sync failure, or the missing source-version matrix lanes.
