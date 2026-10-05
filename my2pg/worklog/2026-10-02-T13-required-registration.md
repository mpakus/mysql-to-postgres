# 2026-10-02 — T13 — mandatory external-MDL test registration

Status: coordinator integration in progress. The native case is authored and registered in the Rust integration target; its mandatory lane ID still needs registry/test updates and a fresh full lane.

## Reference research before coordinator edits

- Read `docs/reference-coding.md`, the T13 task/acceptance notes, `tests/support/harness.py`, `tests/support/test_harness.py`, `tests/support/matrix.py`, and `tests/support/test_matrix.py`.
- XERJ `project-my2pg` query: `REQUIRED_CASES mandatory registration exact native test IDs hidden skips T13`, current indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It returned the established T11 required-case registration, T22 exact-discovery preflight and earlier T13 snapshot registration evidence.
- XERJ `ref-rust-postgres` query: `required test IDs cargo ignored list matrix CI`, pinned revision `1084ca8f5b5302e161892f2fa40abf71b4060c10`. Results were unrelated configuration source; the earlier T22 preflight worklog documents that upstream Rust Postgres CI has no reusable exact-required-ID contract. Reuse the project's own cross-platform registry and tests; no upstream code is applicable or copied.
- Read the exact registry and adjacent tests: `tests/support/harness.py:21-40,59-73`, `tests/support/test_harness.py:13-48`, `tests/support/matrix.py:139-183`, and `tests/support/test_matrix.py:75-90,130-145`. `matrix.py` derives its exact discovery requirements from `harness.REQUIRED_CASES`, so one base ID will propagate to all runtime lanes and offline discovery. `test_harness.py` separately asserts generic IDs on Linux/Darwin and exercises missing/ignored/failed/duplicate behavior.
- T13's authored-case worklog `2026-10-02-T13-external-metadata-lock.md` records the XERJ/reference research and direct source/test inspection for `tests/cases/t13_external_mdl.rs`. It selects an external MySQL WRITE lock plus a real blocked migration SELECT, then requires cancellation, zero acknowledged rows and disappearance of the exact worker connection while the lock holder remains alive.

## Chosen adaptation and acceptance

Add the exact external-MDL test ID to the existing base `REQUIRED_CASES` list and the platform test's generic-ID assertions. Do not add a second registry or relax version/platform requirements. The T13 test file is already declared in `tests/integration.rs`; validate the adjacent support unit suites, then rerun the full serial MySQL 8.4/PostgreSQL 16 lane so required-case preflight proves the test was discovered and executed exactly once.

No task-board/checklist closure is made here. The focused T13 case is not a substitute for the complete T13 concurrency acceptance or T22 matrix.
