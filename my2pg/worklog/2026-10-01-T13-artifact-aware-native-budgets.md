# 2026-10-01 — T13 — native fixtures account for durable artifact memory

- Status: fixture correction validated; broader T13 remains open
- Agent/role: I — Integrator
- Task and specification links: [T13](../docs/agent-tasks.md); [resource and artifact admission](2026-10-01-T15-artifact-memory-admission.md)
- Workspace/worktree: shared checkout `/Users/renatibragimov/www/pg/my2pg`
- Base revision and tested revision: project index snapshot `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; test run `target/integration/my2pg-mysql84-pg16-939d0f914d6c`
- Dependencies and their tested revisions: T15 artifact reservation is integrated in this dirty checkout; upstream pins remain those in `docs/reference-repositories.json`.
- Claimed files: `tests/cases/t08_pipeline.rs`, `tests/cases/t13_concurrency.rs`, `tests/cases/t13_shutdown_phases.rs`, this worklog.

## Intended result

Make the T08/T13 native resource tests budget the run-lifetime artifact reservation introduced by T15. A test asking for `n` concurrent table pipelines must budget `A + n×P`, where `A` is the artifact worker reservation and `P` is one pipeline reservation. Schema-only tests must still budget `A`, while retaining zero table-pipeline admission and their separate index-worker assertion. Run the cases on a fresh owned fixture so earlier panic state cannot influence PostgreSQL's global event-trigger catalog.

## Reference research (before coding)

| Problem/query and prefix | Repository commit and file/line | Source/test behavior observed | Adaptation or rejection; our acceptance test |
| --- | --- | --- | --- |
| `schema-only artifact reservation memory_bytes`; `project-my2pg` | Project index `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `src/pipeline/scheduler.rs:114-190,494-522`, `worklog/2026-10-01-T15-artifact-memory-admission.md:9-17` | T15 subtracts the fixed `artifact_reservation_bytes` before dividing remaining memory by `pipeline_reservation`. Schema-only skips row-pipeline bytes but still needs `M >= A`. | Retain the T15 contract; update stale native fixture values instead of weakening admission. Assert schema-only completes with artifact headroom and table workers remain zero. |
| `event trigger wait_active timeout cleanup`; `project-my2pg` | Same indexed revision; `tests/cases/t13_concurrency.rs:98-100,197-220,790-835`, `tests/cases/t13_shutdown_phases.rs:90-118,145-180`, `tests/cases/t08_pipeline.rs:165-190` | The `exact_memory` helper budgets only `n×P`; the run report proves T13 admitted 1/2 pipelines with `application_bytes=151856` and `artifact_bytes≈40.8–43.3 KiB`. Schema-only fixtures still set `memory_bytes=1`. A panic before explicit trigger cleanup contaminates subsequent cases because event triggers are database-global. | Add artifact headroom below one additional pipeline reservation, set schema-only fixture budgets above `A`, and use the harness's required serial test order/fresh pair. Validate exact worker diagnostics, successful event-gate phases, and later T16 planning on a clean fixture. |
| `event trigger transaction cleanup connection drop`; `ref-rust-postgres` | Pin `1084ca8f5b5302e161892f2fa40abf71b4060c10`; returned passages in `postgres/src/transaction.rs:1-36` and `tokio-postgres/src/transaction.rs` rollback guard | Transaction drop provides rollback, but it does not clean a separately committed database-global event trigger after a test panic. | Reject transaction-drop as cleanup for the global object; do not broaden this memory-budget correction into an async cleanup redesign. Existing fixture teardown and fresh owned-pair isolation remain the boundary. |
| `test cleanup after panic async`; `ref-dmt-rs` | Pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; search returned unrelated TUI panic-hook and security-plan passages, no database-fixture cleanup precedent. | No applicable pattern was found. | Record no-match rather than importing unrelated terminal cleanup. |
| Follow-up `artifact reservation pipeline_reservation native exact memory`; `project-my2pg` | Same indexed revision; returned `scheduler.rs:421` plus the T15 and T13 admission worklogs. | The exact-one-pipeline rerun used `M=207000`, actual `A=45206`, and still admitted two workers. With the logged fixture layout, 128 KiB headroom exceeds `P`; the prior margin assumption was incorrect. | Use 64 KiB headroom, greater than observed `A` (40.8–47.9 KiB) and less than observed fixture `P` (~75.9 KiB), so `floor((M−A)/P)` stays exactly `n`. Acceptance is the emitted `table_workers=n/requested` diagnostic and gate count. |
| `fixed-budget RSS tenfold source rows time bound`; `project-my2pg` | Same indexed revision; `docs/testing-and-performance.md:121-139`, `tests/cases/t13_concurrency.rs:452-520` | The test is a bounded-memory correctness oracle, while the testing guide reserves stable comparative timing for dedicated multi-run benchmarks. In the second native run, 20k rows/table completed in 12.8s; the 200k rows/table case had committed 140.4k/table at 89.7s, so its 90s process deadline is too tight for this host. | Preserve the 10× workload, fixed memory, RSS capture and correctness checks; extend only the deadlock guard to 180s. Do not call this timing a benchmark or alter production budgets. |

Read the original T15 admission code, the current T08/T13 tests and their fixture helpers. `rtk` is available at `/opt/homebrew/bin/rtk` and all repository commands use `rtk run`. No installed `ponytail` executable, tool, or skill file was found in the configured skill roots; apply the documented smallest-complete-solution approach and record this environment limitation. No XERJ reference or project source is copied.

## Changes

T08 and schema-only index fixtures now budget 1 MiB for durable artifact control state while using zero table-pipeline bytes. The first two-pipeline margin was set to 128 KiB; the second native run proved that this exceeds the tested pipeline reservation and can admit an extra table worker. The final source uses 64 KiB headroom, above observed artifact `A` and below measured `P`; the third fresh native run passed the exact-one-worker case. The fixed-budget RSS timeout is 180s; its 10× workload, fixed application memory and RSS assertions passed in 128.2s for the larger input. The integrated 126-case suite passed after the T12 writer repair froze.

## Verification

| Check | Exact command or artifact | Environment/revision | Result |
| --- | --- | --- | --- |
| First integrated native suite | `rtk run 'python3 tests/support/harness.py mysql84 pg16'` | Disposable pair `my2pg-mysql84-pg16-939d0f914d6c`, serial execution | Failed: 101/112 integration tests passed. T08 and T13 memory fixtures omitted `A`; the T13 schema-only panic skipped global trigger cleanup and contaminated T16. Exact IDs and evidence are in `rust-tests.json`/`rust-tests.log`. |
| Second integrated native suite | Same command | Fresh disposable pair `my2pg-mysql84-pg16-841c8297518b`, serial execution | Improved to 109/112 integration tests passed. T08, the gated T13 queue/cancellation tests, T13 shutdown, and both required T16 cases passed. Three failures remain: T12 reject-file collision final-report comparison, T13 fixed-budget RSS exceeded its 90-second deadline, and T13 exact-one-pipeline budget admitted two workers because the initial 128 KiB headroom exceeded this fixture's `P` (~75.9 KiB). No event-trigger contamination remained. |
| Third integrated native suite | Same command | Fresh owned pair `my2pg-mysql84-pg16-10a9d7a6f734`; binary revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, dirty working tree | Passed 126 cases, zero failures. All seven required T10/T13/T15/T16 case IDs passed exactly once. T12 reject-file collision passed; T13 exact-one-pipeline, schema-only index and shutdown gates passed. The 10× RSS comparison passed: 20,000 rows/table peaked at 27,744 KiB and 200,000 rows/table peaked at 28,160 KiB, with 773,648 application bytes. Full log and machine-readable result are under `target/integration/my2pg-mysql84-pg16-10a9d7a6f734`; log SHA-256 is `77ed20d237b4e56e6bfc6432250468bd6a2369d7371df3fb5c9dca916d92dfb3`. Harness exited 0 and stopped its owned pair. |
| Offline checks | `rtk run 'bin/cargo fmt --all -- --check && bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings && bin/cargo test --offline --locked --all-features --lib --tests -- --test-threads=1'` | Current working tree | Formatting, strict Clippy and the complete offline lib/tests command passed. Main integration target: 10 passed, 0 failed, 112 native-only cases ignored. |

## Decisions and deviations

The earlier T13 tests correctly exercised pipeline-only memory before T15 added a run-lifetime artifact reservation. The test inputs were stale. This correction changes fixture budgets and the RSS test's deadlock guard only; it does not change migration limits or production preflight. The fresh native suite validates 64 KiB headroom against the exact-one-worker oracle and the existing 10× fixed-budget RSS assertions. Wider T13 fault and external-MDL evidence remains open.

## Limitations or blocker

The integrated native development suite now passes, but this does not complete the full T13 fault/external-MDL matrix, T12 physical storage fault lane, T15 full persistence gate, or grouped M2 acceptance. The earlier T16 event-trigger failure was downstream contamination in the first run, not evidence against T16's fail-closed diagnostic.

## Handoff

- Completed acceptance cases: T08 schema-only artifact budget; T13 exact-one-pipeline, schema-only index, shutdown and fixed-budget 10× RSS cases; integrated serial suite 126/126; seven harness-required IDs exactly once.
- Artifacts and digests: `target/integration/my2pg-mysql84-pg16-10a9d7a6f734/rust-tests.json` and `.log`; log SHA-256 `77ed20d237b4e56e6bfc6432250468bd6a2369d7371df3fb5c9dca916d92dfb3`.
- Files/revision ready to integrate: fixture and timeout changes in `tests/cases/t08_pipeline.rs`, `tests/cases/t13_concurrency.rs`, and `tests/cases/t13_shutdown_phases.rs` are validated in the current dirty working tree.
- Remaining work: complete T13's broader fault/external-MDL acceptance and remaining task gates; preserve this suite as accepted evidence, not task completion.
- Next task/owner: I — Integrator; F/G own remaining T13 work under the task board.
