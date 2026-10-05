# 2026-10-01 — T15 private artifact worker entry

Status: bounded integration slice implemented by the coordinator. Scope is `src/main.rs`, one actual-binary CLI regression in `tests/contracts.rs`, and this worklog. The writer is not yet connected to the run/recovery lifecycle; T15 and M2 remain open.

## Reference research before code

Read `docs/reference-coding.md`, `docs/scope-and-compatibility.md`, and the ownership/task rules in `docs/agent-tasks.md`; RTK and Ponytail guidance were already read for this task family. The current project index snapshot is revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.

XERJ `project-my2pg` query `runner DurableArtifacts admission reports reject budget` returned `src/pipeline/scheduler.rs`, the T15 protocol contract, and the runner worklogs. XERJ `ref-dmt-rs` query `bounded WriteJob transfer write_chunk` returned `crates/dmt-rs/src/transfer/mod.rs` at pinned revision `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Read the original transfer channel setup and dispatch (`mod.rs:414-418`, `450-630`): the bounded queue transfers owned jobs, and writer failures cancel the pipeline. It has no subprocess-private entry or durable artifact ACK; no implementation is copied.

Read the current `src/main.rs` command entry, `report/artifact_io.rs:31,1734` (`WORKER_ARGUMENT` and synchronous `worker_stdio`), `tests/support/artifact_worker.rs:94-123`, `tests/cases/t09_cli.rs`, and adjacent `tests/contracts.rs:289-350`. The production runner starts normal CLI parsing under `#[tokio::main]`; the current artifact writer launches the current executable with the exact private marker. Existing binary tests prove normal help/version and the artifact tests exercise protocol frames in a separate helper binary, but nothing verifies that the actual `my2pg` binary branches before Clap/runtime initialization.

## Chosen adaptation and proof

Use a small synchronous `main` dispatcher. Only an exact first argument equal to `WORKER_ARGUMENT` enters `worker_stdio`; it must have no additional arguments. That path returns before Clap parsing and Tokio runtime/signal setup, so stdin/stdout remain the binary protocol and protocol failures do not print a CLI usage page. Every other invocation enters the existing async command path unchanged.

Add an actual-binary process test for a private marker followed by EOF and for a marker with an unexpected argument. Assert both exit nonzero without usage text or argument echo; preserve existing `--help`/`--version` tests as normal-path guards. Run the focused contracts and formatting checks. This proves argv routing only; it does not prove durable process startup cancellation, artifact memory admission, report/reject integration, or shutdown receipts.

## Implementation and verification

`src/main.rs` now dispatches the exact private marker synchronously before constructing Tokio or parsing Clap. Worker failures use a fixed stderr diagnostic; extra argv is rejected without echo. Normal commands still enter the prior signal-aware async command path. `tests/contracts.rs` exercises the actual Cargo-built binary with protocol EOF and an extra argument.

Passed: `bin/cargo test --test contracts` (15 passed), `bin/cargo clippy --all-targets --all-features -- -D warnings`, `bin/cargo fmt --check`, and `git diff --check`. Refreshed the shared project index: 256 files, 975 passages, 11 exclusions. A post-refresh `project-my2pg` query for `WORKER_ARGUMENT private artifact worker main` returned the new test/worklog and actual `src/main.rs` entry point. No MySQL/PostgreSQL fixture was needed for this CLI-only change.
