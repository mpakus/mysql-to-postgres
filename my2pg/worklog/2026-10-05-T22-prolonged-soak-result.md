# 2026-10-05 — T22 — prolonged T13 repeat-stability result

- Status: defined repeated-stability threshold passed; broader T22 gate remains open
- Agent/role: coordinator / integrator
- Task and acceptance criteria: [T22 task board](../docs/agent-tasks.md#task-board), [prolonged-soak criteria](2026-10-05-T22-prolonged-soak-criteria.md), [release checklist](../docs/checklists.md#release)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty; no Rust source changed during this run
- Runtime: Darwin/ARM64 host, Docker `linux/arm64`; MySQL 8.4.11 image `mysql@sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242`; PostgreSQL 16.15 image `postgres@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea`
- Fixture: `my2pg-mysql84-pg16-483d507f9edc`; stopped after the run
- Fixture inputs: MySQL SQL SHA-256 `efbe952f0397182d11caf187cc806b741abd642799a6d7ad8e554c3bf49f7c56`; PostgreSQL SQL SHA-256 `af1413b85316897eb0a81f249713131fddc929f515d21840b44cb835b1c5e504`; harness SHA-256 `96ec8f7163086460613d1dfeba6a2a7f65040b8d7ef02316a988b0cf60d83aa6`
- Executables: application SHA-256 `c3e45d02ce6dabd0eb390fbb525991d3966723adf72ce403a0f7882d08a34b9a`; integration-test binary SHA-256 `006bbc922fc637be21442753ac706f75fb8c5c76cbbeb03b4e8155de4e5be1df`

## Method

Started one uniquely owned fixture with `rtk run './tests/run-integration.sh mysql84 pg16 --start'`, sourced its generated `env.sh`, and ran the complete native T13 concurrency target 24 times serially:

```sh
./bin/cargo test --offline --locked \
  --features artifact-worker-tests,native-import-tests \
  --test integration t13_concurrency -- \
  --ignored --nocapture --test-threads=1
```

The same application and integration-test executable were used throughout. Each command's raw combined output was saved as `target/integration/my2pg-mysql84-pg16-483d507f9edc/t13-soak-NN.log`. Each target runs 11 tests, including queue/cancellation and injected worker-fault cases plus `actual_cli_fixed_budget_rss_does_not_scale_with_tenfold_source_rows`. The RSS case uses a fixed 773,648-byte application budget, migrates 20,000 and 200,000 rows per table in separate actual CLI processes, and checks exact committed rows and durable reports.

## Results

All 24 cycles passed 11/11, for 264 passed T13 test executions and zero failures. Sum of measured command wall times was 3,616.72 seconds (60 minutes, 16.72 seconds); the 24 Cargo test summaries total 3,613.67 seconds. Individual command wall times ranged from 147.74 to 154.59 seconds. All 24 logs contain the exact `11 passed; 0 failed; 0 ignored` target summary.

The 48 emitted RSS records form 24 complete 20,000/200,000-row pairs. All retain the 773,648-byte application budget. The maximum larger-profile RSS increase was 464 KiB (below the 1 MiB threshold); the minimum was −304 KiB. Every profile reported a maximum of 3 source and 3 target connections. An independent parser found zero missing logs, summaries, profiles, or acceptance violations.

| Cycle | Measured seconds | Raw log SHA-256 |
| ---: | ---: | --- |
| 01 | 152.27 | `8ac36fbf834b090162cc5633ae97ca1f59a851f995712dc514e4ab180573a6b1` |
| 02 | 153.47 | `55ee243b1bc1459e57a519032cbb6e282fa11ace61289250b71d1cf3ccedf543` |
| 03 | 153.47 | `2ccbb130126ea5319e4d6dba52e37667de67553c11914377c7f00a2e806b1543` |
| 04 | 152.57 | `c8c54c878ceddf853e472f5b506f8758aaf8e7bc72bdf08a79b1620c51e3d49f` |
| 05 | 152.64 | `627271367173300affd8d70045f6ffabb6e1067decee63267b93db8811056776` |
| 06 | 152.98 | `fe016d228a8ef06b08eefe373ed8f03ce4b0558e46b794985eef0e1639e0a75b` |
| 07 | 152.89 | `9e34101be6597428afe71e704f69443e2e78cca2ac68b6ab79bf301f32dedc5a` |
| 08 | 154.59 | `b2f4ec334fcd291797be7253c79667fa613e5025e6c0f5d637716c622aef914f` |
| 09 | 153.46 | `e170c24a62c97ca167f6dd9f46145580f3d341843fbc9db7f82c1892379072f1` |
| 10 | 153.25 | `0f601400edab1c6a55f7c94405e4dfd830b6bb433f0ce71b27d84a980e028081` |
| 11 | 148.18 | `37370fa83b21c958dbcb5902101dfdc4e0ee5c946d317d66c8b11e7fe6a89307` |
| 12 | 148.85 | `656e81876442e4d381e543ddb5f186a7b36a4452645344cef4b38b0c1e246b99` |
| 13 | 150.02 | `5de39671893a7f1600a0a26ebf798c1acf707858a72a30f492279cebef755ceb` |
| 14 | 148.78 | `d01d7f8a602818a38aa7d9e1bffe34cbf93f23f1a9554712d8ee24f89649b837` |
| 15 | 151.23 | `3c19c8ebb9da56f8ea8bddac8fd94a67981f9689fd873f75a0794933f95e9f4d` |
| 16 | 148.95 | `f2e69873fd217e3ac4407677f0a60e031d4ec94f8f4024ffd0c77fe4f3139f97` |
| 17 | 147.94 | `bd7b7e5fd7c42d803742cd8a21c643a323ee38cb026013b8a2e6253ee018cca4` |
| 18 | 149.06 | `35fa558f05c4c51ff7b0cdb222bfc3923969f5d4faa73c1341ca449c03f59d9a` |
| 19 | 147.90 | `93167b6debe8e8967b17012a508e9f69b57f300a6048ddc6c4b5eb04270fdd9b` |
| 20 | 150.40 | `bb9e031353f374c04ba234d0864f0f17c6cfeff6368a16135232b96bf9b496a7` |
| 21 | 148.11 | `0f84c15458796aae18b5e4b7393eaec252af3d8009312bc817cdc349cd2c05f0` |
| 22 | 148.33 | `99b7fb7911277fe80a67de4b10d17c201d22b8b76ec3eb93611297dcc619a7fd` |
| 23 | 147.74 | `3ee88b8944e99279bcfe034bcb54e91cc41e257b1e2614cf51c5556aef259f06` |
| 24 | 149.64 | `7c1bde26ca538768d7635c4b76084bd0247380210028ff05e5b28e6262149caf` |

The machine-readable ledger is `target/integration/my2pg-mysql84-pg16-483d507f9edc/t13-soak-summary.json`, SHA-256 `82dfc1bb025e326464f0eab50b8faf3226afdee0e00ef299517a7c71983170a6`. It includes the exact image pins, fixture/harness and executable digests, measured durations, per-cycle hashes, parsed RSS pairs, and acceptance checks.

The runner stopped exactly this fixture. `connections.json` records `state=stopped`; a post-cleanup `docker ps -a --filter label=org.my2pg.integration=true` returned no containers.

## Limits and handoff

This closes only the defined repeated-stability sub-gate for one Darwin/ARM64 host and one MySQL 8.4/PostgreSQL 16 pair. It does not provide a single-process one-hour memory trace, slow-destination coverage, every intermittent transport/filesystem fault, native AMD64/MySQL 5.7, hosted CI, or benchmark acceptance. Keep T22 and overall release status open; T21–T23 remain in progress.

## Review

- Reviewer: `/root/t24_independent_review` (independent current-artifact audit).
- Findings and resolutions: no mismatches. Reviewer matched all 24 cycle hashes/pass summaries/durations, all 48 raw log RSS observations to the 48 `rss.json` records and summary pairs, the 464 KiB maximum growth, 3/3 connection peaks, pinned fixture metadata, and executable hashes. It confirmed the acceptance scope remains limited to repeated stability and that T22/release remain open. The reviewer could not query Docker from its read-only environment; the coordinator's owned stop marked fixture metadata stopped and the post-cleanup Docker inventory returned no labeled containers.
- Task-board update/reference: T22 remains in progress for broader fault/platform/hosted gates; T24 no-go status remains unchanged.
