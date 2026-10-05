# 2026-10-02 — T21 — baseline readiness audit

- Status: audit complete; benchmark not run
- Agent/role: coordinator
- Task and specification links: [T21](../docs/agent-tasks.md#task-board), [performance method](../docs/testing-and-performance.md#proving-faster)
- Workspace/worktree: `/Users/renatibragimov/www/pg/my2pg`
- Base revision and tested revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` (working tree contains unrelated in-progress changes)
- Dependencies and their tested revisions: T19 and T20 complete; T17 remains in progress
- Claimed files: this worklog only

## Intended result

Establish whether the retained 2026-10-01 pgloader artifacts can support T21 performance claims, and identify the next benchmark prerequisite without describing smoke timings as benchmark evidence.

## Reference research (before coding)

No code was changed in this audit. XERJ searches were still used to locate the project contract and relevant reference material.

| Problem/query and prefix | Repository commit and file/line | Source/test behavior observed | Adaptation or rejection; our acceptance test |
| --- | --- | --- | --- |
| `benchmark fixed corpus performance RSS correctness`; `project-my2pg` | Working-tree corpus revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `docs/testing-and-performance.md:111-142` | Requires actual MySQL input, correctness on each run, one warmup plus five randomized measured runs, timing phases, process/container RSS, pinned resources, and declared multi-scale corpus. | Follow the contract; reject the saved three-row elapsed times as benchmark observations. Later acceptance needs fixed corpus digests and equal correctness checks for every measured loader. |
| `bench report elapsed throughput employees`; `project-pgloader` | pgloader `231ab86778ca5ffd7de40878714760c8b4860cdf`; `clojure/tests/bench/Makefile:1-150`, `clojure/tests/bench/report.py:1-120` | Existing machinery captures process wall time around loaders and parses loader phase summaries, but the suite mixes a MariaDB employees source with SQLite and CSV workloads. | Reuse the separation of process wall time and loader-reported phases as a design idea only. Reject its workload as a MySQL comparison corpus; independently record process/container RSS and correctness. |
| `benchmark throughput pipeline bounded memory`; `ref-dmt-rs` | dmt-rs `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; `docs/benchmarks.md:150-205`, `crates/dmt-rs/src/transfer/mod.rs:361-470` | Benchmark notes attribute observed throughput changes to source, target, and row shape, and disclose host/container resource settings; transfer uses bounded channels and reports per-job transfer stats. | Keep target DB and corpus shape explicit, and measure memory under increasing input sizes. Do not transfer numeric results or implement its different multi-source features. A my2pg benchmark must pass its independent value/structure oracle before timing is accepted. |

## Changes

- Inspected the retained v3/v4 JSON, logs, load files, checksums, baseline runner, and original pgloader benchmark scripts.
- Confirmed that both saved baseline JSON files are marked `benchmark: false`, run only three rows, and use MySQL 8.4.11 → PostgreSQL 16.15 on arm64.
- The v3 artifact passes six recorded data/sentinel checks and reports an informational 1.285-second process time.
- The v4 artifact exits zero but fails the binary value check: expected hex `00015c09ff`, observed `5830303031356330396666`. Its 2.763-second elapsed value is not a valid performance baseline.
- No my2pg benchmark runner or benchmark corpus is present. The current run-time database harness is for correctness fixtures and is not a repeated performance protocol.

## Verification

| Check | Exact command or artifact | Environment/revision | Result |
| --- | --- | --- | --- |
| XERJ project/reference queries | `python3 my2pg/bin/reference-search.py ...` for the three queries above | Local XERJ, corpus labels shown above | Found project performance contract, upstream timing scripts, and dmt-rs notes. Initial sandboxed localhost access was denied; the same read-only queries succeeded with the normal local-service permission. |
| Inspect correctness baselines | `tests/fixtures/baselines/2026-10-01/baseline-v3.json`, `baseline-v4.json`, and corresponding `.log` files | Saved fixture digest `mysql.sql=efbe952f0397182d11caf187cc806b741abd642799a6d7ad8e554c3bf49f7c56`; upstream pgloader revision above | v3: 6 checks passed. v4: binary check failed. Neither is a benchmark. |
| Inspect upstream benchmark protocol | `pgloader/clojure/tests/bench/Makefile`, `report.py`, and `docker-compose.yml` | Pinned pgloader checkout, read-only | Confirms workload mismatch: the existing relational benchmark source is MariaDB; other cases use SQLite/CSV. |

## Decisions and deviations

- Did not run a timing experiment: v4 fails correctness on the retained binary fixture, and the three-row corpus cannot measure the declared workloads or memory behavior.
- Did not update T21 status. Its implementation dependencies include T17, which remains in progress; the benchmark task itself remains planned.
- The next T21 implementation step is to create a deterministic, fixed-seed MySQL corpus manifest and an owned repeated-run harness that records seed digest, row count, loader/config/image identities, correctness result, process wall time, phase times, RSS, and raw artifacts. Start with a narrow no-binary workload on which both pinned baselines prove correctness, then add workload classes only when all loaders satisfy the same oracle.

## Limitations or blocker

- No qualifying large-data baseline exists yet. Do not quote the 1.285s or 2.763s values as speed evidence.
- v4's binary mismatch must either be fixed by an accepted mapping that passes independent checks or excluded from any workload where it fails; it cannot be called the fastest correct baseline without that evidence.
- T17 is unfinished, and there is no benchmark harness, fixed-seed MySQL corpus, repeated raw-run set, RSS series, or Rust performance result.

## Handoff

- Completed acceptance cases: baseline readiness and correctness classification.
- Artifacts and digests: original baseline hashes remain listed in `tests/fixtures/baselines/2026-10-01/artifact-hashes.json`.
- Files/revision ready to integrate: this audit worklog at source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- Remaining work: finish T17; implement fixed MySQL corpus and runner; collect repeated correct v3/v4/Rust results with RSS and full environment pins; profile and optimize only measured bottlenecks.
- Next task/owner: T21 / F with coordinator-owned harness and corpus review.

## Review

- Reviewer: pending
- Findings and resolutions: pending
- Task-board update/reference: none; T21 remains planned.
