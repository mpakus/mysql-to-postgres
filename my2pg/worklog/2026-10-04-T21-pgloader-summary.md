# T21 pgloader v4 summary telemetry

## Before-code evidence

- Task: request and retain pgloader v4's native JSON summary in each T21 run artifact; do not mark incomplete phase data comparable.
- Project query: `project-my2pg "performance runner phase comparability gate adapters"`; `project-my2pg "phase event catalog build plan finalize verify elapsed COPY report"`.
- Reference query: `project-pgloader "grand-total phases post COPY Wall-Clock Time summary JSON"`; `project-pgloader "summary report file JSON statistics CLI option"`.
- Project index revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; pgloader pin: `231ab86778ca5ffd7de40878714760c8b4860cdf`.
- Workflow: `docs/reference-coding.md`; relevant contract: `docs/testing-and-performance.md`, `tests/performance/runner.py`, `tests/performance/test_runner.py`.

## Findings

- `clojure/src/pgloader/cli.clj` exposes the `--summary` option and `clojure/src/pgloader/core.clj` writes the summary after the run.
- `clojure/src/pgloader/summary.clj` serializes per-entry `pre`, `data`, and `post` totals plus a grand total. The post phase includes an explicitly labeled `COPY Wall-Clock Time` entry.
- `clojure/src/pgloader/core.clj` times that entry around concurrent table COPY futures. Per-table `data` totals and the concurrent wall interval have different semantics.
- `clojure/tests/bench/report.py` parses `grand-total.total-nanos`, rows, bytes, and the labeled COPY wall value from this JSON schema.
- The benchmark runner currently omits `--summary`, despite mounting each run directory at `/bench`. Its normalized gate requires `catalog`, `copy`, `index_constraints`, and `verification`; the pgloader summary alone does not prove that mapping.
- Existing My2pg phase-marked output also lacks the complete normalized set. This change will retain source metrics and hashes while leaving both phase-comparison eligibility and T21 acceptance unchanged.

## Chosen adaptation

Have the pinned v4 invocation write `/bench/pgloader-summary.json`. Parse and validate its raw aggregate and COPY wall fields into a separate `pgloader_summary_metrics` artifact field. Reject malformed, negative, or missing required numeric data for that raw summary status. Keep `phase_metrics` explicitly unsupported for v4 with a precise explanation that only partial source phases are available. Add database-free adapter and parser tests. No pgloader implementation code or pinned reference files will be modified.

## Verification

  `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-pycache python3 -m unittest discover -s tests/performance -p 'test_runner.py' -v` — 39 tests passed. `python3 -m py_compile tests/performance/runner.py tests/performance/test_runner.py` and `rtk git diff --check` passed. No Docker or database benchmark was run; metrics were tested from synthetic JSON matching the pinned writer's schema. Refreshed only `project-my2pg` to 432 files and 1,521 passages (14 exclusions), then queried `pgloader-summary-telemetry` and confirmed this worklog and adapter parser are searchable. Pinned reference indexes were not rebuilt.

## Before-code review follow-up

- Project query: `project-my2pg "malformed JSON phase totals duplicate COPY parser"`; the indexed current source shows `tests/performance/runner.py:541-581` rejects unreadable JSON, requires pre/data/post totals, and requires exactly one `COPY Wall-Clock Time` entry. Adjacent adapter tests at `tests/performance/test_runner.py:541-581` cover a valid summary, a missing file, and a negative COPY duration, but not malformed JSON, absent phase totals, or duplicate COPY labels.
- Reference query: `project-pgloader "summary JSON phase totals COPY wall time"`; pinned pgloader revision `231ab86778ca5ffd7de40878714760c8b4860cdf`, `clojure/src/pgloader/summary.clj:121-146` emits all three phase totals, grand totals, and per-entry labels/timings. Read adjacent upstream tests in `clojure/test/pgloader/summary_test.clj:1-120`; they exercise rendered summaries and stats lifecycle, not JSON malformed-input behavior. This parser is my2pg's adapter, so the regressions belong in our Python tests; no upstream source change is appropriate.
- Required guidance: RTK is installed and used for searches/reads. No Ponytail skill file or executable was discoverable in the active environment or listed skill catalog; follow the project's documented smallest-complete-solution guidance and record this environment limitation rather than claiming the unavailable skill was applied.
- Adaptation and proof: add three database-free tests to `tests/performance/test_runner.py` using the existing temporary-file pattern. Assert `unsupported` and the specific fail-closed reason for malformed JSON, a missing `data.total.total-nanos`, and two COPY wall entries. Do not change production behavior; run the full performance runner tests and package-boundary checks.

## Test coverage follow-up

- Added those three regressions; the full performance runner suite now passes 42/42. The work is test-only, so no MySQL/PostgreSQL integration lane was rerun.
- `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-pycache python3 -m unittest discover -s tests/performance -p 'test_runner.py' -v` — 42 passed. `rtk git diff --check` passed.
- `rtk test ./bin/cargo package --allow-dirty --locked --offline` passed twice; both builds compiled the extracted 223-file package (2.6 MiB unpacked, 546.9 KiB compressed). Captured SHA-256 values matched: `60e469754de9861fddb011467974f8e8381925f680d8e1ed64b974eff0c7247f`. `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-pycache python3 -m unittest tests.support.test_package -v` — 3 passed.
- No XERJ reference corpus was refreshed. The coordinator will refresh `project-my2pg` after these worklog/checklist updates; immutable pinned reference indexes remain unchanged. T21 timing and all previously open release gates remain open.

## Independent review follow-up: invalid UTF-8 summary

- Project XERJ query: `project-my2pg "pgloader summary UnicodeDecodeError malformed bytes unsupported summary hash run artifact"`; index revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` points to `tests/performance/runner.py:541-546`, its adjacent parser tests, and run-record construction at `tests/performance/runner.py:1282-1323`.
- Reference XERJ query: `project-pgloader "write-summary-json UTF-8 malformed JSON summary tests"`; pinned revision `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `../pgloader/clojure/src/pgloader/summary.clj:121-146`: `write-summary-json` serializes the summary as JSON; adjacent `../pgloader/clojure/test/pgloader/summary_test.clj:1-120` tests summary rendering and stats, not corrupted JSON input. The corruption boundary belongs to our Python adapter.
- Finding: `Path.read_text()` can raise `UnicodeDecodeError`, which is not covered by the current `(json.JSONDecodeError, OSError)` handler. That escapes the adapter before the runner appends/persists the run record, although the later record path hashes the raw summary bytes independently.
- Adaptation and proof: extend the fail-closed read-error handler to include `UnicodeDecodeError`; add a database-free test that writes `b"\\xff"` and requires `unsupported` plus the existing “could not be read” reason. Verify the complete performance unit suite and package checks. Do not modify the pinned pgloader source or its index.

## Review fix verification

- Applied the one-clause exception-handler fix in `tests/performance/runner.py` and added one invalid-UTF-8 regression beside the malformed-JSON test in `tests/performance/test_runner.py`. The already-existing record path hashes summary bytes independently, so the fail-closed parser result can still be persisted.
- RTK 0.49.0 and Ponytail full guidance were used. Ponytail's ladder selected the shared parser boundary and standard-library exception handling; no helper, dependency, or abstraction was added. This corrects the earlier environment note for this follow-up (the prior test-only follow-up had reported Ponytail unavailable at that time).
- `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-pycache python3 -m unittest discover -s tests/performance -p 'test_runner.py' -v` — 43 passed. `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-pycache python3 -m unittest tests.support.test_package -v` — 3 passed. `py_compile` and `rtk git diff --check` passed.
- `rtk test ./bin/cargo package --allow-dirty --locked --offline` passed twice; each extracted 223-file package compiled. Both archives match SHA-256 `2b64112a436c699cd96c3daeaaf1110d905f5cc1bac509624f93b01ef1e9fbff`.
- The coordinator refreshes only `project-my2pg` after this worklog update. The pinned pgloader corpus remains unchanged; release timing and previously open release gates remain open.
