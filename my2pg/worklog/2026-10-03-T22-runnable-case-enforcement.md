# T22 runnable-case execution enforcement

- Status: implementation in progress
- Owner: coordinator / I, shared native harness
- Base revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, dirty worktree
- Finding: the independent T24 audit observed that the native runner verifies exact pass status only for required-case IDs. Non-required cases classified `run` can be missing or ignored without blocking a lane, so the MySQL 5.7 134-case count is not currently enforced at runtime.

## Reference research before code

- Re-read `docs/reference-coding.md`, T22 task ownership in `docs/agent-tasks.md`, and native lane invariants in `docs/testing-and-performance.md` and `docs/checklists.md`.
- XERJ query: `project-my2pg "mysql_case_plan run disposition every discovered case exactly once integration execution result missing case harness"`, indexed source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It returned `tests/support/harness.py`, the exact-disposition T22 worklog, and earlier required-case registration evidence. No upstream reference implementation is needed: this is an internal runtime-contract gap, and an external matrix cannot define which of this project's test IDs must execute.
- Read the current original runtime path in `tests/support/harness.py` around case planning, `case_results`, required-case checks, result artifact writing, and failure exits; read `tests/support/test_harness.py` exact inventory, mocked run, missing/ignored/failed/duplicate and pending tests, plus `tests/support/test_matrix.py` database-free discovery checks. T24's finding points to the result-check region around lines 729–764.
- Adaptation: after Rust reports complete, require every discovered case whose explicit MySQL disposition is `run` to appear exactly once with status `ok`; continue to require `not_applicable` cases not to appear, reject unclassified results, and preserve the separate required-case subset evidence. Include all runnable-case results in the persisted JSON so later lane auditing can distinguish source-feature exclusion from success. Tests must fail for missing, ignored, failed, and duplicate non-required runnable cases, as well as prove 134/23 dispositions for MySQL 5.7.

### Platform correction identified by independent review (before follow-up edit)

- XERJ query: `project-my2pg "MACOS_REQUIRED_CASES target_os macos t12_uncertain_storage t12_durable_faults native harness platform available cases"`, indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It returned the exact shared inventory and the earlier Darwin-vs-Linux required-discovery contract.
- Re-read `tests/cases/t12_uncertain_storage.rs:1-3`, `tests/cases/t12_durable_faults.rs:1-3`, `tests/support/harness.py` inventory/required tuples, `tests/support/matrix.py` `MACOS_CASES`, and `tests/support/test_matrix.py` Linux/Darwin discovery assertions. Both Rust modules have file-level `#![cfg(target_os = "macos")]`; four IDs are in the MySQL inventory but only exist in a macOS test binary.
- Review finding: the first runtime enforcement counted every MySQL `run` case on Linux, which would fail even though Rust correctly omits these OS-specific tests. The MySQL disposition is independent of host platform, so do not reclassify these as MySQL-incompatible. Keep a separate explicit platform exclusion set, omit its cases from Linux run requirements, persist the omitted IDs/reasons, and verify Darwin still requires all four.

## Implementation and verification

- Changed `tests/support/harness.py`: persists `runnable_case_results` and `unmet_runnable_case_ids`, and fails the lane unless each run-disposition case appears exactly once with `ok`.
- Strengthened `tests/support/test_harness.py`: the mocked successful lane reports all run-disposition IDs, and missing, ignored, failed, or duplicate non-required candidates each fail. Required-case evidence remains independently asserted.
- `rtk test python3 -m unittest discover -s tests/support -v`: 35 passed.
- `rtk test python3 tests/support/matrix.py --check --all`: exit 0; matrix preparation compiled/listed tests and started no databases.
- `rtk test python3 -m doctest tests/support/harness.py tests/support/matrix.py`: passed.
- `rtk test env PYTHONPYCACHEPREFIX=/private/tmp/my2pg-pycache python3 -m py_compile tests/support/harness.py tests/support/test_harness.py tests/support/test_matrix.py`: passed.
- `rtk git diff --check`: passed. No database/containers or hosted job ran.
- Follow-up platform correction: centralized the four file-gated cases (three `t12_durable_faults`, one `t12_uncertain_storage`) as `MACOS_ONLY_CASES`, reused that set in matrix discovery, and added `platform_unavailable_case_reasons` plus `runnable_mysql_case_ids`. Linux evidence records those four omissions as `target_os=macos` platform exclusions and enforces all remaining run cases; Darwin enforces the full run set.
- Expanded the existing runtime simulation to exercise every Linux- and Darwin-visible run ID and to prove an absent non-required candidate fails. It verifies the explicit four-case Linux platform exclusion evidence and an empty exclusion map on Darwin. The same test matrix still injects missing/ignored/failed/duplicate outcomes for a non-required run candidate.
- Follow-up verification: `rtk test python3 -m unittest discover -s tests/support -v` — 35 passed; `rtk test python3 tests/support/matrix.py --check --all` — exit 0, list-only/no database execution. This host is Darwin, so matrix discovery includes the four macOS-only IDs; the unit test explicitly exercises the Linux exclusion.
- This pre-NLS-separation inventory counted 134 MySQL 5.7 run cases and 23 source-feature exclusions on Darwin, with 130 Linux-visible candidates after four OS-gated cases. The later NLS correction removed one PostgreSQL-only target from the MySQL inventory: the current counts are 156 total, 133 Darwin run candidates and 23 source-feature exclusions; Linux has 129 platform-visible run candidates after its four macOS-only exclusions. The native AMD64 lanes must prove the current 129 IDs execute exactly once alongside required-case checks. See [the NLS case separation](2026-10-03-T22-nls-case-separation.md).
