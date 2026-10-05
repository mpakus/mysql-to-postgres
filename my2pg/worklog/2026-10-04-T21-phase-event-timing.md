# 2026-10-04 — T21 — keep table COPY times out of run-phase timing

- Status: in progress
- Agent/role: coordinator / integrator, temporary T21 F ownership
- Task and specification links: [T21 task board](../docs/agent-tasks.md#task-board), [performance method](../docs/testing-and-performance.md#proving-faster)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Dependencies and tested revisions: T17, T19, and T20 are complete at this source snapshot
- Claimed files: `tests/performance/runner.py`, `tests/performance/test_runner.py`, this worklog

## Reference research (before coding)

- Project XERJ query: `project-my2pg "T21 benchmark phase markers catalog COPY indexes constraints verification"`; indexed project revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` points to `tests/performance/runner.py:488-530`, `tests/performance/test_runner.py:645-681`, and the T21 fail-closed phase audit.
- Reference XERJ query: `project-pgloader "stats entries pre data post create table indexes COPY phase labels"`; pinned revision `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `clojure/src/pgloader/summary.clj:121-155`, `clojure/src/pgloader/stats.clj:76-117`, `clojure/src/pgloader/core.clj:802-812,987-990,1027-1094`, and adjacent `clojure/test/pgloader/summary_test.clj:1-124`. pgloader's summary uses source-native aggregate entries; its table/copy work and run-global phases are not substitutes for My2pg's event clock.
- Local event finding: `src/pipeline/mod.rs:1425-1465` sets `elapsed_millis` to table-local `copy_elapsed_millis` for table-scoped events, but to time since the run timer for unscoped events. `tests/performance/runner.py:496-500` currently discards the table scope and mixes both clock domains as ordered run-global boundaries. The current comparison gate still rejects the partial My2pg phase set, but an individual run can retain misleading phase intervals.
- Ponytail full guidance and RTK 0.49.0 were applied: the minimal root fix is to preserve the event scope and exclude table-local events from the global phase timeline; per-table COPY elapsed values already remain available separately from the report.
- Adaptation and proof: add a database-free regression with a table-scoped COPY event plus global `finalize`/`verify` events. Require the COPY duration to remain in `copy_elapsed_millis_by_table` and never appear as a global phase interval. Keep pgloader's normalized phase metrics unsupported and preserve the existing fail-closed canonical phase-set gate.

## Changes

- `tests/performance/runner.py` now includes only unscoped phase events in the global timeline; table-scoped elapsed values remain available in `copy_elapsed_millis_by_table` from the run report.
- `tests/performance/test_runner.py` adds a regression where a table-local COPY value would otherwise produce a false global interval. It asserts the value remains attributed to its table, not the run timeline, and that the resulting partial phase set cannot pass the canonical comparison gate.

## Verification

| Check | Exact command or artifact | Environment/revision | Result |
| --- | --- | --- | --- |
| Performance parser suite | `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-phase-pycache python3 -m unittest discover -s tests/performance -p 'test_runner.py' -v` | Darwin/ARM64; dirty source at `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | 44 passed |
| Package boundary | `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-phase-pycache python3 -m unittest tests.support.test_package -v` | Darwin/ARM64 | 3 passed |
| Python syntax | `rtk test python3 -m py_compile tests/performance/runner.py tests/performance/test_runner.py` | Darwin/ARM64 | passed |
| Source package | `rtk test ./bin/cargo package --allow-dirty --locked --offline`, twice | Darwin/ARM64; extracted 223-file archive | both compiled; hashes matched `6dc50668af7f1553d8ecddfb3a684463495c042a14c5b40aac6d894df6bab7fd` |
| Whitespace | `rtk git diff --check` | Darwin/ARM64 | passed |

## Limitations or blocker

Correcting event clock interpretation does not add missing canonical phases, normalize pgloader's overlapping index/COPY measurements, resolve loader-control gaps, or qualify benchmark timing.

## Handoff

- Completed acceptance: table-local COPY elapsed data cannot become a run-global phase boundary; partial metrics remain ineligible for cross-loader comparison.
- Changed files: `tests/performance/runner.py`, `tests/performance/test_runner.py`, this worklog.
- Shared XERJ refresh: coordinator refreshes only `project-my2pg` after dependent checklist/package evidence is updated; pinned references remain unchanged.
- Remaining T21 work: define and instrument matching phase semantics across both loaders, close or explicitly accept settings/resource gaps, then run the repeated correctness-gated benchmark on a dedicated idle host with a capacity-planned corpus.
