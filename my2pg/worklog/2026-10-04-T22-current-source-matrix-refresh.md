# 2026-10-04 — T22 — current-source ARM64 matrix refresh

- Status: six native ARM64 base cells accepted at the current Rust executable; T22 remains open
- Agent/role: coordinator / integrator
- Task and specification links: [T22 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release), [matrix contract](../docs/testing-and-performance.md#test-harness-and-ci)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Base revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the working tree is dirty
- Tested executable SHA-256 across all six cells: `c3e45d02ce6dabd0eb390fbb525991d3966723adf72ce403a0f7882d08a34b9a`
- Dependencies: T22 applicability review and platform-aware runnable-case enforcement are complete; the harness requires every runnable case exactly once and every applicable required ID exactly once
- Changes: no application, fixture, or harness code changed; this entry and checklist reconciliation record current runtime evidence

## Method and pinned inputs

Ran `rtk test ./tests/run-integration.sh <mysql-lane> <postgres-lane>` serially on the Darwin/ARM64 host. Each command created a uniquely owned Compose project, prepared the current executable, ran the full native integration target with `--test-threads=1`, saved `rust-tests.json` and its raw log, then stopped the project. Each saved `connections.json` now says `state=stopped`; a post-run inventory found no remaining `org.my2pg.integration=true` containers.

- MySQL 8.0.46 image: `mysql@sha256:213bbfaf699693a40a20a12bb4342d2589a15a3dc7153db698eaed252a92458e` (`linux/arm64`).
- MySQL 8.4 image: `mysql@sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242` (`linux/arm64`).
- PostgreSQL 16 image: `postgres@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea` (`linux/arm64`).
- PostgreSQL 17 image: `postgres@sha256:0b2882c46a2b8d8e148431b333394348cf4d7f15aec6df7f17d51dc641e56cfa` (`linux/arm64`).
- PostgreSQL 18 image: `postgres@sha256:89f747171c4b0af0eacf5984550060be79786dbe286eb60cfa691d79d1e8b23f` (`linux/arm64`).
- Shared fixture hashes: MySQL SQL `efbe952f0397182d11caf187cc806b741abd642799a6d7ad8e554c3bf49f7c56`; PostgreSQL SQL `af1413b85316897eb0a81f249713131fddc929f515d21840b44cb835b1c5e504`.
- Shared harness SHA-256: `96ec8f7163086460613d1dfeba6a2a7f65040b8d7ef02316a988b0cf60d83aa6`.

## Results

Every cell passed 156 cases with zero failed IDs, zero unmet runnable IDs, zero unclassified IDs, and zero unmet required IDs. Required-case totals are 26 for PostgreSQL 16 and 25 for PostgreSQL 17/18. The exact executable hash was identical for the five freshly rebuilt/current runs and the previously verified 8.4→16 lane.

| Cell | Isolated project | `rust-tests.json` SHA-256 | Raw test log SHA-256 |
| --- | --- | --- | --- |
| MySQL 8.0 → PostgreSQL 16 | `my2pg-mysql80-pg16-c7c9aad44873` | `7654957e923da7a95554666409d870edaa0f64ccb47fd5fde1e596e71eeaaa09` | `616d437e48b728bb85371030cb32e2f8055eddc11264f71dda09ebee6d070145` |
| MySQL 8.0 → PostgreSQL 17 | `my2pg-mysql80-pg17-73c3d37adc70` | `5935792936e838e85b0f513c58635c1da68cb035359ba015b4a837f7466edc73` | `a5a15cfd2e8f55cea8cf43728fb61260bd71690d24024524d7cd87d5903cfc4a` |
| MySQL 8.0 → PostgreSQL 18 | `my2pg-mysql80-pg18-9dba744b6ef3` | `f44e5aa1ff39464b3a8664cbcfee312f798599a6355a1324bc7a00dfecd149ba` | `f095c5cb47dd50d07531f3d17a91e6956bb5b756084c4e55f4eef733fd3ac460` |
| MySQL 8.4 → PostgreSQL 16 | `my2pg-mysql84-pg16-4aa749211489` | `2deac1bf2cd13bcc90be77541b8ba36cbd0af4142744eb6479cdeaf26879f9ea` | `b07383dce7d9a6cee1be6dd38ffc9fd6e8848fa7e7509e417be5c8ed5beacd9a` |
| MySQL 8.4 → PostgreSQL 17 | `my2pg-mysql84-pg17-84107cd497ac` | `eadf1ff98cb898b24f4ae2cefdde257931ee3e147e9a3c69e011695545cda5e6` | `47f5ce94ecb8516d8d5593ea4204184aa25f376d9e911ffc301e4540612ac1f2` |
| MySQL 8.4 → PostgreSQL 18 | `my2pg-mysql84-pg18-46b87cea2d83` | `6f0fc4ad19ba5b3d353631a4b564f6c54bfaaf40b5db7302cc2e01e94eb399cf` | `62a66712c639d6a6ac394d75eaa19901a557e087536a640357c04e298dc14f70` |

All six `rust-tests.json` files record the tested revision as `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` and `working_tree_dirty=true`; all records identify the same generated executable and harness/fixture hashes above. The artifacts live under `target/integration/` and are ignored by the source package.

## Acceptance limits and handoff

This refresh closes only the six MySQL 8.0/8.4 × PostgreSQL 16/17/18 base cells on Darwin/ARM64. It does not prove MySQL 5.7, native Linux/AMD64, hosted Actions, the separate NLS/PostGIS lanes, soak/fault breadth, T21 comparable timing, release distribution, or a real user's migration. The MySQL 5.7 x64 workflow and the other release gates remain open.

- `rtk git diff --check` passed.
- After this evidence entry and the T21 phase-event regression were recorded, the coordinator refreshed the project XERJ corpus: `project-my2pg-source`, 435 files, 1,526 passages, 14 exclusions. Query `project-my2pg "T22 uniform current-source ARM64 matrix refresh executable hash"` returned this report and the linked checklist at revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the T21 query also returned its new timing worklog. See the [release checklist](../docs/checklists.md#release).
- Next: continue the remaining external/runtime gates; refresh the shared project index only after later source or evidence changes are ready for indexing.
