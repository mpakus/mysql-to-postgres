# T21 — release binary path in the local adapter

- Status: local release-binary correctness smoke passed; broader T21 gates remain open
- Owner: coordinator
- Task: [T21](../docs/agent-tasks.md#task-board), [benchmark contract](../docs/testing-and-performance.md#proving-faster)
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree is dirty

## Reference research before implementation

| Query and prefix | Pinned revision and evidence | Finding and adaptation |
| --- | --- | --- |
| `current performance runner --my2pg-bin loader_invocation argv path`; `project-my2pg` | `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `tests/performance/runner.py:557-584,1180-1214`; `tests/performance/test_runner.py:72-103,679-849` | The runner records `args.my2pg_bin` and its digest in the result at `runner.py:1109-1111`, but execution hard-codes `values["my2pg_bin"] = "my2pg"` at `:1184`; the local manifest's argv uses `{my2pg_bin}`. Pass the resolved override only to `local-binary` manifests; preserve the in-container executable name for `my2pg-container`. Add an argv assertion through the database-free `main()` test harness, then rerun the optimized binary against the owned disposable fixture. |
| `benchmark runner executable path loader argv process`; `project-pgloader` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; no project-local custom-binary-path analogue was returned | pgloader's adapter is container/JAR based, so it does not supply a host-binary override pattern. Keep its invocation unchanged. |
| `current performance runner --my2pg-bin loader_invocation argv path`; `ref-dmt-rs` | dmt-rs pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; returned CLI Cargo manifest, no matching executable-path injection pattern | This reference does not address the runner seam; do not adapt its multi-crate CLI structure. |

Read the current performance specification, runner, local and container manifests, and adjacent invocation/main tests. RTK and Ponytail guidance were applied. The first live release-binary attempt returned `No such file or directory: 'my2pg'` before a loader could run, matching the static path mismatch. Keep the fix narrow and preserve the container loader's `/`-rooted executable convention.

Follow-up XERJ query `T21 local release runner host CA file config TLS path MY2PG_TLS_CA` against `project-my2pg` returned the runner and T21 setup evidence at the same revision. Query `TLS root certificate file path client config rustls postgres` against `ref-rust-postgres` at `1084ca8f5b5302e161892f2fa40abf71b4060c10` returned its client configuration docs but no runner-path pattern to reuse. Read `tests/performance/runner.py:1178-1215`, `tests/performance/loaders.example.json`, and `tests/support/harness.py:540-568`: the runner uses `/tls/ca.pem` for both local and container invocations, while the host harness exports the real CA as `metadata.env.MY2PG_TLS_CA`. A second live attempt reached the release binary and failed before database work with `source.ca_file: cannot access the referenced file`. Adapt local-binary configs to the harness's host CA path while retaining `/tls/ca.pem` for container execution; add a database-free regression for both argv and generated CA path before the next live retry.

Follow-up query `T21 local runner report directory run_dir artifact path report_path_for_run` against `project-my2pg` returned `tests/performance/runner.py:541` and adjacent invocation code. The retained `migration.toml` confirms `report.directory = "/bench/report"`; that path is a container mount and not writable/meaningful to a host executable. The release run exited 1 with `migration artifact or console I/O failed`, and no report was produced. `ref-rust-postgres` COPY/output queries returned database-driver examples, not a CLI artifact-directory abstraction, so reuse the runner's existing per-run `report/` directory instead of adding a new abstraction. Change only the local-binary report path to `run_dir / "report"`, keep `/bench/report` for container runs, and assert all three host paths (executable, CA, report directory) in the existing mocked-main test.

## Acceptance criteria

- Unit regression passes: local-binary `{my2pg_bin}` expands to the resolved `--my2pg-bin` path and local config contains host CA/report paths; container argv and mount paths remain unchanged.
- The focused performance-runner unit suite, Python compilation and diff check pass. The release-profile smoke workload passes against the explicit local-binary manifest.
- This validates local release execution only; it does not close T21 phase/resource equivalence, repeated benchmark, large-profile, or dedicated-host gates.

## Implementation and verification

The local-binary manifest now receives the resolved `--my2pg-bin` path, the host fixture CA path, and the per-run report directory. Container manifests retain their container executable and `/tls/ca.pem` and `/bench/report` mount paths. The existing database-free `runner.main()` test exercises the actual local-binary argv and generated CA/report paths; container-path tests remain in the same 37-test suite.

- `rtk run 'bin/cargo build --release --locked --offline'` — passed; optimized macOS ARM64 binary SHA-256 `614da7876e1ab89be10ac64e9b4fdd94386fa711eb0e268ced314441cb1e92b9`.
- `rtk run 'python3 -m unittest tests/performance/test_runner.py -v'` — **37 passed**; Python compilation and `rtk git diff --check` passed.
- Started a uniquely owned MySQL 8.4.11/PostgreSQL 16.15 fixture on the `linux/arm64` database images, then ran `tests/performance/runner.py` with `--my2pg-bin` set to the release executable, the explicit local-binary manifest, `one_large_narrow_integer`, smoke profile, and `--correctness-only`.
- Result `target/integration/my2pg-mysql84-pg16-e96dd8ddad8c/t21/one_large_narrow_integer-smoke-b40691d5719d/results.json`: completed, exit 0, correctness passed, timing ineligible. The independent source and target oracles matched all 1,000 rows, ordered digest `f8406d8e2108a42b5ead973406de6916a204cfcb47ec49ec6fb83cfe16bc05c1`, aggregate, columns/nullability, primary key, table count and foreign-key count. The durable migration report records 1,000 committed rows and complete status.
- Results SHA-256 `9aec421fb0d4ebd6696d288ea1ac695efb79aee0794905ca4973e9ebedfe95fc`; migration report SHA-256 `26984a9402f893ad151baab272204e3b086c28a3383bffe434a3185824f42903`; loader log SHA-256 `ff15adb6fe4ac0f1d46134fb53df32bc4b0d7d9bff2b9105f8861a491d4f62ff`.
- Stopped the exact harness-owned Compose project. `connections.json` records `state=stopped`; an independent Docker query for that project returned no containers.

This closes local invocation and one macOS ARM64 release-binary correctness smoke only. `verification.mode=none` in the app report; correctness was independently checked by the benchmark runner's source/target oracle. No timing was accepted. Comparable phase semantics, adapter resource/control equivalence, repeat/large profiles and a dedicated idle host remain open.

## Current-source release smoke on 2026-10-04

- Rebuilt with `rtk test ./bin/cargo build --release --locked --offline --bin my2pg` — passed. The current executable reports `my2pg 0.1.0`, exposes only the MySQL-to-PostgreSQL CLI scope, and has SHA-256 `614da7876e1ab89be10ac64e9b4fdd94386fa711eb0e268ced314441cb1e92b9`.
- Ran `rtk test env TMPDIR=/private/tmp PYTHONPYCACHEPREFIX=/private/tmp/t21-release-smoke-pycache python3 tests/performance/runner.py target/integration/my2pg-mysql84-pg16-751835f4e51b/connections.json --loaders tests/performance/loaders.example.json --my2pg-bin /Users/renatibragimov/www/pg/my2pg/target/release/my2pg --workload one_large_narrow_integer --profile smoke --correctness-only` against owned native Linux/ARM64 MySQL 8.4.11/PostgreSQL 16.15 fixtures.
- Result: `completed-correctness-only`, exit 0, one release-binary run, independent correctness passed for all 1,000 rows. Ordered digest `f8406d8e2108a42b5ead973406de6916a204cfcb47ec49ec6fb83cfe16bc05c1`; source/target aggregates, target columns and primary key matched. Timing is explicitly ineligible and zero measured runs were recorded.
- Result JSON SHA-256 `bc72841aca506ca66ecd44f1cb182ca04817ff3408c6177dbd83e132525f799c`; durable migration report SHA-256 `732ed0444500e9ecb9643e39dbc561b5be0ade55257b8882f26b4ec9c0e02087`; loader log SHA-256 `fb4a1840d03583269d82cd5a2aa8f4ef1100db1d951946ba1921ede98fdaf0b0`.
- Stopped the exact fixture through `tests/run-integration.sh --stop`; metadata records `stopped`, and the post-run labeled-container inventory was empty. This refreshes only the macOS ARM64 release-binary smoke; native Linux/distributed artifacts and all comparative timing/RSS gates remain open.
