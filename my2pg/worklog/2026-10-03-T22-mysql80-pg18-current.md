# T22 — current-source MySQL 8.0 to PostgreSQL 18 native lane

- Status: accepted native runtime cell; T22 remains open
- Agent/role: coordinator
- Task and specification links: [T22](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` (working tree dirty; harness records exact built executable hash)

## Research and method

Before running the current-source matrix cell, read `my2pg/docs/reference-coding.md`, the MySQL-only boundary in `my2pg/docs/scope-and-compatibility.md`, T22's ownership and release checklist, the harness implementation and adjacent matrix tests. Queried the shared `project-my2pg` XERJ index for the native integration matrix, current-source runner, and T22 acceptance requirements. No reference implementation changes were needed for this test-only acceptance; pinned references and shared indexes were left untouched.

## Acceptance

Ran the repository's owned integration lane for `mysql80 pg18`. The harness completed with exit code 0: **156 cases passed, zero failed case IDs, and zero unmet required case IDs**. The independent test preparation build also exited 0.

- Actual pair: Oracle MySQL 8.0.46 → PostgreSQL 18.6, pinned `linux/arm64` images.
- MySQL digest: `sha256:213bbfaf699693a40a20a12bb4342d2589a15a3dc7153db698eaed252a92458e`.
- PostgreSQL digest: `sha256:89f747171c4b0af0eacf5984550060be79786dbe286eb60cfa691d79d1e8b23f`.
- Artifact: `target/integration/my2pg-mysql80-pg18-00faa66b9e96/rust-tests.json`; its report records all 156 runnable cases passing and every required case as `ok`.
- `connections.json` records the lane as `linux/arm64`, state `stopped`, and the unique owned Compose project `my2pg-mysql80-pg18-00faa66b9e96`.
- Preparation build executable SHA-256: `c3e45d02ce6dabd0eb390fbb525991d3966723adf72ce403a0f7882d08a34b9a`.

This adds one current-source runtime cell. It does not complete T22: MySQL 5.7, other required platform/version cells, fuzz/soak, hosted CI evidence, and release-candidate integration remain open. It also does not alter T21 comparative performance eligibility or T23 distribution/legal gates.
