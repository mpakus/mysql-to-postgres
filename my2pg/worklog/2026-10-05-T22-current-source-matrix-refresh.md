# 2026-10-05 — T22 — current-source six-cell matrix refresh

- Status: six Darwin/ARM64 base cells passed at one production executable hash; T22 and release gates remain open
- Agent/role: coordinator / integrator
- Task and specification links: [T22 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release), [matrix contract](../docs/testing-and-performance.md#test-harness-and-ci)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Production executable SHA-256 across all six cells: `7e5ba49d63c03b652863b1c89fd56af5b5f4a6556c5a0a031f4c89e1b74a6c57`

## Method and pinned inputs

Ran `rtk test ./tests/run-integration.sh <mysql-lane> <postgres-lane>` serially for MySQL 8.0.46 and 8.4.11 crossed with PostgreSQL 16.15, 17.11, and 18.6. Each run built the current integration test executable, selected the complete native database test target, required every runnable case exactly once and every applicable required ID exactly once, and saved a ledger and raw log. The `connections.json` for each accepted artifact records `state=stopped`.

The first six-cell pass found one failure in `t13_snapshot_mdl::cancellation_terminates_single_snapshot_reader_blocked_during_preflight`: the test observer decoded nullable MySQL `PROCESSLIST.STATE` as `String`. The diagnosis, XERJ searches, pinned `mysql_async` evidence, selected adaptation, and repair verification are recorded in the [nullable processlist worklog](2026-10-05-T22-single-snapshot-nullable-processlist.md). The failed ledger is retained but excluded from acceptance. After the test-only fix, all five other cells were rerun on the final integration-test source snapshot; the repaired MySQL 8.4→PostgreSQL 18 cell itself passed all 156 cases.

Immutable image pins, platform, fixture SQL hashes, and harness checksum are the same as the [2026-10-04 matrix refresh](2026-10-04-T22-current-source-matrix-refresh.md): MySQL 8.0/8.4 and PostgreSQL 16/17/18 `linux/arm64` images, fixture hashes `efbe952f0397182d11caf187cc806b741abd642799a6d7ad8e554c3bf49f7c56` and `af1413b85316897eb0a81f249713131fddc929f515d21840b44cb835b1c5e504`, harness SHA-256 `96ec8f7163086460613d1dfeba6a2a7f65040b8d7ef02316a988b0cf60d83aa6`.

## Results

Every accepted cell exited 0 after 156 passing cases, with zero failed case IDs, zero unmet required IDs, zero unclassified IDs, and zero not-applicable cases run unexpectedly. PostgreSQL 16 lanes required 26 IDs; PostgreSQL 17/18 lanes required 25. All six ledgers record the same production executable hash above, source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, and `working_tree_dirty=true`.

| Cell | Stopped fixture project | Result-ledger SHA-256 | Raw-log SHA-256 |
| --- | --- | --- | --- |
| MySQL 8.0 → PostgreSQL 16 | `my2pg-mysql80-pg16-89b19248cd34` | `80d5e79cfbda039705bf077405a97cc77d1e6dfbb8ea4210453066bc76108f81` | `e549e5dd2fa73b8eed4f52228ceb9222af7fc67ee3b0ed38048183e7f84f3e3f` |
| MySQL 8.0 → PostgreSQL 17 | `my2pg-mysql80-pg17-cab147d59611` | `38a296b206fe674d1063a33c3d65d76450e22b69163e1cb24261bb2a27acc362` | `cbb23065fce20c05b8b6ecca424d49acbf97969dbe12dfaa008f99752c67f93a` |
| MySQL 8.0 → PostgreSQL 18 | `my2pg-mysql80-pg18-ee3140899253` | `831787324f411ec9b1a833a155b4f575d05ac64be315783004ed92d62d63cc78` | `275c16b0899a5cfcbfa60a4cbdc834cb37d415475b3177c11234e4790dbc9463` |
| MySQL 8.4 → PostgreSQL 16 | `my2pg-mysql84-pg16-2cc4013ca64b` | `453dd5d606924662316a84e00a9ba9abac7609d13e355892734bdf9e9c839e52` | `3f485e6e2f60a778a1b7aae88d953a6ea1e907ec7160b5bfda7b1837e58f425b` |
| MySQL 8.4 → PostgreSQL 17 | `my2pg-mysql84-pg17-4d34356d0a36` | `54db244f2377a0b0b8b1e7775027e9c9fd31c72e46ff92e3f7bf49499c33a5c4` | `80eaf688ae0abf4e7b43d264709e9e95d0afe162b59d55492813202762e9b716` |
| MySQL 8.4 → PostgreSQL 18 | `my2pg-mysql84-pg18-3a85373dda7a` | `8a4d006d1fcee9b7d2ebb194b10cfd02735baf73bdfa5d697084e9de8280e2d3` | `67221155de418d5bd2848ea678ea8deddf43924da6073b549623c784b6d8a6af` |

The primary matrix claim is the six `rust-tests.json` ledgers plus their raw logs and stopped connection records under `target/integration/`. All cell IDs and hashes above were read back from those artifacts after the runs.

## Acceptance limits and handoff

This refresh supplies current-source local evidence for the six available Darwin/ARM64 MySQL 8.0/8.4 × PostgreSQL 16/17/18 base cells. It does not prove native MySQL 5.7/AMD64, native Linux, hosted Actions, optional NLS/PostGIS lanes, broader cross-platform soak/fault coverage, T21 benchmark acceptance, legal review, distribution checksums, or real-user migration readiness. T22/T23 and the integrated release checklist remain open for those items.

`rtk git diff --check`, Cargo formatting, integration-target compilation, and strict integration-target Clippy passed on the current source. The coordinator must refresh `project-my2pg` after this worklog/checklist update; immutable reference indexes remain unchanged.
