# T22 — current-source MySQL 8.0/8.4 matrix reruns

Status: five base matrix cells have current dirty-source reruns recorded here; the companion PR-lane worklog supplies the sixth MySQL 8.0/8.4 → PostgreSQL 16/17/18 cell. MySQL 5.7 and wider platform/release gates remain open.

## Run contract

- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, with uncommitted implementation changes present.
- Host: Darwin/ARM64; pinned database images ran as native `linux/arm64` containers.
- Each run used `rtk run ./tests/run-integration.sh <mysql-lane> <postgres-lane>`, the harness's serial complete native suite, and its uniquely owned fixture project. The harness recorded normal executable/build evidence, case IDs, and cleanup state.
- No application code or fixture changed during these runs.

## Results

| Lane | Server versions | Image pin suffixes (MySQL / PostgreSQL) | Result | Test artifact SHA-256 | Log SHA-256 |
| --- | --- | --- | --- | --- | --- |
| MySQL 8.0 → PostgreSQL 16 | 8.0.46 / 16.15 | `213bbfaf6996` / `721873c34ceb` | Exit 0; 156 passed; all 26 required IDs passed exactly once; 0 unmet | `5d50baafdb77497d9388bde41c3e5f803b4e2e2752fc2c203b0431f262738d0c` | `4c5cd11be5ea1250de88c6fa45db74f189bf4ebfad1c1a91d7073bb47fba5279` |
| MySQL 8.0 → PostgreSQL 17 | 8.0.46 / 17.11 | `213bbfaf6996` / `0b2882c46a2b` | Exit 0; 156 passed; all 25 required IDs passed exactly once; 0 unmet | `c6fd9affc03b52198d00bfcabac74e7872bfe12da500e7e98a64ff06b4e327e5` | `a61a66602e654f34a608c4249c683a919f78008f36d0108bea55a5509a999e12` |
| MySQL 8.4 → PostgreSQL 17 | 8.4.11 / 17.11 | `6ea90827b110` / `0b2882c46a2b` | Exit 0; 156 passed; all 25 required IDs passed exactly once; 0 unmet | `789fb3712cc1c02b5680d90c85d8e3abcef6f7a727e85f76478856f57558941f` | `28394b26b972c83250a00463ba1805f7b21475bc87620991494a2cfc2399a4c5` |
| MySQL 8.4 → PostgreSQL 18 | 8.4.11 / 18.6 | `6ea90827b110` / `89f747171c4b` | Exit 0; 156 passed; all 25 required IDs passed exactly once; 0 unmet | `1b09156d73549aefd0da00fe124f169cf553123c8258aa4e9618f1b7c09f278a` | `34b1af5490094e283416e5c735f7183db58ae11c85f5606f7a8edf1fcdf04d23` |

Artifacts:

- `target/integration/my2pg-mysql80-pg16-78f7eb39e995/`
- `target/integration/my2pg-mysql80-pg17-886e02ff7dfa/`
- `target/integration/my2pg-mysql84-pg17-1a0706d7fe8c/`
- `target/integration/my2pg-mysql84-pg18-a65a9631cd9a/`

All four connection records say `stopped`. A post-run Docker inspection showed only the pre-existing Strangler services. Together with the current local PR lanes at [T22 PR reruns](2026-10-02-T22-current-pr-lanes.md), this supplies current dirty-tree evidence for all six MySQL 8.0/8.4 → PostgreSQL 16/17/18 base cells. It does not cover MySQL 5.7, hosted Actions, other native architectures, Linux physical-sync faults, soak/fuzz, comparative benchmarks, or release packaging acceptance.
# 2026-10-04 current-source lane follow-up

- Source revision: Git HEAD `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, dirty worktree.
- Command: `rtk test ./tests/run-integration.sh mysql84 pg16` using the current `./bin/cargo` wrapper and repository-pinned Rust 1.99.0 toolchain.
- Runtime: Darwin/ARM64 host; MySQL 8.4.11 and PostgreSQL 16.15 from the exact `linux/arm64` pins in `tests/compose/images.json`; disposable project `my2pg-mysql84-pg16-4aa749211489`.
- Result: exit 0, 156 passed, no failed or unmet required/runnable IDs, no unclassified IDs. Result JSON SHA-256 `2deac1bf2cd13bcc90be77541b8ba36cbd0af4142744eb6479cdeaf26879f9ea`; test log SHA-256 `b07383dce7d9a6cee1be6dd38ffc9fd6e8848fa7e7509e417be5c8ed5beacd9a`; current production test executable SHA-256 `c3e45d02ce6dabd0eb390fbb525991d3966723adf72ce403a0f7882d08a34b9a`.
- Harness metadata is stopped and contains the pinned image identities and fixture digests; post-run Docker inventory found no remaining containers labeled `org.my2pg.integration=true`. Existing Strangler Compose containers were left running.
- This refresh adds current-source evidence for the 8.4→16 cell only. It does not establish hosted execution, MySQL 5.7 support, performance acceptance, or release readiness.
