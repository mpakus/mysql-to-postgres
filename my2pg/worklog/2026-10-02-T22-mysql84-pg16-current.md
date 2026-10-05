# T22 — MySQL 8.4.11 to PostgreSQL 16.15 native lane

- Status: accepted native lane
- Agent/role: coordinator
- Task and specification links: [T22](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` (working tree dirty; exact build and fixture hashes are captured in the harness artifact)

## Acceptance

Ran `rtk run 'python3 tests/support/harness.py mysql84 pg16'` on Darwin/ARM64. The latest harness run completed successfully with **153 passed cases, zero failed cases, and zero unmet required cases**. All 23 required IDs were observed exactly once as `ok`, including the required planned-DDL failure, T10's empty-text and lossy ENUM/SET metadata checks, T11 identity exhaustion and policies, T12 lost-COMMIT-ack and storage uncertainty, T13 snapshot behavior, T15 artifact persistence, T16 importer behavior, T17 expression/index/ON UPDATE policies, T20 integer ranges, and the plain-PostgreSQL T17 missing-PostGIS refusal.

- Actual source/target: MySQL 8.4.11 → PostgreSQL 16.15, both pinned `linux/arm64` images.
- Image digests: MySQL `sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242`; PostgreSQL `sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea`.
- Artifact: `target/integration/my2pg-mysql84-pg16-92362583765a/rust-tests.json`; 153 passed, 23 required IDs, no failed IDs, no unmet required IDs; harness log SHA-256 `406d6489509e23bc5f7c777ddb7edf05f40a589c6770e75e2659e60c2cc0f2de`.
- Fixture metadata: `connections.json` records state `stopped`; `containers.log` records both exact server versions and successful fixture shutdown.

This adds one current full runtime cell and is not complete T22 certification. MySQL 5.7 and 8.0 pin gaps, PostgreSQL 17, PostGIS-positive and NLS/fault/platform variants, CI, fuzz/soak, and release packaging remain open.

The same run passes all 14 registered T13 concurrency, shutdown-phase, and later-table snapshot cases. Its fixed-budget RSS test compares 20,000 with 200,000 rows per table at 773,648 application bytes: peak child RSS is 28,000 KiB and 28,096 KiB respectively, with measured source/target client backends peaking at 3/3 in both samples. This is development-host evidence for the bounded queue/connection checks, not the T21 benchmark gate. T13's separate SingleSnapshot external-MDL termination edge remains open in its runner worklog.
