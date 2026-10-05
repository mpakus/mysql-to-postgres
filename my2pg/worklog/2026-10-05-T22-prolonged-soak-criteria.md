# 2026-10-05 — T22 — prolonged-soak acceptance criteria

- Status: repeated-stability criteria passed; broader T22 fault/platform gate remains open
- Agent/role: coordinator / integrator
- Task and specification links: [T22 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release), [fault/soak method](../docs/testing-and-performance.md#test-harness-and-ci)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Dependencies: current-source ARM64 MySQL 8.4.11/PostgreSQL 16.15 pins and T13 concurrency suite

## Purpose and evidence audit

The release checklist requires soak/failure runs to show no whole-table memory accumulation, stuck worker, or silent data loss. The existing eight-cycle sample ran 11 T13 cases per cycle for 1,219.48 seconds total. It correctly remained open, but neither the checklist nor test method had an objective duration/count/RSS threshold. The T13 RSS case samples two separate CLI migrations (20,000 and 200,000 rows per table) under a fixed 773,648-byte application budget; repeating the suite tests repeat stability and fault behavior, not one process held open for an hour.

An independent audit found the fault/soak plan names large rows, slow destination, intermittent transport, cancellation, bounded RSS trend, and filesystem failures, with no numeric threshold. It also confirmed a longer repeat run alone cannot close the whole T22 gate. This acceptance definition therefore closes only the prolonged repeated-stability sub-gate; uncovered fault types and platform lanes remain tracked separately.

## Acceptance definition

Using one owned database fixture and one unchanged executable on one host and pinned server pair:

- Run the complete `t13_concurrency` target serially at least 24 times and accumulate at least 3,600 seconds of measured target runtime.
- Require all 11 registered cases to pass in every cycle, including the blocked queue/cancellation, socket-failure, SIGTERM, sibling-ACK/commit, and fixed-budget RSS cases.
- For every cycle, require the existing RSS case's exact row-count and durable-report assertions, peak source/target connections no greater than 3, and RSS growth no greater than 1 MiB between 20,000 and 200,000 rows per table.
- Retain each raw output log and `rss.json`; stop only the uniquely owned fixture and verify cleanup.

These thresholds qualify only the repeated-stability sample on the tested host and server pair. They do not substitute for a slow-destination case, broader network/filesystem fault breadth, MySQL 5.7/AMD64, hosted CI, or other T22 gates.

## Reference research

No implementation code is changed by this task. Project XERJ and source inspection located the requirements in `docs/testing-and-performance.md`, `docs/checklists.md`, `docs/agent-tasks.md`, and `tests/cases/t13_concurrency.rs:477-567`. The RSS case starts a fresh CLI child for each of the two row profiles and checks RSS/rows/reports. The threshold above makes the planned repeat-stability run reproducible without treating repeated process samples as a single-process soak.

## Verification

The documented 24-cycle/60-minute threshold passed on one owned MySQL 8.4.11/PostgreSQL 16.15 fixture. See the [result ledger](2026-10-05-T22-prolonged-soak-result.md) for cycle hashes, duration, all 48 RSS observations, executable hashes, and cleanup evidence.

## Limitations

The prolonged repeated-stability sub-gate passed for this host and pair. Keep the T22 release checkbox open for remaining fault breadth, server/platform lanes, and hosted execution. Keep the overall release recommendation no-go until every release gate passes.
