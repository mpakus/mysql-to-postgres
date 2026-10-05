# T15 — direct pipeline integration tests use the real worker binary

Status: coordinator preparation before test edits. Scope is a small integration-test adapter and the existing direct `pipeline::run` acceptance callers; production CLI dispatch stays on its own executable. T15 remains open until native durable-run acceptance passes.

## Reference research before code

Read `docs/reference-coding.md`, `docs/agent-tasks.md`, the T15 private worker entry and artifact lifecycle worklogs. XERJ query `project-my2pg` / `pipeline run integration tests current_exe worker executable` returned current T15 worker and artifact lifecycle evidence at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. XERJ query `ref-dmt-rs` / `worker process executable path` returned the CLI entry and signal script at pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Read `src/pipeline/mod.rs:233-260`, `src/main.rs:130-145`, `tests/cases/t08_pipeline.rs:1-95`, `tests/cases/t13_concurrency.rs:1-110`, `tests/integration.rs:1-55`, `Cargo.toml:1-24`, and `../references/dmt-rs/crates/dmt-rs-cli/src/main.rs:1-120`.

The normal `pipeline::run` resolves `current_exe`, which is the Cargo test harness when a native test calls it; only the real `my2pg` executable dispatches the private artifact-worker argument. The explicit `run_with_artifact_executable` seam preserves production behavior and lets integration tests name Cargo's exact `CARGO_BIN_EXE_my2pg`. The dmt-rs CLI owns ordinary CLI dispatch and does not solve this embedded-library test-runner problem; no reference code is copied.

Chosen adaptation: add one shared integration-only wrapper that calls the explicit seam with `env!("CARGO_BIN_EXE_my2pg")`, then route existing direct run tests through it. Keep `inspect`, `plan`, and all CLI tests unchanged. The acceptance command will execute the native fixture suite with the harness-selected features; every direct run must now reach the compatible production worker. No database run is performed by this adapter itself.

## Implementation and checks

Added `tests/support/test_pipeline.rs`, which calls the explicit trusted executable seam with Cargo's `CARGO_BIN_EXE_my2pg`. Routed the direct native pipeline callers in T08/T10/T11/T13/T15 acceptance modules through this helper; read-only `inspect`/`plan` calls and actual CLI subprocess tests remain unchanged. Registered B's T15 production-worker case under `artifact-worker-tests`.

- `bin/cargo fmt --all -- --check`: pass.
- `git diff --check`: pass.
- `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --tests --no-run`: pass; all library, CLI, artifact worker, contract, importer and integration targets compile with both mandatory feature lanes.
- `bin/cargo test --offline --locked --all-features --lib --tests -- --test-threads=1`: pass after allowing only the suite's ephemeral localhost binds. 89 library, 15 contract, 1 ordinary driver-spike, 18 importer and 9 ordinary integration tests passed; database-dependent cases stayed ignored. The sandbox-only attempt failed three T14 cases at loopback bind with `Operation not permitted`; the approved local retry passed.
- `bin/cargo clippy --offline --all-targets --all-features -- -D warnings`: pass after wrapper registration and cleanup of now-unused imports.
- B's focused `t15_durable_run::production_worker_uses_admitted_artifact_bytes_and_retains_confirmed_report`: pass (1/1, 0.48s) against the actual binary; B also passed all-target/all-feature check and strict Clippy before freezing its lease.

The wrapper makes direct integration calls compatible with the private production worker. Two `#[ignore]` library-unit panic cases still call `run()` from the Cargo unit-test executable; they need a dedicated worker-entry adaptation before they can count as native T13 evidence. T15's two database-backed new cases and full T12/T13 native regression remain open. T16 still requires independent fixture/policy work; the source-default fix and unchanged T10 test are covered in the C worklog.
