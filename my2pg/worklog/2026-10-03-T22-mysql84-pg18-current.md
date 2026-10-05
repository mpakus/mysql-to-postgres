# T22 — current-source MySQL 8.4 to PostgreSQL 18 native lane

- Status: accepted current-source native runtime cell; T22 remains open
- Agent/role: coordinator
- Task and specification links: [T22](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` (working tree dirty; exact executable and fixture evidence retained by the harness)

## Research and method

This was a test-only refresh. Read the reference-coding workflow, MySQL-only scope, T22 ownership/checklist, matrix image pins, native harness ownership and cleanup, and the prior PostgreSQL 18 compatibility evidence. Queried the shared project XERJ index for T22's accepted matrix cells and current-source native-run records. No production or fixture code changed, and pinned references and shared indexes were not modified during the run.

Earlier PostgreSQL 18 compatibility evidence records a 138-case pass, and the current-source matrix rerun worklog already records a 156-case pass for this cell. This independent rerun reconfirms the same current-source cell and preserves a fresh, uniquely owned fixture artifact.

## Native result

Ran `rtk run 'python3 tests/support/harness.py mysql84 pg18'` on Darwin/ARM64. The owned fresh fixture ran Oracle MySQL 8.4.11 → PostgreSQL 18.6 using the reviewed `linux/arm64` pins. The harness exited 0: **156 runnable cases passed, zero failed case IDs, zero unmet runnable IDs, and zero unmet required IDs**. Preparation/build also completed successfully. The harness recorded the unique Compose project as stopped after cleanup.

- Artifact: `target/integration/my2pg-mysql84-pg18-cb6f0bb240cd/`
- Test result: `rust-tests.json`; test log: `rust-tests.log`; fixture state: `connections.json` (`state=stopped`, `architecture=arm64`, MySQL `8.4.11`, PostgreSQL `18.6`).
- MySQL image digest: `sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242`.
- PostgreSQL image digest: `sha256:89f747171c4b0af0eacf5984550060be79786dbe286eb60cfa691d79d1e8b23f`.
- Test log SHA-256: `93ad5d3e7696696195359d8954f16f2917b7070bd1be452a7d5ce68cbaee5afa`.
- Test result JSON SHA-256: `4961569393abde6b861c9be21f803febc50c7a8e8fe651e1b32e87f0a4dede6d`.
- Build JSON SHA-256: `5ae13f3dd74a8bedb9e045bf31ecf9f0f0dd27eb7307acae95762ab225d6fb31`.
- Cargo.lock SHA-256: `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`.

This accepts the current-source ARM64 MySQL 8.4 → PostgreSQL 18 runtime cell. It does not complete T22 version/platform certification or the separate NLS, fault, fuzz/soak, hosted CI, or release gates.
