# 2026-10-02 — T13 — SingleSnapshot metadata-lock registration

Status: registration complete; native compile and execution pending coordinator verification.

## Reference research (recorded before registration edits)

- Project HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; shared project XERJ index revision is the same. The T13 worker's new case is not yet in that index, so it was read directly.
- Project query: `project-my2pg "SingleSnapshot external metadata lock cancellation later table" -k 6 --full 60`. Hits included the prior external-MDL acceptance worklog, the existing SingleSnapshot later-table test and shutdown worklog. Their originals were opened below.
- Pinned query: `ref-mysql_async "query result cancellation drop stream" -k 5 --full 60`, pinned revision `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`; `ref-dmt-rs "cancellation bounded producer signal" -k 5 --full 60`, pinned revision `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`.
- Read `references/mysql_async/src/queryable/query_result/result_set_stream.rs:220-375` and `queryable/query_result/tests.rs:90-205`: dropping/consuming result streams is an ownership and cleanup concern, not evidence that an already executing server command can be interrupted. Read `references/dmt-rs/scripts/test-signals.sh` and its CLI signal handler as search context; generic SIGINT handling does not prove one blocked MySQL command is cancelled.
- Read `tests/cases/t13_external_mdl.rs:195-328`: existing external-MDL coverage observes the exact blocked Frozen worker and asserts its backend disappears after cancellation. Read `tests/cases/t13_snapshot_consistency.rs:102-233` and new `tests/cases/t13_snapshot_mdl.rs:1-380`: SingleSnapshot reuses the original source session and the new case explicitly characterizes the presently uninterruptible blocked SELECT, releases its own lock, then requires cancelled completion and zero acknowledged work. It does not claim server-side interruption while held.
- Read `tests/integration.rs:1-95`, `tests/support/harness.py:18-38`, and `tests/support/test_harness.py:10-36` for the exact module path, platform-independent mandatory registry and registry self-test. The worklog preceding the worker implementation is `worklog/2026-10-02-T13-single-snapshot-mdl.md`.

## Chosen adaptation and verification

Register `t13_snapshot_mdl` as an explicit path module in the ignored native integration target and add its full test ID to the harness's required list and generic registry assertion. This follows the existing T13 external-MDL structure and makes omissions fail preflight on both supported host systems. Verify the pure registry tests and native ignored-case discovery/compile; a real native run additionally requires the owned MySQL 8.4/PostgreSQL 16 fixture.

The first integration-target compile exposed that moving the fixture's credentials into the spawned runner partially moved `Case` while its observer borrowed the fixture. The regression now resolves a separate credential value from the same config for the spawned task and does not retain a redundant final `RunReport` binding. This is a test ownership correction.

## Live-test diagnosis before assertion revision

- The first fresh MySQL 8.4/PostgreSQL 16 lane built successfully and ran 154 accepted cases, but the new case failed. Saved evidence: `target/integration/my2pg-mysql84-pg16-0a701e45d3d0/rust-tests.json`, suite log SHA-256 `a9a899b3e44ad6782aabc6bc6e8075835447c54ac14a9eb8434c744265427cfa`; harness stopped the exact project.
- Failure: the assertion rejected the expected `run_finished == true` state while the exact MySQL connection ID still reported a metadata-lock wait. The report already showed cancellation (exit 130). The test failed while unwrapping that observation error, not the cancellation report checks.
- XERJ queries against the rebuilt project/pinned indexes: `project-my2pg "SingleSnapshot table_stream cancellation table_stream query cancellation result"`; `ref-mysql_async "Drop stream query result cancel connection drop"`. Read originals at `src/pipeline/mod.rs:2150-2190,2210-2240`, `src/mysql/mod.rs:36-52,737-750`, `references/mysql_async/src/conn/pool/mod.rs:360-378`, and `references/mysql_async/src/queryable/query_result/result_set_stream.rs:145-205`.
- A parallel source audit read `src/pipeline/mod.rs:1257-1280,2172-2180`, `src/mysql/mod.rs:588-594,741-750`, and `references/mysql_async/src/conn/pool/mod.rs:360-378`. `table_pipeline` selects cancellation and drops its producer future; the outer runner then closes its sole shared SingleSnapshot source socket. There is no registered source ID/control connection for this mode, so MySQL retains the blocked server statement after the client run returns.
- The independent diagnosis read the saved native evidence: `rust-tests.log:966-967` reports the assertion failure; the case had already observed the same ID still in `Waiting for table metadata lock`; its report says cancelled with zero rows and `elapsed_millis=59`.
- Adaptation: characterize this shutdown limitation explicitly. Require the runner to finish cancelled while repeated fresh observer samples still show the same metadata-lock wait and the lock-holder remains alive. Release only the fixture-owned lock, then poll until that exact server thread disappears before fixture cleanup. Do not treat client future completion as server-command termination.
