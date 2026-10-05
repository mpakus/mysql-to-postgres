# 2026-10-05 — T21 — require explicit run-wide phase intervals

- Status: parser contract implementation in progress; benchmark phase gate remains closed
- Owner: coordinator / integrator
- Task: [T21 task board](../docs/agent-tasks.md#task-board), [performance method](../docs/testing-and-performance.md#proving-faster)
- Project source index: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`
- Pinned pgloader reference: `231ab86778ca5ffd7de40878714760c8b4860cdf`
- Claimed code/test files: `tests/performance/runner.py`, `tests/performance/test_runner.py`; this worklog

## Reference research (before coding)

| Query and prefix | Indexed revision | Original implementation and adjacent tests | Finding and adaptation |
| --- | --- | --- | --- |
| `benchmark phase boundaries catalog copy index constraints verification`; `project-my2pg` | `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | `tests/performance/runner.py:488-535,592-603`; `tests/performance/test_runner.py:536-710`; [phase model audit](2026-10-04-T21-phase-model-audit.md) | The runner currently differences ordered point markers and final report time. Table-scoped COPY is excluded, but these inferred durations still lack canonical producer scope. Accept only explicit run-wide start/end intervals and the exact canonical phase set. |
| `COPY Wall-Clock Time Create Indexes futures await phase`; `project-pgloader` | `231ab86778ca5ffd7de40878714760c8b4860cdf` | `pgloader/clojure/src/pgloader/core.clj:800-825,987-990,1027-1094`; `pgloader/clojure/test/pgloader/summary_test.clj:1-124` | pgloader's native summary has a run-wide COPY wall duration and overlapping index work, but not absolute phase boundaries or a catalog/verification contract. Keep native summary diagnostic-only; do not derive missing intervals from `pre/data/post` totals. |
| `summary_test pre data post total-nanos entry`; `project-pgloader` | `231ab86778ca5ffd7de40878714760c8b4860cdf` | `pgloader/clojure/src/pgloader/summary.clj:121-157`; adjacent `summary_test.clj:32-124` | Upstream tests verify aggregates/rendering and wall-nanos display, not normalized benchmark events. Do not change the pinned checkout or treat its tests as proof of the new contract. |

The existing `docs/reference-coding.md`, T21 phase model/event worklogs, comparison runner, and nearby tests were read. RTK 0.49.0 and Ponytail full guidance were applied. Ponytail supports the smallest root fix: reject synthetic durations and consume explicit interval observations; no new dependency or production abstraction is required for this parser step.

## Change and proof

The parser will accept only records that explicitly identify a canonical phase, run-wide scope, and nonnegative start/end on the same elapsed-millisecond clock. It will reject missing, duplicate, incomplete, reversed, unknown-scope, and table-local records. The normalized result is supported only when all four required phase names are present exactly once. Concurrent intervals remain independent; no residual or sequential subtraction is allowed.

Database-free regressions will cover a valid complete set with COPY/index overlap, missing and duplicate phases, negative/reversed boundaries, unknown scopes, and legacy point markers. Existing comparison checks continue to require every run to expose exactly the canonical set. The producer instrumentation for My2pg and pgloader remains a separate follow-up; until both emit this schema, real phase comparison must remain unsupported.

## Verification

- `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-explicit-phases-pycache python3 -m unittest discover -s tests/performance -p 'test_runner.py' -v` — 44 passed.
- `rtk test python3 -m py_compile tests/performance/runner.py tests/performance/test_runner.py` — passed.
- `rtk git diff --check` — passed.
- Coordinator refreshed `project-my2pg-source`: 446 files, 1540 passages, 14 exclusions. The follow-up XERJ query `project-my2pg "explicit run-wide phase interval operation classes"` returned this worklog, parser, regression tests, and checklist at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- No database workload or timing was run. Neither loader currently emits the required interval records; comparison therefore remains unsupported. Do not run repeated timing until producer instrumentation and correctness checks are complete.

## Handoff

- The parser and its database-free tests now require exactly four explicit run-wide intervals with fixed operation classes and preserve overlapping COPY/index durations.
- Project performance documentation defines the event schema. The checklist remains open for producer instrumentation, workload correctness, parity, capacity, and dedicated-host timing.
- Next T21 implementation: instrument both producers at their actual operation boundaries and add fixture-backed event contract tests. Leave pinned pgloader sources read-only; any upstream instrumentation must be an attributed out-of-tree patch incorporated into the controlled benchmark artifact.
