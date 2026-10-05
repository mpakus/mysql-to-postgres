# 2026-10-01 — T12 — preserve reject-artifact failure classification

- Status: in_progress
- Agent/role: I — Integrator
- Task: T12 COPY rejection failure reporting; [task board](../docs/agent-tasks.md)
- Workspace: shared checkout `/Users/renatibragimov/www/pg/my2pg`
- Indexed revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; native evidence: `target/integration/my2pg-mysql84-pg16-841c8297518b`
- Claimed file: `src/pipeline/mod.rs` only; B owns `src/report/artifact_io.rs` and T12 fixture repair.

## Intended result

Keep reject persistence failure distinct from console/observer output failure in the run diagnostic. `RecoveryFailure::RejectIo` should report `REJECT_ARTIFACT_IO`; `ObserverIo` retains its existing console/artifact classification. This makes the T12 actual CLI assertion match the established rejection-artifact contract while preserving nonzero exit and exact durable row accounting.

## Reference research before code

| Query/prefix | Revision and original evidence | Finding and adaptation |
| --- | --- | --- |
| `REJECT_ARTIFACT_IO recovery RejectIo diagnostic`; `project-my2pg` | Index `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; read `src/pipeline/mod.rs:1064-1100`, `src/pipeline/recovery.rs:35-52,295-315,420-455`, and `tests/cases/t12_runner.rs:470-530`. | `copy_fault` currently folds `RejectIo` and `ObserverIo` into `ARTIFACT_OR_CONSOLE_IO`; the exact T12 assertion expects `REJECT_ARTIFACT_IO`. Split only those match arms. Keep observer failures and unknown target-write failures unchanged. |
| `reject file cannot write error`; `project-pgloader` | Pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; XERJ returned `clojure/src/pgloader/reject.clj` and the FK reject fixture. | Pgloader appends reject data/log files during row rejection, but its return contract offers no comparable structured distinction between reject-file and console observation failures. No diagnostic behavior is copied; the Rust task's own code and independent assertion are authoritative. |

Read [reference-coding.md](../docs/reference-coding.md) and current T12/T15 worklogs. `rtk` is installed and commands use `rtk run`; no Ponytail skill/tool is installed in the configured roots, so follow the project's documented smallest-complete-solution guidance and preserve this limitation.

## Changes

`src/pipeline/mod.rs::copy_fault` now maps only `RecoveryFailure::RejectIo` to `REJECT_ARTIFACT_IO`; `ObserverIo` retains `ARTIFACT_OR_CONSOLE_IO`. B owns the separate worker's definitive pre-write collision refusal, and its offline production-child regression verifies the failed ticket remains uncommitted while final report publication remains available. The T12 CLI regression continues to require exact equality between its JSON outcome and persisted final report.

## Verification

| Check | Result |
| --- | --- |
| Native T12 reject-file collision | Fresh serial native suite passed `t12_runner::actual_cli_reject_file_collision_preserves_final_failed_report_and_prior_commits`; persisted report and CLI outcome equality held with no `ARTIFACT_SHUTDOWN_UNKNOWN`. |
| Focused offline test | Production-child collision/report-publication and artifact-I/O regressions passed; full formatting, strict Clippy and offline lib/tests also passed. |

## Limitations

This patch only classifies a proven reject-artifact I/O error. It does not reinterpret unknown sync/write outcomes or change the artifact worker protocol. The collision path is validated in a fresh owned database pair. T12's physical storage-sync/ENOSPC acceptance remains open.
