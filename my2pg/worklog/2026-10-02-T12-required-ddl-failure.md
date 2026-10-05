# T12 — planned DDL failure must fail the actual run

- Status: required-DDL failure path implemented and natively accepted
- Agent/role: coordinator
- Task and specification links: [T12](../docs/agent-tasks.md#task-board), [failure handling checklist](../docs/checklists.md#m2-reliable-core)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`
- Lease: new `tests/cases/t12_required_ddl.rs`, registration in `tests/integration.rs` and `tests/support/harness.py`, this worklog; no production or shared model edits.

## Reference research before code

| Query | Revision and original files | Finding | Adaptation and proof |
| --- | --- | --- | --- |
| `required DDL failure database error status abort table migration`; `project-my2pg` | Current indexed snapshot `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; read `src/pipeline/mod.rs:2554-2646`, `src/plan/mod.rs:1124-1225,1628-1670`, `tests/contracts.rs:202-213`, `tests/cases/t17_check_override.rs:1-190`, and `tests/cases/t12_runner.rs:1-160` | The runner records each planned object as a failed step before execution, runs each DDL statement in a transaction with a 60-second deadline, rolls back server-reported errors, and returns a `DDL` failure with SQLSTATE but without raw target exception text. CHECK creation is in the Finalize phase after COPY. Existing unit coverage only proves a synthetic `failed_steps` report maps to a nonzero exit; no native end-to-end planned-DDL failure is registered. The T17 test covers an explicit valid target CHECK expression and preflight refusal when a source CHECK has no policy. | Add one isolated native case using a reviewed source CHECK plus an explicit but syntactically invalid target expression. Require actual PostgreSQL `42601`, failed table/run status and durable final report; independently assert that the copied row remains acknowledged, the failed step is the source CHECK, the target table exists without that CHECK, raw target exception text is absent, and exit is nonzero. This tests honest failure after COPY rather than implying earlier committed data is rolled back. It does not endorse invalid overrides. |
| `CREATE INDEX constraint failure required DDL error migration`; `project-pgloader` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; read `clojure/src/pgloader/core.clj:135-149,730-800`, `clojure/test/pgloader/ddl_test.clj:340-390` | The upstream transaction helper rolls back failing DDL, while the table creation phase records a fatal load error and adds the table to a failed set so it is not copied. Nearby DDL tests check generated SQL but do not prove the real PostgreSQL failure path. | Preserve the important rollback/no-copy behavior, but adapt it to the Rust runner's required-step ledger, sanitized SQLSTATE diagnostic and persistent report. No upstream code or test data is copied. |

RTK and Ponytail guidance were applied. This is a test-only increment; the tests intentionally induce a parser error at the PostgreSQL boundary using an otherwise accepted explicit expression.

## Acceptance plan

- Test the real CLI with an owned MySQL/PostgreSQL TLS fixture.
- Assert exit code 1, report status Failed, SQLSTATE `42601`, the planned CHECK in `failed_steps`, and durable `report.json` identical to the final event.
- Since CHECK DDL is finalized after COPY, assert the committed source row remains, the target table exists without the failed CHECK, and the report still marks the run failed rather than complete.
- Assert user-facing output omits raw PostgreSQL exception text and both database URLs/secrets.
- Register this case as required on all supported native MySQL lanes and run it in a fresh serial harness.

## Native acceptance

`rtk run 'python3 tests/support/harness.py mysql84 pg16'` passed on Darwin/ARM64 with MySQL 8.4.11 and PostgreSQL 16.15. The lane passed **153 cases**, with 23 required IDs each observed exactly once, no failed or unmet IDs, and `connections.json` state `stopped`. The new required ID is `t12_required_ddl::native_required_check_ddl_failure_persists_failed_report_after_copy` and is recorded once as `ok`.

- Artifact: `target/integration/my2pg-mysql84-pg16-92362583765a/rust-tests.json`; harness log SHA-256 `406d6489509e23bc5f7c777ddb7edf05f40a589c6770e75e2659e60c2cc0f2de`.
- The native report records run/table `failed`, SQLSTATE `42601`, the exact source CHECK in `failed_steps`, and one committed row. Its `report.json` matches the final CLI report. Independent catalog/data reads show the table and copied row remain while the failed CHECK is absent. User output contains neither the raw parser text nor either connection URL.
- In the same lane, the existing required DDL/report/reject failure and recovery cases also pass: `t12_durable_faults`, `t12_lost_commit_ack`, `t12_recovery`, `t12_runner`, and `t12_uncertain_storage`.

This closes the planned-DDL failure checklist behavior. T12 remains in progress for physical storage-sync fault acceptance and any untested platform/runtime cases.
