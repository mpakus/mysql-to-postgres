# T07 — PostgreSQL connection, catalog and COPY transactions

Owner: transport agent E (reused after T03/T05 handoff). Implemented files: `src/postgres/`, `tests/cases/t07*`, this worklog. Coordinator owns shared contracts/root wiring/Cargo; D owns conversion/planning; B owns artifacts/reject persistence. Pipeline ownership belongs to the renewed F lease. No more edits to MySQL without coordinator reassignment.

## Reference research — before coding

Re-read shared model/config and architecture COPY transactions/recovery/shutdown, AUTH/COPY/COMMIT acceptance cases. RTK and Ponytail remain active.

XERJ `project-my2pg` query `COPY transactions uncertain commit` returned architecture and exit-priority/task contracts. `ref-rust-postgres` symbol searches `commit`, `copy_in_error`, `cancel_query` returned pinned `1084ca8f5b5302e161892f2fa40abf71b4060c10`. Read original `tokio-postgres/src/transaction.rs:17–79`, `src/copy_in.rs:75–155`, `src/cancel_token.rs:18–46`, `src/cancel_query.rs`, and `tests/test/main.rs:739–762`.

Findings/adaptation:

- Transaction Drop enqueues rollback without observing its result. We explicitly observe rollback on known failed batches; a failed rollback stays a failure.
- COPY must be finished and its count checked before commit; returned committed rows require acknowledged COMMIT. Any transport failure after commit begins is conservatively indeterminate and never automatically replayed.
- Server cancellation is racy and has no positive success acknowledgement. Closing the owned connection task bounds resource cleanup; runtime batch stage is visible independently of the COPY future so the coordinator can distinguish earlier cancellation from a sent commit.
- Catalog reads use pg_catalog with exact schema identity and ordered attributes. Planning/session initialization does not execute hooks or DDL. Local native TLS already passed CA/hostname positives/negatives in T03; production connection must use the same verification policy plus optional PEM identity.
- Passfile is explicitly configured, privately readable, uses first matching host/port/database/user with wildcard and backslash escaping, and supplies a missing URL password. Never inherit ambient credentials or driver URL TLS overrides.

Acceptance will use real disposable PostgreSQL16 with qualified odd names, public sentinel isolation, NULL/binary/COPY escaping, finish/commit/rollback semantics, constraints/init failures with safe classification, verified TLS/client identity, catalog/dependency inspection, query cancellation and task cleanup. No test skip will count as requested integration success. PostgreSQL17/18 and fault-injected lost commit acknowledgement remain later full-matrix/recovery gates.

## COPY initialization investigation — before correction

Live missing-table and denied-INSERT startup tests each returned a rollback protocol failure on a fresh connection; ordinary COPY row failures and row-count mismatch rollback succeeded. Diagnostic exposed only `unexpected message from server`, without server payloads or credentials. XERJ `ref-rust-postgres` query `COPY initialization rollback permission denied` returned pinned transaction rollback and cancellation tests; original adjacent `copy_in.rs`, `query.rs:256–268`, `client.rs:630–639`, and `connection.rs` were read, and checked against downloaded tokio-postgres 0.7.18.

The COPY startup request uses `query::encode` including Sync; dropping its sender on startup error makes `CopyInReceiver` emit CopyFail plus another Sync. The later explicit rollback receives an unexpected protocol message. Adaptation: retain the original sanitized COPY failure as the cause of a separately observed fatal rollback failure, abort the owned protocol task, and verify backend disappearance. Do not mask failed rollback or reuse this connection. Preflight privileges/existence avoids expected failures but cannot eliminate races; the runtime path remains fail-closed. No retry of indeterminate commit or failed rollback is permitted. Earlier failed test runs are evidence of the driver limit, not successful acceptance.

## Implementation and runtime handoff

Implemented `src/postgres/mod.rs` and ordered, schema-bound SQL reads (`tables.sql`, `columns.sql`, `enums.sql`, `dependencies.sql`, `table_dependencies.sql`), with runtime cases in `tests/cases/t07_postgres.rs`. No unused pipeline files were created. Shared fields and root wiring were coordinator changes; owned T05/T07 plan literals were updated for those fields.

- `connect(&TargetConfig, resolved_url)` returns a concrete non-cloneable `TargetConnection` with a public driver `Client`. It validates explicit network identity, disallows URL transport/session overrides, supports verified native TLS with CA and paired PEM client identity or explicitly disabled TLS, resolves the configured private passfile without ambient PG credentials, validates session policy before connecting, and binds deterministic session settings. Errors retain only stage, failure class, SQLSTATE and static messages.
- `inspect(&Client, schema)` requires PostgreSQL16/17/18, returns ordered base/partitioned tables and columns, generated/identity flags, enums, extensions, schema/database and per-table privileges including inherited-role ownership and active RLS, exact incoming FK/view dependencies, conservative unknown external dependencies, and every occupied relation name/kind including views/sequences/indexes. Catalog queries bind exact schema identity; generated writes qualify every target name.
- `copy_bytes(&mut TargetConnection, &TablePlan, Bytes, expected_rows)` and `copy_batch(...,&EncodedBatch)` preserve retained byte slices, validate contiguous row offsets, COPY only explicit copyable target columns, finish/count-check, then acknowledge COMMIT before returning committed rows. Row failures and count mismatch explicitly roll back; failed rollback retains the original sanitized cause, aborts the connection and remains fatal. There are no automatic transaction retries or replay after indeterminate COMMIT. `execute_ddl(&Client, sql)` propagates sanitized database failures.
- `stage_handle()` and `shutdown_handle()` operate independently of a mutable COPY future. `ShutdownHandle::cancel()` uses the original TLS policy to send a CancelToken request with a three-second deadline, then aborts the protocol task. Cancellation is marked atomically before that request: entering COMMIT fails if cancellation has already won, preventing stale pre-COMMIT classification when futures run concurrently. A previously entered CommitAttempt remains uncertain until an acknowledged result. CancelToken itself has no server success acknowledgement. Synchronous guard Drop aborts the owned socket task; `close()` drops the client first and observes task completion under a five-second deadline. Drop alone does not promise instantaneous server interruption during an active query.

Runtime environment: Rust1.99.0, tokio-postgres0.7.18/postgres-native-tls0.5.3/native-tls0.2.18, native arm64 OracleMySQL8.4.11/PostgreSQL16.15, pinned fixture metadata in `target/integration/my2pg-mysql84-pg16-dad0a38e415d/connections.json`. No sibling application resources were changed.

| Verification | Actual result |
| --- | --- |
| `rtk run 'my2pg/bin/cargo clippy --locked --all-targets -- -D warnings'` | Passed after atomic cancellation barrier; no warnings. |
| `rtk run 'my2pg/bin/cargo test --locked --lib postgres::tests'` | 2 passed: identifier/passfile escaping and cancellation/commit ordering. |
| Source leased fixture env, then `my2pg/bin/cargo test --locked --test integration t07_postgres -- --ignored --nocapture` | 5 passed, 0 failed. Real acknowledged COPY rows, NULL/bytea/escaped text/generated-column exclusion, duplicate rollback, count mismatch rollback, fatal startup failure preserving42P01 and backend close; restricted-role denied42501 with SELECT-only privileges/activeRLS/incoming FK; view/sequence occupancy; trusted TLS/untrustedCA/URLdowngrade rejection; private passfile; dropped task/backend disappearance; sleeping COPY and deferred-trigger COMMIT canceled independently with preserved stages. |
| QA fresh automatic harness `6b80a6baa497` | Reported20 passed (11 source/driver +9 integration),12 seed checks passed, owned resources cleaned/stopped. Shared worker lease retained. The subsequent atomic barrier was separately checked by strict Clippy, unit tests and the five live T07 cases. |

Limits: PostgreSQL17/18 runtime matrix, production-target mutual certificate authentication/negative hostname combinations, fault-injected lost COMMIT acknowledgement, bisection/reject persistence and full-pipeline global RSS remain their later tasks. The COPY-startup driver protocol limit remains explicit and version-sensitive; it is fatal safely, not claimed as a successful rollback. No T07 skips were counted as runtime success.

Status: T07 local PostgreSQL16 transport gate passes; coordinator owns milestone/checklist acceptance and later matrix/recovery gates.
