# Development and current acceptance

Run from `my2pg/`. The [task board](agent-tasks.md) and task worklogs identify implemented behavior and remaining gates; the command list alone is not a release claim.

Rust 1.99.0 is pinned in `rust-toolchain.toml`, including rustfmt and Clippy. `bin/cargo` uses the coordinator-installed sibling `.toolchains/` runtime when present, otherwise the normal Cargo on PATH. Install the pinned toolchain through [official rustup](https://rustup.rs/) on another machine; generated toolchains, build outputs, credentials and per-run artifacts are ignored.

```sh
rtk run './bin/cargo build --locked'
rtk run './bin/cargo fmt --all -- --check'
rtk run './bin/cargo clippy --locked --all-targets --all-features -- -D warnings'
rtk run './bin/cargo test --locked --all-targets --all-features'
rtk run './bin/cargo run --locked -- --help'
```

Ordinary Cargo tests explicitly ignore the database cases. They prove no database result until the required lane below runs those cases. Automatic test discovery includes all test binaries; a requested database lane fails if dependencies, endpoints or passing cases are absent.

```sh
rtk run './tests/run-integration.sh mysql84 pg16'
rtk run 'python3 tests/support/fixture_manifest.py'
rtk run 'python3 tests/support/test_harness.py'
```

The harness requires Docker Compose and OpenSSL, starts a unique pair of pinned native arm64 containers, checks deterministic seeds and TLS, runs every ignored database test, preserves logs/digests under `target/integration/`, and stops only its own resources. Read [harness usage](../tests/compose/README.md) before keeping a worker fixture alive. MySQL 5.7/8.0 and PostgreSQL 17/18 lanes remain acceptance work; requesting an unpinned lane fails explicitly.

The runnable offline command is `target/debug/my2pg check docs/examples/mysql-to-postgres.toml --output json`. Set the example's `MY2PG_MYSQL_URL` and `MY2PG_POSTGRES_URL` references to valid network URLs first. `check` opens no database connection and prints references, never resolved passwords. Relative credential/certificate/hook/report paths resolve beside the configuration file. Unknown fields, unsupported source schemes, contradictory policies and unsafe limits produce status2. Output I/O failure produces status1.

Before every code change, use [reference coding](reference-coding.md) and record evidence in the task worklog. The coordinator owns dependencies, module wiring, task status and shared index refreshes. Format only your owned files during parallel editing; coordinate whole-tree mutations.

Current CI defines the offline build checks. A workflow file is not evidence that a remote CI job ran. Database CI lanes, full server/platform matrix, faults/soak, release packaging, content reconciliation and speed/RSS gates remain required before v1 acceptance.
