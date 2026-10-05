# T22 — native database lanes in CI

Status: workflow change prepared; hosted-runner execution is pending.

## Baseline and ownership

- Coordinator owns `.github/workflows/ci.yml` wiring. Database lanes use the existing `tests/run-integration.sh` and `tests/support/harness.py`; no second harness or test list is added.
- Project HEAD before changes: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. The worktree is already dirty; this task edits only `.github/workflows/ci.yml`, this worklog, and the task/checklist docs after validation.
- Applied `rtk run` to repository/tool commands and read Ponytail guidance at `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`. The change reuses the existing harness and does not add a dependency or CI action.

## Reference research before editing

- XERJ project query: `project-my2pg "integration CI native test runner matrix" -k 5 --full 50`. It returned `worklog/2026-10-02-T22-current-required-lane-refreshes.md`, the matrix preflight and lane reports at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- Pinned reference query: `ref-dmt-rs "GitHub Actions cargo test workflow" -k 5 --full 70`. It returned the reference release workflow at commit `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; the query was broad and the original workflow was read directly before adaptation.
- Read local CI `.github/workflows/ci.yml`, `tests/run-integration.sh`, `tests/support/harness.py:1-60, 135-240, 340-445`, `tests/support/test_harness.py`, `tests/compose/compose.yml`, `tests/compose/images.json`, and the integration-lane contract in `docs/testing-and-performance.md`.
- Existing harness requires host architecture to match the pinned image platform, uses reviewed immutable arm64 images for `mysql84/pg16` and `mysql80/pg18`, fetches dependencies indirectly only from the Cargo cache, then intentionally builds native tests with `--offline --locked` and saves required-case evidence before owned cleanup.
- Pinned-reference adaptation: use the project's existing SHA-pinned checkout and its small matrix-based job shape; do not copy dmt-rs release/build logic. Add no artifact action or new package. A matching native runner and the current harness are the narrowest route to the two PR database lanes required by the project plan.
- Current official GitHub runner documentation lists `ubuntu-24.04-arm` as an arm64 GitHub-hosted runner. The matching runner image documentation lists Docker and Docker Compose. Links: <https://docs.github.com/en/actions/reference/runners/github-hosted-runners> and <https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Arm64-Readme.md>.
- A local all-target/all-feature run initially failed in the sandbox: three T14 tests could not bind loopback sockets, and T15's production artifact-worker test returned `Busy` while another test binary in the same process could own the singleton worker. The original worker uses one process-global child slot (`src/report/artifact_io.rs:633-676, 938-947`); adjacent T15 tests serialize their own cases with a module-local lock, which does not cover `t15_durable_run.rs`. An escalated local rerun with `--test-threads=1` passed all database-free tests and kept native database cases ignored. The PR's offline test command must also be serialized.

## Adaptation and acceptance

Add a two-entry native arm64 CI matrix for the documented every-PR lanes: MySQL 8.4 → PostgreSQL 16 and MySQL 8.0 → PostgreSQL 18. Install and select the pinned Rust toolchain explicitly. Run `cargo fetch --locked` before the harness because its native build uses offline mode, then invoke the owned lane runner. Serialize the offline tests too, because their test harnesses share a process-global artifact worker. Keep current PR workflow permissions read-only. Do not claim this change is an observed hosted CI pass; that requires a GitHub Actions execution at a pushed revision.

Acceptance: parse/validate the workflow, confirm exact lane arguments and `RUSTUP_TOOLCHAIN`, run the current offline harness/unit checks available here, and retain existing native-lane evidence. No live fixture can run on this host while Docker socket access is denied.

## Validation

- Ruby's standard YAML parser accepted `.github/workflows/ci.yml`; the parsed matrix contains exactly `mysql84/pg16` and `mysql80/pg18`. The manifest confirms all four pins are `linux/arm64`, matching `ubuntu-24.04-arm`.
- `RUSTUP_TOOLCHAIN=1.99.0 ./bin/cargo --version` reported Cargo `1.99.0`.
- `./bin/cargo fmt --all -- --check` passed; `./bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings` passed.
- The initial default-parallel all-target test run, inside the sandbox, failed three local-listener tests with `PermissionDenied` and one artifact-worker test with `ArtifactFailure::Busy`. This is retained as a failed diagnostic run, not accepted evidence.
- `./bin/cargo test --offline --locked --all-targets --all-features -- --test-threads=1` passed when run with local socket access enabled: library 126 passed/3 ignored, binary 1 passed, contracts 15 passed, driver 1 passed/11 ignored, importer 18 passed, integration 17 passed/142 ignored, and NLS 0 passed/1 ignored. No database integration cases were enabled by this command.
- `python3 tests/support/fixture_manifest.py` passed with 13 `.load` files, 18 parser cases, 19 assertion cases and 65 case IDs; `python3 tests/support/test_harness.py` passed 7/7; `git diff --check` passed.
- `actionlint` is not installed. No hosted GitHub Actions execution can be triggered from this local workspace; native database CI remains pending an Actions run at a pushed revision. The current host's Docker socket is unavailable, so no native lane was started from this task.
- The coordinator refreshed only `project-my2pg` after the workflow/task-board edits; it indexed 386 files into 1,348 passages with 11 exclusions. A final refresh follows this worklog update. Pinned reference indexes were not changed.
