# 2026-10-03 — T21 — fail-closed phase comparability

- Status: audit found a fail-open phase gate; tightening it without running database workloads or timings
- Owner: `/root/t21_controls`, scoped to `tests/performance/` and this worklog
- Specification: [performance method](../docs/testing-and-performance.md#proving-faster)
- Working source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree is dirty
- Reference pins: pgloader `231ab86778ca5ffd7de40878714760c8b4860cdf`; dmt-rs `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`

## Reference research (before implementation)

| Query and prefix | Indexed revision and source lines | Finding | Adaptation and proof |
| --- | --- | --- | --- |
| `phase metrics status empty markers unsupported timing_eligible`; `project-my2pg` | Project index `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `tests/performance/runner.py:481-517,1249-1260`; `tests/performance/test_runner.py:533-535` | The parser returned no `status` for My2pg, and the gate only rejected an explicit `unsupported` status. Empty/missing/invalid phase data therefore counted as comparable. Existing tests checked only the pgloader unsupported case. | Require each run to explicitly yield normalized phase values and require identical nonempty phase-name sets across all runs. Unit-test missing, empty, unsupported, and mismatched data without databases; validate current v4 remains ineligible. |
| `pgtable-secs process-update-stats-stop-event summary phase timing`; `project-pgloader` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; `src/utils/monitor.lisp:301-320` and `clojure/tests/bench/report.py:1-77` | v4 records table COPY elapsed values and summary/process elapsed time but does not expose the T21 normalized phase contract. Existing benchmark tooling separates process wall time from loader summaries. | Keep v4 explicitly unsupported until a source-backed normalizer can map equivalent phase boundaries; never infer comparable phases from process elapsed alone. |
| `benchmark phase metrics timing memory cpu workload results`; `ref-dmt-rs` | dmt-rs pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; `docs/benchmarks.md:150-205` (from indexed passage) | Reference results disclose workload and resource limits; their job transfer stats do not define this app's normalized migration phases. | Reuse no metric definitions or numeric results. Require local matching phase keys, while leaving unsupported controls and dedicated-host execution as independent blockers. |

### Local contract and source review

- Read `docs/reference-coding.md`, `docs/testing-and-performance.md:111-160`, latest T21 worklogs, the current runner, the comparison manifest, and adjacent runner tests through `rtk` before deciding to edit.
- `tests/performance/runner.py:487-517` parses phase markers and derives elapsed intervals. My2pg's event sources are `src/pipeline/mod.rs:1780` (`finalize`) and `:1907` (`verify`); run elapsed time is finalized at `:1048`. The runner also retains per-table COPY elapsed values from its report.
- `tests/performance/runner.py:1249-1260` currently accepts any phase metrics whose status is not `unsupported`; absent status is therefore accepted. `tests/performance/test_runner.py:533-535` only proves pgloader is unsupported.
- Pinned pgloader's original monitor implementation at `pgloader/src/utils/monitor.lisp:301-320` measures table copy spans. Its report parser and local manifest still provide no normalized phase mapping; `loaders.comparison.example.json` deliberately declares phase/resource/behavior gaps.
- Read `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`; follow its minimal-change ladder. RTK is installed and used via `rtk run`; the repository guidance and prior T21 worklog state no separate RTK skill file exists in the searched Codex skills trees.

## Chosen change

Add a small, database-free comparator that fails closed unless every recorded loader run has explicit supported values for the complete canonical phase set `catalog`, `copy`, `index_constraints`, and `verification`. Matching only partial telemetry is insufficient for the performance contract. Make phase extraction explicitly supported only when the report contains a valid total elapsed time and at least one valid phase interval; otherwise label it unsupported. Keep the pinned pgloader adapter unsupported. This gate describes comparable phase telemetry only; it does not claim semantic equivalence, close other loader-control gaps, or make benchmark timings acceptable.

## Validation plan

- Add focused unit cases for missing/empty phases, malformed totals, unsupported v4, and mismatched phase names.
- Run `rtk test python3 -m unittest my2pg.tests.performance.test_runner -v`, Python compilation, and `rtk git diff --check` only. Do not run Docker/database workloads, timings, or refresh the shared XERJ index.
- Report that settings/resource equivalence, complete phase semantics, capacity-planned large data, and dedicated idle-host measurements remain open.

## Implementation and verification

- `tests/performance/runner.py` now labels extracted phase data unsupported unless it has ordered, named markers and a valid final elapsed time. Cross-run comparability requires every loader run to expose exactly the full canonical phase set (`catalog`, `copy`, `index_constraints`, `verification`) with explicit `supported` status. This closes the missing-status/empty-map fail-open path and prevents a matching partial set from satisfying the documented phase requirement.
- `tests/performance/test_runner.py` covers valid intervals, absent markers, out-of-order markers, malformed totals, unsupported pgloader data, missing/empty/mismatched phase sets, a matching partial phase set, and the report's false comparability result when metrics are absent.
- Verification: `rtk test env TMPDIR=/private/tmp PYTHONPYCACHEPREFIX=/private/tmp/t21-pycache python3 -m unittest discover -s tests/performance -p 'test_runner.py' -v` — 37 passed; `rtk test env TMPDIR=/private/tmp PYTHONPYCACHEPREFIX=/private/tmp/t21-pycache python3 -m py_compile tests/performance/runner.py tests/performance/test_runner.py` passed; `rtk git diff --check` passed.
- No Docker, database, correctness workload, or timing command was run in this audit. Existing binary-medium evidence is prior work: its container-wide RSS was sampled and direct-child RSS represented the Docker client, so neither is an equivalent loader-process peak-RSS comparison.

## Remaining T21 evidence

- The current My2pg event stream provides `finalize` and `verify` phase markers, and the report provides per-table COPY elapsed values; it does not yet provide a complete, normalized catalog/COPY/index-constraint/verification phase set. The pinned pgloader adapter remains explicitly unsupported for normalized phases. Thus `phase_metrics_comparable` stays false for the current comparison manifest.
- The manifest still lists concrete adapter gaps for byte/queue/oversized-row/global-memory bounds, reset sequences, rejection caps/durable acknowledgement and phase metrics. A 384 MiB JVM heap under the same 512 MiB Docker cap is not an equivalent My2pg memory budget. Correctness parity and matching outer Docker caps do not close these gaps.
- Actual per-container memory and CPU are sampled at one-second intervals, not peak/high-water or process CPU accounting. `wait4` observes only the direct child; for container runs that child is Docker CLI, not the loader. Acceptance still needs loader-container high-water RSS and comparable CPU evidence (or a documented, validated equivalent collector).
- Without the dedicated idle benchmark host, do not run the repeated 1-warmup/5-measured suite, large/memory profiles, or any concurrent DB matrix. A stable run still needs a capacity-planned multi-GiB corpus, correctness on every measurement, randomized loader order, pinned cache policy/storage/DB/network/build settings, phase and CPU/RSS evidence, and raw artifacts. No speed/RSS acceptance claim is possible from this host or from existing correctness-only runs.
