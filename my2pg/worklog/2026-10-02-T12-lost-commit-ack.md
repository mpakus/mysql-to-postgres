# T12 — commit completed by PostgreSQL but acknowledgement dropped

Status: scoped new test/worklog only; no production recovery or shared harness changes. Goal is a deterministic proof for the still-open case “after COMMIT is sent and the server commits, the acknowledgement is lost.”

## Reference research before test code

Applied RTK (`rtk run` wrapper used for project searches and source reads). Ponytail guidance is not installed/discoverable in the configured skill roots or available tool catalog; this limitation is recorded rather than claimed as applied. Current XERJ project index revision returned by search is `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; current working files may be newer, so the originals below govern. No shared index was refreshed.

XERJ queries and pinned references:

- `project-my2pg`: `lost COMMIT acknowledgement physical sync failure T12 checklist` → `worklog/2026-10-02-T12-uncertain-storage.md`, `docs/agent-tasks.md`, `worklog/2026-10-01-T12-durable-faults.md`, and `worklog/2026-10-01-T07-postgres-transport.md` at project-index revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. The existing proof cancels a lock-blocked COMMIT and then encounters real ENOSPC while persisting the report; the independently observed server transaction rolls back. It explicitly does not prove server-committed/lost-ACK or physical fsync failure.
- `ref-rust-postgres`: `commit response lost connection COMMIT transaction` → `tokio-postgres/src/transaction.rs`, `tokio-postgres/tests/test/main.rs`, and related transaction files at pin `1084ca8f5b5302e161892f2fa40abf71b4060c10`.
- `ref-rust-postgres`: `copy_in_error` (`--symbol`) → `tokio-postgres/tests/test/main.rs:721` at the same pin; the test drops an in-progress COPY and proves no rows appear, but does not cover a completed server commit whose result is hidden from the client.
- `ref-rust-postgres`: `CommandComplete CommandTag C protocol COPY COMMIT` → pinned protocol tags/parser and related client/COPY tests at `1084ca8f5b5302e161892f2fa40abf71b4060c10`.
- `project-my2pg`: `CommitAttempt FailureKind Indeterminate copy_bytes` → current `src/postgres/mod.rs:61,121,841` and adjacent T07/T12 tests at the same project-index revision.
- `project-pgloader`: `commit acknowledgement COPY transaction retry` → `clojure/src/pgloader/batch.clj:181` and `src/pg-copy/copy-rows-in-batch.lisp` at pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. These legacy retry paths do not define an acceptable lost-ACK policy for my2pg.

Read the current originals: `src/postgres/mod.rs:788-875` (`copy_bytes` stages and accounting boundary), `tests/cases/t12_uncertain_storage.rs` (actual sent-COMMIT interruption), `tests/cases/t12_runner.rs:232-407` (COPY/COMMIT cancellation assertions), and `tests/cases/t12_durable_faults.rs:107-150,260-370` (physical ENOSPC and separate descriptor `fsync(EINVAL)` evidence). Read pinned `references/rust-postgres/tokio-postgres/src/transaction.rs:49-63`: `commit` marks its object done and awaits `batch_execute("COMMIT")`; `postgres-protocol/src/message/backend.rs:20-30` defines the backend `CommandComplete` tag; its test `copy_in_error` at `tests/test/main.rs:721-750` tests COPY abort only. Also read current `TargetConfig` TLS mode and `connect_initialized` (`src/config/mod.rs:45-75`, `src/postgres/mod.rs:357-500`): `tls_mode=disable` is explicit and selects `NoTls`; harness endpoints are loopback. No code or fixture was copied.

Chosen test-only adaptation: run the actual CLI against an owned PostgreSQL fixture through a short-lived, loopback-only plaintext PostgreSQL protocol proxy (`tls_mode=disable` only in this disposable test config). Forward complete length-framed messages. When the backend emits `CommandComplete("COMMIT")`, treat that as the independent boundary proving PostgreSQL completed the COMMIT, then close both proxy sockets without forwarding the acknowledgement or `ReadyForQuery`. The separate direct fixture connection must observe the committed source rows. The CLI must exit nonzero with exact `Indeterminate` status, zero acknowledged rows for the active batch, and all active rows classified indeterminate; a keyless target must contain the source multiset exactly once, proving no retry/replay. No production hook, shared runner change, fabricated SQL error, forced rollback, or TLS downgrade outside this generated local-only test connection is introduced.

Acceptance test: new ignored database case in a new T12-owned test module, to be registered by the coordinator. It must fail unless the proxy observes COMMIT completion, must confirm CLI error/report status and independent target contents, and must shut down the proxy and drop only uniquely owned source/target objects. Run compile-only first; database execution requires a fresh coordinator-approved owned fixture. This case addresses only lost COMMIT acknowledgement after commit. Physical storage-sync failure remains distinct and open.

## First compile/native diagnostic and correction

Coordinator registered the new module in `tests/integration.rs`. The focused compile passed with `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration t12_lost_commit_ack --no-run`. The first native attempt exited at PostgreSQL connection validation (status 2) before any report/table setup. Two subsequent diagnostic attempts showed only the proxy's 57-byte frontend startup packet and no backend response; the owned MySQL84/PostgreSQL16 fixture was stopped after the attempts. No migration or lost-ACK assertion was reached.

Root cause from local task/await inspection: the async ignored test used blocking `std::process::Command::output()`. `#[tokio::test]` defaults to the current-thread runtime, so waiting synchronously for the CLI's 10-second target-connect deadline starved the spawned proxy task. The CLI had sent startup while the proxy was not polled; after the child timed out and returned, the proxy consumed the buffered startup and saw EOF. Corrected the test harness to use `tokio::process::Command::output().await`, allowing proxy and CLI to make progress concurrently. This was test-infrastructure repair, not a production failure. Temporary frame tracing was removed; stdout/stderr are retained under the private test artifact directory.

## Corrected native acceptance

After converting CLI invocation to async process wait, compile-only and strict Clippy passed:

```sh
bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration t12_lost_commit_ack --no-run
bin/cargo clippy --offline --locked --features artifact-worker-tests,native-import-tests --test integration -- -D warnings
```

The focused ignored native case passed **1/1 in 0.72s** on a fresh owned MySQL `8.4.11` / PostgreSQL `16.15` fixture (`my2pg-mysql84-pg16-0af8431756a3`). The fixture was stopped through its exact `connections.json`; retained metadata reports `state=stopped`. The test evidence directory is `target/integration/my2pg-mysql84-pg16-0af8431756a3/t12_lost_ack_93834_18dabcb57ebcc788/`:

- `proxy-proof.json` SHA-256 `b663d072f0eb3efc9614b42a98180366b30fd819c2141fba4984676b003bd44a` records backend `COPY 2`, backend `COMMIT`, and confirms neither the COMMIT acknowledgement nor following ReadyForQuery was forwarded to the CLI.
- The durable `report.json` records `status=indeterminate`, `rows_read=2`, `committed_rows=0`, and `indeterminate_rows=2`. The CLI exits `1`; stdout/stderr are retained beside the proof.
- An independent target connection observed exactly `(1, one)` and `(2, two)`, so the server completed the commit while the client did not count it, and no retry/replay added duplicates. The test dropped its uniquely named source table and target schema before teardown.

The final focused case source SHA-256 is `a0a24d72d2758b7ac87d358efcb0d28da549f7382fc438fd1c8b514d164162d1`. This closes only the native lost-ACK case on this MySQL84/PostgreSQL16 host pair. It does not prove physical fsync failure, other server versions/platforms, or full T12 acceptance. The coordinator registered the case and made it a platform-generic mandatory harness result; the complete native suite later passed with it exactly once. See [coordinator integration evidence](2026-10-02-T11-T12-test-registration.md).
