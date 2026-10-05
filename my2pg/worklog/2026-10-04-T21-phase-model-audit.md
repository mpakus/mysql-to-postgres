# 2026-10-04 — T21 — canonical phase model feasibility audit

- Status: source audit complete; no phase adapter change is justified from current telemetry
- Owner: `/root/t21_gate_audit`, temporary read-only phase audit
- Task and contract: [T21 task board](../docs/agent-tasks.md#task-board), [performance method](../docs/testing-and-performance.md#proving-faster), [reference coding workflow](../docs/reference-coding.md)
- Project index snapshot: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; pinned pgloader: `231ab86778ca5ffd7de40878714760c8b4860cdf`
- Claimed files: this worklog only; `runner.py` and `test_runner.py` were not edited

## Reference research

| Query and prefix | Indexed revision | Source/test evidence | Finding and adaptation |
| --- | --- | --- | --- |
| `phase catalog copy index_constraints verification elapsed event report`; `project-my2pg` | `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | `src/pipeline/mod.rs:1421-1465,1780,1907,2551`; `src/model.rs:769-859`; `tests/performance/runner.py:488-535,538-603`; `tests/performance/test_runner.py:536-710` | Global events are elapsed-time point markers; table/range COPY events use a table-local clock. The report has per-table COPY duration and total run elapsed, not a run-global COPY interval or catalog/index/verification durations. Keep the four-phase comparator fail-closed; do not convert table-local durations into global intervals. |
| `summary pre data post fetch meta data Create Indexes COPY Wall-Clock Time`; `project-pgloader` | `231ab86778ca5ffd7de40878714760c8b4860cdf` | `clojure/src/pgloader/core.clj:539-545,749-799,800-825,987-990,1027-1094`; `clojure/src/pgloader/summary.clj:130-157`; `clojure/src/pgloader/stats.clj:86-115`; adjacent `clojure/test/pgloader/summary_test.clj:1-124` | The JSON retains `pre/data/post` aggregate totals, entry labels and elapsed nanoseconds. It has a COPY wall measurement and some index/constraint operation durations, but not a complete common phase contract. The existing upstream tests cover totals/rendering, not a stable named-entry benchmark contract. Do not infer missing boundaries from aggregate totals. |
| `Create tables Create Indexes Primary Keys Foreign Keys COPY Wall-Clock Time summary test`; `project-pgloader` | `231ab86778ca5ffd7de40878714760c8b4860cdf` | Same original `core.clj` locations; `summary_test.clj:32-124` | Pgloader starts some index work before/during COPY (`core.clj:800-825`) and waits for it after COPY (`:1027-1036`). Its index elapsed duration overlaps COPY. Preserve independent durations if future telemetry supports them; never sum them into a disjoint timeline or subtract COPY from `post`. |

## Why the existing measurements cannot safely pass the canonical gate

The accepted phase set is `catalog`, `copy`, `index_constraints`, and `verification`. Pinned pgloader v4 exposes its native groups `pre`, `data`, and `post`, entry-level elapsed values, and one separately labeled concurrent `COPY Wall-Clock Time`. Those are useful diagnostics, but they do not establish four matching scopes. In particular, `pre`/`data`/`post` are aggregation buckets, not a definition of the project's canonical phases; some target schema creation is not separately timed; and index work overlaps the COPY wall interval. A label-to-name rename would hide those scope differences.

My2pg's unscoped `finalize` and `verify` events report elapsed time since run start at the event point. Table/range COPY events report local table elapsed time (`src/pipeline/mod.rs:1421-1465,2551`), while the final report stores per-table COPY duration and end-to-end elapsed time (`src/model.rs:821-859`, `src/pipeline/mod.rs:1048`). These observations cannot recover catalog duration, aggregate concurrent COPY wall time, index/constraint duration, or isolated verification duration. Taking deltas between the two current global markers would combine all finalization work, and using the report end would include artifact settlement after verification.

Therefore, the available pgloader summary plus My2pg events cannot safely produce comparable values for all four canonical phases. The current `status=unsupported` path and exact-set `phases_are_comparable` check in `tests/performance/runner.py:488-491,592-603` are correct and must remain unchanged until measurements share an explicit scope contract.

## Concrete instrumentation contract

Before implementing a normalizer, define and emit observations for both loaders with the same documented semantics:

- Every observation has a monotonic start and end relative to one run clock, a canonical phase name, and enough scope to distinguish run-wide from table/range work. Durations are nonnegative; concurrent phases may overlap and remain independent intervals.
- `catalog` covers source catalog discovery and target schema/table preparation, with included operations enumerated explicitly.
- `copy` is the wall interval from first COPY worker start through last COPY worker completion. Per-table elapsed values remain a separate diagnostic and are never summed to obtain this interval.
- `index_constraints` records the elapsed wall interval for the corresponding index and constraint work, including any overlap with COPY; operation labels and included object classes are retained so differences in PK/FK/check behavior fail closed.
- `verification` measures the exact same independent source/target oracle around the same runner call for every loader, with its inclusion or exclusion from total end-to-end time fixed in the manifest.
- Keep loader-native summaries alongside normalized observations. A missing operation, ambiguous label, absent boundary, failed phase, or incompatible scope yields `unsupported`; do not fill a phase from the residual of grand total or assume phases partition total elapsed time.

Acceptance after that contract is implemented: fixture-backed parser tests for each loader's recorded boundaries; tests for missing/duplicate/negative events and allowed overlap; tests that table-local COPY values cannot satisfy run-wide COPY; a database-free comparator test requiring exactly all four canonical phases with supported scope metadata; then correctness-only runs proving the observations exist on actual MySQL workloads. Only after separate control/resource parity and capacity planning should repeated timing be considered on the required isolated host.

## Guidance and checks

- Read `docs/reference-coding.md`, `docs/testing-and-performance.md`, the current T21 phase/event worklogs, the runner, comparison manifest, and adjacent tests before this decision.
- RTK 0.49.0 was used for searches and source reads. Ponytail guidance was read from `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`; its minimal-change rule supports retaining the fail-closed branch rather than adding a lossy adapter.
- XERJ searches succeeded against the local endpoint. The first returned project passages from the snapshot above; the second and third returned original Clojure source and adjacent tests at the pinned pgloader revision above. Pinned reference indexes and checkouts were not changed.
- No database workload or timing was run. `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-phase-model-pycache python3 -m unittest discover -s tests/performance -p 'test_runner.py' -v` passed 44/44; `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-phase-model-pycache python3 -m unittest tests.support.test_package -v` passed 3/3; `rtk git diff --check` passed. These checks confirm the current parser/comparator tests and package boundary while making no database claims.

## Handoff

No code change is recommended from the current evidence. The next implementation needs the shared instrumentation contract above, followed by source-backed producer changes for both loaders or a documented approved benchmark contract change. Resource/control equivalence, large-profile correctness, process/container high-water and CPU evidence, and the dedicated benchmark host remain independent T21 gates. The coordinator owns checklist and shared-index updates.
