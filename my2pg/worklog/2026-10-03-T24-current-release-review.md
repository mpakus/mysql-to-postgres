# 2026-10-03 — T24 — current dirty-tree independent review

- Status: targeted review complete; release recommendation is no-go. The original T22 coverage gap was fixed, but independent inspection found a Linux CI regression in the fix; existing M4 release gates also remain open.
- Reviewer: independent Codex agent
- Reviewed repository: `/Users/renatibragimov/www/pg/my2pg`
- HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; worktree was dirty.
- Scope: current T21 medium-binary result, T22 MySQL 5.7 applicability/AMD64 CI changes, T23 current source-package hash, core commit accounting and MySQL-only boundary.

## Reference research

Read the current root and project `AGENTS.md`, `docs/reference-coding.md`, `docs/scope-and-compatibility.md`, `docs/agent-tasks.md`, `docs/checklists.md`, and the T21/T22/T23/T24 worklogs. Used RTK 0.49.0 and the required Ponytail review guidance. XERJ was queried read-only; the shared index was not refreshed.

| Query / prefix | Index revision | Result inspected |
| --- | --- | --- |
| `T21 medium binary 5000 rows payload 409648000 bytes correctness timing-ineligible` / `project-my2pg` | `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | Retrieved and read `2026-10-03-T21-binary-medium-correctness.md`, runner, test, and checklist passages. |
| `MySQL 5.7 134 run candidates 23 not applicable AMD64 CI native` / `project-my2pg` | `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | Retrieved and read current case-applicability, pending-case adaptation, AMD64 pin/CI worklogs, harness, support tests and workflow. |
| `T23 package hash 374b7daf 220 files source package revalidation` / `project-my2pg` | `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | Retrieved the latest package-revalidation worklog and release checklist. |
| `copy_in finish transaction commit error test` / `ref-rust-postgres` | `1084ca8f5b5302e161892f2fa40abf71b4060c10` | Inspected COPY `finish` and abort behavior in `tokio-postgres/src/copy_in.rs` and `tests/test/main.rs::copy_in_error`. |
| `result stream cancellation pending result drop cleanup` / `ref-mysql_async` | `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5` | Inspected `query_result/result_set_stream.rs`, `query_result/mod.rs`, and adjacent result-drop tests. |
| `bounded byte queue cancellation permits` / `ref-dmt-rs` | `4e8015f7e841dbdf9df01e953aeb3948dfb3199a` | Inspected orchestrator task/permit placement. It is only a design reference, not an acceptance oracle. |

These searches did not lead to copied reference code. The current design uses explicit COPY finish/transaction commit and explicit source-reader cleanup; T24 conclusions are based on my2pg behavior and its tests.

## Findings

### P1 — T22 can report a successful lane without proving all applicable `run` cases executed

`tests/support/harness.py:681-689` resolves a static 157-case inventory into `run`, `not_applicable`, or `pending` before database startup. In the run path (`720-764`), it parses every observed case, but validates exact `['ok']` status only for the small `required_cases(...)` subset (`729-731`). It rejects unknown observed IDs and observed non-applicable cases (`732-762`), then accepts any nonzero suite pass count (`763-764`). It does **not** require each other case assigned `run` to appear exactly once and pass. `tests/support/test_harness.py:53-87` exercises missing/ignored/failed/duplicate states only for required IDs, so the gap is untested. A known but omitted/compile-gated/filtered non-required case remains classified as `run` in the evidence without failing the lane.

This matters to the newly configured MySQL 5.7 AMD64 jobs (`.github/workflows/ci.yml:56-77`): the static audit makes 134 cases eligible, but the runner currently proves only its required subset plus overall cargo success. The inventory contains platform-conditional cases such as `t12_uncertain_storage::sent_commit_uncertainty_survives_physical_failure_to_publish_new_report` (`tests/cases/t12_uncertain_storage.rs:1-3`), which is macOS-only and absent from the Linux x64 test binary. Therefore the fix should derive the compiled ignored-test list for the active host/lane (for example, `cargo test -- --list --ignored`) and require each applicable `run` ID exactly once with `ok`, with an explicit platform disposition for cases unavailable on that host. Do not simply demand all 134 on Linux.

**Disposition:** the original gap was fixed in the current dirty worktree by requiring every `run` case to appear exactly once as `ok`; see the follow-up finding below. Native 5.7 hosted execution has not occurred.

### P1 — runnable-case enforcement fails Linux lanes on macOS-only inventory cases

The coordinator's fix closes the false-success gap (`tests/support/harness.py:732-734`) by building `runnable` from every case whose plan status is `run`, then requiring `["ok"]`. The plan is built from the complete static inventory at `:682`, and all three MySQL manifests default inventory entries to `run` unless listed in the MySQL 5.7 not-applicable set (`:262-273`). However, at least four inventory IDs are compiled only on macOS: `t12_uncertain_storage.rs:2` has file-level `#![cfg(target_os = "macos")]` and its ID is in the inventory at `harness.py:156`; `t12_durable_faults.rs:2` has the same file-level gate and its three IDs are in the inventory at `harness.py:126-128`. These are not MySQL-version exclusions, so they remain `run` in the Linux plan but cannot appear in Linux Cargo test output. The newly configured native x64 GitHub jobs run Ubuntu (`.github/workflows/ci.yml:56-69`), so those lanes will fail the new `unmet_runnable` check even if every Linux-applicable test passes.

The new synthetic support test (`tests/support/test_harness.py:53-94`) constructs output containing every manifest `run` case for both mocked Linux and Darwin. It therefore cannot catch a target-gated test missing on Linux. This is a regression introduced by the enforcement fix, not a reason to restore the earlier permissive behavior. Resolve it with explicit per-platform applicability or compiled-test discovery and a reviewed host-aware plan; preserve exact-once enforcement for each case that is runnable on the current host. Add a test where macOS-only IDs are absent from Linux output and present on Darwin, and verify the persisted evidence explains the platform exclusion.

**Disposition:** new T22 P1 acceptance blocker. No hosted lane was run during this independent review. The fix must be amended before the new AMD64 matrix can produce valid results.

### P1 — no release recommendation can be positive yet

The medium binary correctness run now closes its prior functional gap: the saved result at `target/integration/my2pg-mysql84-pg16-c4aca805d4a8/t21/binary_large_rows-medium-688b4f94d5c6/results.json` has SHA-256 `a7e97fc0d09dfb3010436155ea7753b376be8c3cc6d04afbc1a2ac2e7cc647cb`. Its worklog records 5,000 rows, 409,648,000 payload bytes and correctness-only completion for both loaders. The result is explicitly timing-ineligible: v4 has no comparable phase metrics or equivalent batch/queue/memory controls, the sampled container memory is not an equivalent peak RSS metric, and the large narrow-integer profile remains unverified after exit 137.

T22's static inventory now has 134 `run`, 23 source-feature `not_applicable`, zero `pending`; native x64 pins and the `ubuntu-24.04` workflow job exist. These are preparation, not runtime evidence. The worklog and checklist say hosted AMD64 MySQL 5.7 runs remain open.

T23's latest package hash was independently checked against the current local archive: `shasum -a 256 my2pg/target/package/my2pg-0.1.0.crate` returned `374b7dafbcaf977faa1fe7254a28f7fa537370bcbd127e8b1d1685a08cf5965c`, matching the last package-refresh entry. This is dirty macOS ARM64 package evidence. Qualified dependency-license/fixture-attribution review, native Linux/AMD64 distribution execution, publication, and release checksums remain open.

The release checklist continues to leave timing/equivalence, speed/RSS, soak/failure, complete version matrix, qualified licensing/attribution, distributed artifact runs, and final integrated revision items unchecked (`docs/checklists.md:97-111`).

**Disposition:** no-go for a v1 release or claim that my2pg is faster. Continue the authorized implementation work; the remaining release gates and T22 runtime/coverage requirements are part of the requested objective.

### No new core data-loss, replay, or alternate-source defect observed in reviewed paths

- The source URL validator accepts only `mysql://` (`src/config/validation.rs:955-969`), and connection-time identity/version checks reject recognizable non-Oracle variants and restrict the reviewed MySQL lines (`src/mysql/mod.rs:96-130`).
- PostgreSQL COPY is explicitly finished, its affected-row count is checked, and the transaction's commit acknowledgment is the accounting boundary (`src/postgres/mod.rs:788-875`). Pipeline state retains possible commit ambiguity (`src/pipeline/mod.rs:1171-1202,2974-3047`).
- The registered lost-ack case (`tests/cases/t12_lost_commit_ack.rs:204-354`) observes a completed server COMMIT, drops the client acknowledgment, expects an indeterminate report, independently reads the exact two target rows, and verifies no replay.

This is targeted review evidence only, not a full source audit or database rerun. The code and workflow test changes compiled in the saved T22/T23 results; no implementation, checklist, or index was changed by this reviewer.

## Commands and evidence

- `rtk --version` — 0.49.0.
- `python3 my2pg/bin/reference-search.py ...` — the six queries above; XERJ existing local indexes, read-only; no refresh.
- Read-only source inspection: `git diff -- .github/workflows/ci.yml tests/support/harness.py tests/support/matrix.py tests/support/test_harness.py tests/support/test_matrix.py tests/compose/images.json docs/checklists.md tests/cases/t05_mysql.rs tests/cases/t10_empty_string_default.rs tests/cases/t10_enum_metadata.rs tests/cases/t10_label_recovery.rs`; `sed`/`nl` of current harness, test and core source paths listed above.
- `shasum -a 256 my2pg/target/package/my2pg-0.1.0.crate my2pg/target/integration/my2pg-mysql84-pg16-c4aca805d4a8/t21/binary_large_rows-medium-688b4f94d5c6/results.json` — both hashes match the latest saved T23/T21 worklogs.
- `git -C my2pg diff --check` — passed.
- No database tests, CI jobs, containers, benchmarks, implementation edits, or checklist edits were run by this read-only review.

## Handoff

1. Amend T22 enforcement with platform-aware inventory and tests that model Linux-missing/macOS-present gated cases, then run the native AMD64 MySQL 5.7→PostgreSQL 16/17/18 jobs and retain exact logs/artifact hashes.
2. Complete T21 equivalent-setting/timing/RSS gates on a dedicated host; correctness-only medium-binary success does not satisfy speed goals.
3. Complete T23 human license/fixture review and tested native distribution/publication/checksum gates.
4. Repeat the independent release-candidate review after those dependencies close; keep release status no-go until evidence satisfies the checklist.

## Follow-up review after T21/T22 fixes

- Re-read the complete T21 phase-comparability worklog and current parser/tests after `/root/t21_controls` completed. `tests/performance/runner.py:488-533` now labels malformed/absent phase inputs unsupported; `phases_are_comparable` at `:536-546` requires an explicit supported status and the exact canonical phase key set. Tests at `tests/performance/test_runner.py:534-582` cover malformed/missing/empty/mismatched/partial metrics and the positive full-set predicate. The T21 worklog records 37 tests, Python compilation and diff check passing, and no database/timing run. This closes the earlier fail-open comparison gate; it does not make either current adapter phase-comparable or timing-eligible. The T21 worklog says My2pg exposes only finalize/verify markers and pinned pgloader v4 remains unsupported.
- Re-read the coordinator's `worklog/2026-10-03-T22-runnable-case-enforcement.md`: support tests and matrix preflight passed, but there is no native DB/hosted CI result. Independently inspected its implementation and found the Linux target-gating regression above. The repo worklog says 134 MySQL 5.7 static `run` cases / 23 source-feature `not_applicable`; this is not runtime evidence.
- Follow-up XERJ project queries against the existing index: `project-my2pg T21` returned results from indexed source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; searches `project-my2pg T21_phase_comparability_explicit_supported_complete_canonical_phase_set_empty_missing_malformed`, `project-my2pg T22_MySQL57_AMD64_run_disposition_every_case_execution`, `project-my2pg T23_package_hash_220_files_374b7daf`, `project-my2pg runnable_case_mysql57_exact_dispositions`, `project-my2pg current_source_package_374b7daf`, and `project-my2pg phase_metrics_unsupported_comparability_empty` each returned zero hits. The project XERJ index was intentionally not refreshed; latest code/worklog evidence was read directly. Existing pinned reference evidence remains pgloader `231ab86778ca5ffd7de40878714760c8b4860cdf`, dmt-rs `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`, tokio-postgres `1084ca8f5b5302e161892f2fa40abf71b4060c10`, and mysql_async `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5` from the reference searches recorded above.
- Rechecked the two saved artifact hashes: package SHA-256 remains `374b7dafbcaf977faa1fe7254a28f7fa537370bcbd127e8b1d1685a08cf5965c`; T21 correctness-only results SHA-256 remains `a7e97fc0d09dfb3010436155ea7753b376be8c3cc6d04afbc1a2ac2e7cc647cb`.
- Current release checklist at `docs/checklists.md:101-113` still has required version lanes, performance, soak/failure, license/provenance/artifact checksum, distribution, independent release review, and final recommendation entries open. Recommendation remains **no-go**; this review did not run database tests, CI, containers, package builds, or benchmarks.

## Follow-up: T22 platform-aware runnable-case fix verified

- Reviewed the current fix after the coordinator centralized the four macOS-gated IDs in `MACOS_ONLY_CASES` (`tests/support/harness.py:48-53`). Linux now excludes exactly those four from `runnable_mysql_case_ids` and writes explicit `target_os=macos` reasons into `platform_unavailable_case_reasons` (`:350-360,751-764`); Darwin excludes none. The static MySQL 5.7 plan remains 134 version-compatible `run` cases plus 23 MySQL-feature exclusions; the runtime check now requires 130 on Linux and all 134 on Darwin. Platform omissions remain separate from MySQL version dispositions.
- Adjacent source confirms the four IDs correspond to the file-level macOS gates in `tests/cases/t12_durable_faults.rs:2` and `tests/cases/t12_uncertain_storage.rs:2`. `tests/support/test_harness.py:53-96,128-152` now checks missing/ignored/failed/duplicate non-required cases on both simulated hosts, verifies the four explicit Linux exclusions, and asserts Linux/Darwin runnable counts. `tests/support/matrix.py:17,151-157,194-199` reuses the centralized case set in host-specific discovery.
- Verification: the first sandboxed support-suite attempt could not create temporary directories and reported 13 environment errors. Re-ran with the permitted elevated test execution and an external temporary/cache directory: `rtk test env TMPDIR=/private/tmp PYTHONPYCACHEPREFIX=/private/tmp/t24-platform-review-cache python3 -m unittest discover -s tests/support -v` — 35 passed. `rtk test env PYTHONPYCACHEPREFIX=/private/tmp/t24-platform-review-cache python3 tests/support/matrix.py --check --all` — exit 0; list-only preflight, no databases started. No hosted CI or native MySQL 5.7 runtime was run.
- XERJ query `project-my2pg T22_platform_unavailable_cases_runnable_mysql_case_ids` returned zero hits against the existing project index (indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`); no shared index refresh was performed. I relied on the current source, adjacent tests, and the updated T22 worklog for this follow-up.
- **Disposition:** the previously reported P1 Linux case-enforcement regression is closed by the host-aware fix and its focused test coverage. This does not close the MySQL 5.7 hosted-lane evidence gate or change the overall release recommendation: T21 performance comparability, remaining T22 native hosted runs, T23 human license/fixture review and distribution/checksum gates, soak/failure evidence, and final integrated release review remain open. No other release concern changed in this focused verification.
