# T13 — single-snapshot consistency across later tables

Status: isolated native acceptance case; no production change. Owned files are new `tests/cases/t13_snapshot_consistency.rs`, its registration in `tests/integration.rs`, and this worklog. The integrator owns task-board status and XERJ rebuild.

## Research before code

Applied the repository RTK/Ponytail workflow. XERJ `project-my2pg`, query `single_snapshot later tables source snapshot consistency source read transaction`, returned the contract, the existing verifier snapshot case and T13 single-snapshot shutdown review at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. XERJ `ref-mysql_async`, query `consistent snapshot START TRANSACTION read only`, returned the transaction options/implementation at pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`. XERJ `project-pgloader`, query `MySQL consistent snapshot transaction table reads`, returned loader sources but no stronger multi-table snapshot oracle; no legacy code is adopted.

Read current `src/mysql/mod.rs` snapshot and row-stream functions (`start_snapshot` near lines 560–570; `table_stream` near 580–590), `src/pipeline/mod.rs` preflight snapshot start near lines 296–320, and the adjacent `tests/cases/t15_verify.rs::verification_uses_the_supplied_single_snapshot_instead_of_a_new_connection` case near line 781. Read the pinned `mysql_async` transaction implementation at `references/mysql_async/src/queryable/transaction.rs:124–180`; it sets the isolation level and then starts the transaction with `WITH CONSISTENT SNAPSHOT`. The pinned pgloader reference is read-only source material. No reference implementation or test fixture is copied.

The official [MySQL 8.4 consistent-read documentation](https://dev.mysql.com/doc/refman/8.4/en/innodb-consistent-read.html) states that InnoDB consistent reads in one REPEATABLE READ transaction use the same snapshot, including updates committed after that snapshot. The target lock in this test is only a deterministic barrier; the behavioral oracle is the later table's independently observed old source value in PostgreSQL while the admin MySQL connection sees the newer committed value.

RTK is available at `/opt/homebrew/bin/rtk` and commands use `rtk run`. No separate Ponytail or RTK `SKILL.md` is present in the configured skill roots; use the repository's documented smallest-complete-solution approach and do not install tools for this test-only task.

## Chosen acceptance

Use two InnoDB source tables and a single-snapshot, one-worker production pipeline. First create compatible target tables with schema-only mode. In a fresh data-only append run, a temporary row trigger on the first target table signals when the real migration COPY reaches it and blocks that COPY after preflight. While it is blocked, commit an update to the second MySQL source table, remove the trigger and let the later table load. Require the target to contain the value visible at snapshot start, while an independent source query sees the newer committed value. Also require successful two-table row accounting, exact source/target rows and cleanup. The test does not claim safety for non-InnoDB source tables or SingleSnapshot server-side cancellation under external metadata locks.

## Oracle correction from the first native attempt

The first implementation used an access-exclusive relation lock. Its focused fresh-pair run failed before any data migration: target preflight performs `SELECT count(*)` on the existing table, and PostgreSQL correctly blocked that query on the same lock. The owned pair was stopped by the harness. This was a test-barrier placement error, not evidence about snapshot behavior. A renewed XERJ query `project-my2pg / advisory lock event trigger current_setting application_name wait migration worker` found the established scoped PostgreSQL advisory-lock test barrier in `tests/cases/t13_concurrency.rs:213–248,792–835`; those originals show how to hold a fixture-owned advisory lock and observe actual worker activity without locking catalog reads. The barrier was moved to a temporary row trigger on the first target table. It blocks only the migration's real COPY after preflight while allowing source snapshot setup and target catalog queries to finish; no production code or released configuration is changed.

## Final native result

Focused command on a fresh owned MySQL 8.4.11/PostgreSQL 16.15 pair:

```sh
bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration t13_snapshot_consistency::actual_run_keeps_the_original_source_snapshot_for_later_tables -- --ignored --exact --nocapture --test-threads=1
```

Passed 1/1. Pair `my2pg-mysql84-pg16-1baf2eece16e` was stopped after the run. Schema-only created the two target tables; the subsequent data-only single-snapshot run blocked on the first table's actual COPY. While blocked, the test committed `committed-after-snapshot` to the later source table. The migration completed with exact two-table accounting and loaded `snapshot-value` into the later target table, while the independent source read returned `committed-after-snapshot`. The test removed only its temporary trigger/function and owned fixture objects.

The initial access-exclusive-lock probe failed because it blocked target preflight's count query; it was discarded as an invalid oracle and replaced with the row-trigger barrier above. Final formatting (`bin/cargo fmt --all -- --check`) and strict Clippy (`bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings`) passed. The library suite passed 99 tests with 3 intentionally ignored. The serial ordinary integration suite passed 17 with 120 intentionally ignored in the approved local-network lane; its default sandbox attempt had three loopback-only T14 permission failures, which passed on the rerun. This is focused point-in-time behavior evidence, not full T13 concurrency, external metadata-lock, cancellation, or server-matrix acceptance.
