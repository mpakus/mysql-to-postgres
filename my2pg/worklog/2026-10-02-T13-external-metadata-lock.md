# T13 — external metadata-lock cancellation and server disappearance

## Nullable processlist oracle hardening (before code)

- Project XERJ query: `processlist STATE INFO nullable killed connection`,
  prefix `project-my2pg`, index revision
  `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Results point to this test and
  the separate SingleSnapshot processlist observer. The current test assumes
  `STATE` is non-null and decodes it as `String`; a backend can disappear
  between query evaluation and conversion, yielding NULL fields.
- Reference query: `information_schema processlist nullable row Option
  FromRow`, prefix `ref-mysql_async`, pin
  `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`. The driver's `Queryable`
  conversion returns errors/panics for incompatible non-optional `FromRow`
  types; no processlist-specific reference behavior exists.
- Adaptation: decode nullable processlist fields as `Option<String>` and only
  inspect a non-null state. For disappearance polling, select a constant rather
  than nullable metadata fields. This keeps the oracle observational and ensures
  an assertion failure still reaches fixture cleanup; rerun the native case and
  the full required lane.

- Status: focused native external-MDL case accepted and mandatory registration verified. T13 remains in progress for broader SingleSnapshot cancellation/shutdown coverage; the coordinator owns task-board/checklist updates and shared XERJ refresh.
- Owned files: new `tests/cases/t13_external_mdl.rs` and this worklog only.
- Project revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; current XERJ project snapshot: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- Reference pins: `ref-dmt-rs` `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; `ref-mysql_async` `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`; `project-pgloader` `231ab86778ca5ffd7de40878714760c8b4860cdf` (reviewed earlier in the T13 shutdown investigation).

## Reference research before code

Applied the repository RTK/Ponytail workflow. `rtk` is available at `/opt/homebrew/bin/rtk`; the existing T13 shutdown worklog records that no separate Ponytail or RTK `SKILL.md` is present in configured skill roots, so I used the project guidance for the smallest complete independent test. No code or fixture is copied from a reference.

| XERJ query and corpus | Pinned/current evidence read | Finding and adaptation | Acceptance oracle |
| --- | --- | --- | --- |
| `project-my2pg`: `T13 external metadata lock server shutdown migration cancellation` and `external metadata lock KILL CONNECTION exact backend` | Current project index at `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `src/pipeline/mod.rs:923-950,1288-1345,1520-1533`; `tests/cases/t13_concurrency.rs:194-201,213-280`; `worklog/2026-10-01-T13-single-snapshot-shutdown.md` | The Frozen runner holds its worker's exact MySQL connection ID until the coordinator sends `KILL CONNECTION` over its separate source control connection. Existing queue coverage locks tables before readers start and tests cancellation/faults, but it does not assert disappearance while the external MDL holder remains connected. | Gate the runner after source preflight at actual target DDL, acquire a fixture-owned MySQL `LOCK TABLES ... WRITE`, then release DDL and observe the app's exact table SELECT in the metadata-lock wait state. Signal normal run cancellation. Require the run to report cancelled with no copied rows and the exact worker ID absent while the lock holder session is still live. |
| `ref-dmt-rs`: `server termination shutdown cancellation bounded channel signal checks` | Pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; read `scripts/test-signals.sh:37-73` and `crates/dmt-rs/src/transfer/mod.rs:413-428` | The reference has a process-signal smoke test and bounded queues, but no evidence for killing a MySQL session blocked on external MDL. Adapt only the bounded, observed-phase testing idea; use my2pg's own shutdown contracts and real MySQL server observations. | Use explicit deadlines around phase observation, run completion, and server-thread disappearance; never infer server cleanup from client task completion alone. |
| `ref-mysql_async`: `drop connection active query transaction cancellation` | Pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`; read `src/queryable/transaction.rs:145-205`, including transaction start/drop/commit behavior | The driver wraps SQL transaction commands and marks uncommitted state for cleanup; that does not establish completion of an external MDL-blocked server command after dropping a client future. | Keep an independent root monitor and assert the registered worker connection itself disappears before releasing the external lock. |

The first unprivileged local-XERJ attempt was denied loopback access by the sandbox. The required read-only XERJ searches succeeded through the approved execution path; no index was rebuilt. `project-pgloader` was already searched for T13 snapshot/shutdown behavior in the referenced prior worklog; it supplied no stronger external-MDL cancellation oracle, and no pgloader code is used.

## Chosen case

The test owns one InnoDB table, one PostgreSQL schema, a target event-trigger gate, a source `LOCK TABLES` holder, and the migration run. It runs full migration in Frozen mode with one table worker. A target `CREATE TABLE` event-trigger advisory barrier guarantees source metadata preflight has finished before the test takes the MySQL table write lock. After releasing that barrier, the table worker's actual SELECT must become visible in `information_schema.processlist` with a metadata-lock wait state. The test sends the ordinary watch cancellation, then requires bounded report completion, exit 130, zero row acknowledgments, the persisted report to agree, and disappearance of the exact blocked worker connection while the lock-holder thread is still present. It unlocks only after observing disappearance, then drops the owned event trigger/function, source table, target schema, and connections.

This proves the Frozen control-connection cancellation path against one real external metadata lock on the pinned development pair. It does not prove client-drop cleanup alone, SingleSnapshot interruptibility, multiple simultaneous MDL waits, every MySQL release, or synchronous filesystem shutdown. Those remain open gates unless this runtime experiment and coordinator review resolve them.

## Nullable processlist oracle fix and refreshed evidence

The latest full lane exposed that MySQL may return nullable `STATE`/`INFO` for a
backend disappearing during observation. The test now accepts optional fields
and polls for `ID` only when checking disappearance. This prevented an oracle
panic that had skipped fixture cleanup and left an event trigger to contaminate
the following T16 test.

The refreshed MySQL 8.4.11/PostgreSQL 16.15 lane passed 156 cases and all 26
applicable required IDs after the fix; see
[the reviewed-source T22 evidence](2026-10-02-T22-mysql84-pg16-reviewed-source-refresh.md).

## Implementation and focused acceptance

Added `tests/cases/t13_external_mdl.rs`. The coordinator registered the module in `tests/integration.rs`. The case uses a target event trigger to stop the full migration after source preflight, then takes an actual MySQL `LOCK TABLES ... WRITE` lock before allowing target DDL to finish and the reader to start. It observes the actual migration user's SELECT in `information_schema.processlist` with a metadata-lock wait state, sends the regular watch cancellation, and polls the exact observed connection ID to disappearance. The fixture lock holder must still be present at that point. The saved report must be cancelled with no rows read/committed, indeterminate work, unresolved work, or failed steps. The test releases its event trigger and table lock and drops its unique source/target fixture objects before closing the connections.

The first focused run exposed a test-oracle issue: a live idle MySQL lock-holder processlist row has `INFO = NULL`. The test decoder initially required a non-null string and panicked while checking that the lock-holder remained alive. That pair was stopped immediately; the decoder now models `INFO` as optional. Cleanup now releases the blockers and owned objects before surfacing report or observation assertion failures.

| Check | Exact command/artifact | Result |
| --- | --- | --- |
| Formatting and compile | `bin/cargo fmt --all`; `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration --no-run` | Passed after the NULL-safe monitor correction. |
| Focused native test | `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration t13_external_mdl::cancellation_kills_reader_waiting_on_external_mysql_metadata_lock -- --ignored --exact --nocapture --test-threads=1` with the generated harness environment | Final test file passed 1/1 in 1.70s on Linux/arm64, MySQL 8.4.11 to PostgreSQL 16.15. Report: cancelled, exit 130, zero rows read/committed, zero indeterminate/unresolved rows, no failed steps. The exact reader backend disappeared within the bounded assertion while the separate lock-holder backend remained live. The test then successfully checked each owned fixture-drop, unlock and connection-close result. |
| Durable test artifact | `target/integration/my2pg-mysql84-pg16-f13e00000003/t13mdl_34008_0/run-18dad1a30ad41c88-84d8-0/report.json` | SHA-256 `10856e0a9305168a28af78b5022bd5f0d7f14e8a020f15737b1c7f8cc93b3fa5`; final report independently inspected: run/table cancelled, zero rows read/committed, zero indeterminate/unresolved rows, zero failed steps. `plan.json` SHA-256 `027fc662b9c3d7e856814e231fb510438fce4cc7742921ef053a282c5500f4dd`. |
| Harness cleanup | Project `my2pg-mysql84-pg16-f13e00000003`, metadata `target/integration/my2pg-mysql84-pg16-f13e00000003/connections.json` | MySQL 8.4.11/PostgreSQL 16.15 pair is verified `stopped`; exact project was stopped with `python3 tests/support/harness.py --stop .../connections.json`. Its `containers.log` SHA-256 is `8cdd7c3a61618fc824e5f5db05ac3f4a80ba3c31acf1e1f22a560544ef953d2d`. Both earlier projects (`my2pg-mysql84-pg16-9c1c86695959` and Docker-socket-denied `my2pg-mysql84-pg16-38aa73f8e97b`) are also verified stopped. |
| Final local checks | `bin/cargo fmt --all -- --check`; `bin/cargo clippy --offline --locked --features artifact-worker-tests,native-import-tests --test integration -- -D warnings`; `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration --no-run`; `git diff --check` | All passed after the final cleanup-result checks were added. |

The coordinator then ran the full serial `mysql84 pg16` harness. The final version of this case was discovered and passed exactly once among 154 passing native cases, with all 24 applicable required IDs satisfied; see [the refreshed T22 lane evidence](2026-10-02-T22-mysql84-pg16-t13-refresh.md). At that time T13 remained open for SingleSnapshot and broader shutdown edges. Those focused SingleSnapshot tests and the real stalled-filesystem shutdown case have since passed; see the [SingleSnapshot closeout](2026-10-02-T13-single-snapshot-control-cancel.md) and [stalled-filesystem closeout](2026-10-02-T13-stalled-filesystem-shutdown.md).

This is evidence for external-MDL cancellation and exact MySQL worker disappearance in Frozen mode on the one tested server pair. It does not close SingleSnapshot interruption, all shutdown phases, platform/version matrix, or the broader T13 task. The parent owns task/checklist updates and the shared XERJ rebuild. No source code or reference files were modified.
