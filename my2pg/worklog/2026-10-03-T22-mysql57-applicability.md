# 2026-10-03 — T22 — MySQL 5.7 case applicability audit

### Coordinator integration decision before shared harness edits

- Re-read `docs/reference-coding.md`, `docs/scope-and-compatibility.md`, and `docs/agent-tasks.md`; coordinator owns the shared harness and index. XERJ searches used for this integration were `project-my2pg 'MySQL 5.7 pending tests versioned TLS plugin DEFAULT empty table enum metadata'` at base revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, `ref-mysql_async 'TLS mysql_native_password caching_sha2_password MySQL server version authentication'` at pinned revision `d7525d…`, and `project-pgloader 'MySQL server version enum metadata empty table DEFAULT column TLS authentication'` at pinned revision `231ab…`.
- Read the original assertions/fixtures in `tests/cases/t05_mysql.rs`, `tests/cases/t10_empty_string_default.rs`, `tests/cases/t10_enum_metadata.rs`, and `tests/cases/t10_label_recovery.rs`, their adjacent `#[ignore]` reasons, and the shared manifest in `tests/support/harness.py` plus its exact-inventory tests in `tests/support/test_harness.py`. The task worklog from the case owner records the changed version-specific assertions and compile-only checks.
- Decision: promote the four formerly pending entries to `run` after adapting each assertion to accept its documented MySQL 5.7 behavior and confirming no post-5.7 fixture syntax is used. Preserve the 23 source-feature incompatibilities as `not_applicable`; no test result is inferred from static review. Prove the manifest is exactly 134 runnable, 23 not applicable, and zero pending; run support tests and list-only matrix preflight. Actual MySQL 5.7 execution remains a separate native AMD64/CI gate.

- Status: static compatibility audit integrated; native MySQL 5.7 database execution remains unverified
- Coordinator: I; read-only source audits: T03/T05–T10, T11–T14, and T15–T20 owners
- Base revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; shared working tree contains earlier T22 platform/harness changes
- Scope: classify all 157 feature-enabled ignored case IDs before allowing the native AMD64 MySQL 5.7 lane to start

## Reference research and source review

Read `docs/reference-coding.md`, `docs/agent-tasks.md`, `tests/support/harness.py`, `tests/support/matrix.py`, adjacent support tests, the listed Rust test modules, and the case setup helpers. XERJ was available locally as `v1.0.0-rc.80`; the three coordinator searches below used existing shared/pinned indexes and did not rebuild them:

| Query and corpus | Indexed revision | Finding and adaptation |
| --- | --- | --- |
| `project-my2pg` — `MySQL 5.7 recursive CTE ignored inventory case applicability native test source` | `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | Retrieved the exact T22 inventory/worklog and MySQL-only test sources. Classify from the concrete case SQL/setup and assertions, not broad module names. |
| `ref-paganel` — `mysql server version support integration tests CTE CHECK constraints compatibility` | `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc` | No reusable applicability oracle for this project's suite; use the reference only as corroboration, not as a disposition source. |
| `project-pgloader` — `MySQL version compatibility tests CTE check constraints` | `231ab86778ca5ffd7de40878714760c8b4860cdf` | `src/sources/mysql/mysql-schema.lisp:181` documents CHECK metadata beginning in MySQL 8.0.16. The Lisp/Clojure implementation is not ported; version behavior is checked against local case SQL and MySQL manuals. |

RTK 0.49.0 and Ponytail guidance were used for repository/test review and worklog discipline. The three read-only task audits separately searched project and pinned indexes: their revisions were project `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, `ref-mysql_async` `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`, `ref-paganel` `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`, and `project-pgloader` `231ab86778ca5ffd7de40878714760c8b4860cdf`. Their reviewed source regions are listed in the rationale below.

## Decision

The exact inventory has 157 cases. Record 23 `not_applicable` cases with source-feature reasons and 134 `run` candidates. `run` means eligible to execute in a future native 5.7 lane, not proven compatible. The pending disposition remains supported and fails closed, but no current case is pending. No MySQL 5.7 database or container was started for this audit.

### MySQL 5.7 not applicable (23)

- `mysql_owned_stream_backpressure_and_cancellation` — nonrecursive CTE in `tests/driver_spike.rs:268` (MySQL 8.0+).
- `pipeline::worker_fault_tests::native_runner_panic_preserves_registered_unread_source_backend` — recursive CTE setup in `src/pipeline/mod.rs:3182`.
- `t05_mysql::abandoned_reader_guard_closes_without_draining` and `t05_mysql::reader_drop_during_panic_and_outside_runtime_closes_promptly` — recursive CTEs in `tests/cases/t05_mysql.rs:325,381`.
- `t05_mysql::catalog_preserves_order_and_original_metadata` — invisible columns and enforced CHECK metadata in `tests/cases/t05_mysql.rs:62,111-121`.
- `t10_binary_default::native_binary_defaults_recover_actual_bytes_without_source_rows_or_writes` — expression-default guard in `tests/cases/t10_binary_default.rs:49-54` (MySQL 8.0.13+).
- `t10_label_recovery::native_recursive_cte_widens_enum_set_and_cannot_recover_labels` — recursive CTE in `tests/cases/t10_label_recovery.rs:143`.
- `t11_structures::native_cli_full_and_schema_only_preserve_renamed_composite_structures` and `t11_existing_policies::native_composite_order_extra_states_and_unknown_graph_fail_before_mutation` — descending source index assertions; MySQL 5.7 accepts but ignores descending key parts (`tests/cases/t11_structures.rs:31,110-124`; `tests/cases/t11_existing_policies.rs:576,585+`).
- `t12_required_ddl::native_required_check_ddl_failure_persists_failed_report_after_copy` and `t17_check_override::native_check_requires_explicit_postgres_expression_and_verifies_it_exactly` — require enforced source CHECK constraints (`tests/cases/t12_required_ddl.rs:37,45-54`; T17 case setup/assertions at `tests/cases/t17_check_override.rs:109-130`).
- Nine T13 cases share `tests/cases/t13_concurrency.rs:61-67`, which sets `cte_max_recursion_depth` and seeds with `INSERT ... WITH RECURSIVE`: `actual_cli_fixed_budget_rss_does_not_scale_with_tenfold_source_rows`; `actual_cli_sigterm_stops_two_blocked_full_queues`; `actual_writer_socket_failure_stops_full_queue_sibling_without_replay`; `concurrent_commit_cancellation_retains_sibling_ack_prefix_without_replay`; `empty_queues_cancel_blocked_source_and_close_reader_writer_pairs`; `exact_one_pipeline_budget_reduces_admission_and_finishes_all_tables`; `full_queues_cancel_two_copy_workers_and_release_all_connections`; `mixed_conversion_copy_rejects_share_one_global_durable_limit`; `source_socket_fault_wakes_empty_consumers_and_stops_sibling`.
- `t17_index_expression::native_functional_index_requires_override_and_preserves_unique_behavior` — functional index requires MySQL 8.0.13+ (`tests/cases/t17_index_expression.rs:101`).
- Both `t17_spatial` cases — shared source fixture inserts `GEOMETRYCOLLECTION EMPTY` (`tests/cases/t17_spatial.rs:101`). The [MySQL 5.7 spatial argument manual](https://dev.mysql.com/doc/refman/5.7/en/spatial-function-argument-handling.html) specifies the legacy input form `GEOMETRYCOLLECTION()`; 8.0 added recognition of the standard `GEOMETRYCOLLECTION EMPTY` spelling.

### Version-adapted cases (4)

- `t05_mysql::production_mysql_tls_mutual_auth_and_policy` branches on `SELECT VERSION()` and expects `mysql_native_password` for 5.7 versus `caching_sha2_password` for 8.0/8.4; unsupported server versions fail explicitly.
- `t10_empty_string_default::native_empty_text_defaults_remain_distinct_without_source_rows_or_writes` no longer evaluates `DEFAULT(column)` over a NULL-extended empty-table row. It retains a populated direct-default control, catalog assertions, read-only/zero-row checks, then verifies actual inserted defaults independently.
- `t10_enum_metadata::supplementary_enum_set_labels_are_indistinguishable_from_question_marks_in_catalogs` expects `@@character_set_system` to be `utf8` on 5.7 and `utf8mb3` on 8.0/8.4, and fails on unreviewed versions. Its dictionary-table query remains an explicit denied-read assertion.
- `t10_label_recovery::native_show_create_and_binary_metadata_lose_unused_labels_in_both_sql_modes` uses no recursive CTE or 8.x-only fixture syntax; its setup uses ENUM/SET values, `SHOW CREATE`, and result-set modes to exercise the server metadata behavior.

### Run candidates (130)

Every other case in `MYSQL_CASE_INVENTORY` is a static `run` candidate. The audit covered the two worker-fault cases and the PostgreSQL-only catalog snapshot as well: the first uses ordinary InnoDB rows and the latter connects only to PostgreSQL (`src/pipeline/mod.rs:3132-3177`; `src/postgres/mod.rs:906-985`). The T06 fixture, driver raw-value/TLS cases, T07–T09, T10 cases without the exclusions above, T11/T12 cases without the exclusions above, eight non-recursive T13 cases, all T14 cases, and T15–T20 cases were source-reviewed. This status does not establish runtime compatibility; the future x64 lane must execute the candidate cases and satisfy required-case checks.

The current harness had incorrectly described both nonrecursive T10 metadata tests as recursive-CTE exclusions. Their assertions and fixture expectations are now version-adapted. The explicit disposition contract assigns `run` to the statically reviewed complement, preserves concrete `not_applicable` reasons, and still requires nonempty evidence for any future pending case. Harness tests check the 157/134/23/0 totals and exact exclusion set.

## Verification and limitations

- `rtk test python3 -m unittest discover -s tests/support -v`: 35 tests passed after using the authorized temporary-file access. Before that, the same tests failed during temporary-directory creation because the execution mount was read-only; no assertion failure was involved.
- `rtk git diff --check`: passed with temporary cache access.
- `rtk test python3 -m unittest discover -s tests/support -v`: 35 tests passed. `python3 -m doctest tests/support/harness.py tests/support/matrix.py`: passed.
- `rtk test ./bin/cargo test --offline --locked --all-targets --all-features -- --test-threads=1`: passed, 178 tests passed and 157 remained ignored (database-backed tests are intentionally not run by this command).
- `rtk test ./bin/cargo fmt --all -- --check`: passed. `rtk git diff --check`: passed.
- `rtk test ./bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings`: passed after the version-aware test edits.
- `rtk test python3 tests/support/matrix.py --check --all`: exit 0; database-free compile/list-only preflight found no pending MySQL 5.7 dispositions and started no database tests. Support tests assert the full 157/134/23/0 inventory; native 5.7 runtime remains unproven.
- No MySQL 5.7 database/image/container or hosted CI workload ran.
- Coordinator XERJ refresh after the changes: `project-my2pg-source`, 415 files, 1495 passages, 14 exclusions. Query `project-my2pg "MySQL 5.7 applicability 157 cases 134 run 23 not applicable pending"` retrieved this audit, the adapted-case notes, and checklist evidence at indexed source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- Native x64 CI and actual MySQL 5.7 execution remain open; collect real source/target results before claiming 5.7 support.

## Process note

XERJ searches, source review, and task ownership review preceded the harness change. The audit worklog was saved in the same integration turn after the initial code patch rather than before it; this sequencing deviation is recorded explicitly. No source code was copied from the read-only references.
