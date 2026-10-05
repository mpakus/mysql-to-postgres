# 2026-10-05 — T22 — repeated T13 stability sample

- Status: eight serial T13 cycles passed; T22 soak/fault gate remains open
- Agent/role: coordinator / integrator
- Task and specification links: [T22 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Owned fixture: `my2pg-mysql84-pg16-5a1be672ad14`; state is `stopped`
- Runtime: Darwin/ARM64 host with Docker `linux/arm64`; MySQL 8.4.11 image `mysql@sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242`; PostgreSQL 16.15 image `postgres@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea`
- Fixture hashes: MySQL SQL `efbe952f0397182d11caf187cc806b741abd642799a6d7ad8e554c3bf49f7c56`; PostgreSQL SQL `af1413b85316897eb0a81f249713131fddc929f515d21840b44cb835b1c5e504`; harness `96ec8f7163086460613d1dfeba6a2a7f65040b8d7ef02316a988b0cf60d83aa6`

## Method

Started one uniquely owned fixture with `rtk run './tests/run-integration.sh mysql84 pg16 --start'`, sourced its generated `env.sh`, and ran the ignored native T13 concurrency target eight times serially:

```sh
./bin/cargo test --offline --locked \
  --features artifact-worker-tests,native-import-tests \
  --test integration t13_concurrency -- \
  --ignored --nocapture --test-threads=1
```

Each complete cycle ran 11 tests. The target includes blocked/full-queue cancellation, socket failures, SIGTERM, index-worker admission, sibling ACK handling, and `actual_cli_fixed_budget_rss_does_not_scale_with_tenfold_source_rows`. That RSS case uses a fixed 773,648-byte application budget and verifies 20,000 then 200,000 rows per table, exact committed-row totals, bounded source/target connections, and durable reports.

## Results

All eight cycles passed 11/11, for 88 passing T13 executions and zero failures. Recorded target time totals 1,219.48 seconds across the serial cycles.

| Cycle | Target seconds | RSS KiB, 20k → 200k rows/table | Source/target peak connections | Log SHA-256 |
| --- | ---: | ---: | ---: | --- |
| 01 | 152.87 | 28,144 → 28,768 | 3 / 3 | `7de364c2e992e6286d56e730aaab0844184c6656d6862d0b81287757b0d2b735` |
| 02 | 152.63 | 28,336 → 28,432 | 3 / 3 | `003337c3938d4c3897b69ec74fba7359d4423922f99a1ae4f75a6f1bae08e03c` |
| 03 | 153.28 | 28,256 → 28,560 | 3 / 3 | `14bea4f0df030817bde0609233f40d2c00d30a9a0d7d762f914665ea7b9decde` |
| 04 | 153.63 | 28,336 → 28,704 | 3 / 3 | `fc439738206d65eac80f168ef05f14d6370150d4807ccd0968905819c3e7209a` |
| 05 | 153.81 | 28,768 → 28,816 | 3 / 3 | `3279d33046fac795a6d6309181a07ed49c3f1043dfd501ff5292046175fbab67` |
| 06 | 153.16 | 28,432 → 28,560 | 3 / 3 | `8415e8bc5ce905e288e7effa76e5fbb7b4a2a345e16747e2d8af0df0ad60fe0b` |
| 07 | 151.97 | 28,224 → 28,688 | 3 / 3 | `b8ecd939d322226650eb1c3b5976e6aed1b4aae6da1fc88226510dd51648bbbe` |
| 08 | 148.13 | 28,320 → 28,768 | 3 / 3 | `8769ae8620f19a3246a97579cf8ec9d0e484ac5cc886cb54f0b859eb4218e72f` |

The per-cycle output and `rss.json` evidence are retained under `target/integration/my2pg-mysql84-pg16-5a1be672ad14/`; that directory also retains the pinned `connections.json` with `state=stopped`. `rtk run './tests/run-integration.sh --stop .../connections.json'` completed, and a post-run `docker ps -a --filter label=org.my2pg.integration=true` returned no containers.

## Limits and handoff

This is repeated serial stability evidence on one Darwin/ARM64 host and one MySQL 8.4/PostgreSQL 16 pair. It is not a prolonged soak acceptance, native AMD64/5.7 evidence, hosted CI, or a benchmark. It does not cover every intermittent transport or filesystem fault. Keep the T22 release checkbox open until its duration/fault criteria and remaining platform lanes are accepted.

- Exact per-cycle logs: `target/integration/my2pg-mysql84-pg16-5a1be672ad14/t13-stability-01.log` through `t13-stability-08.log`.
- The case itself asserts exact migrated row counts and report completion; each cycle passed those assertions.
- Next: continue remaining T22/T23 release gates and keep timing claims ineligible until T21 receives complete equivalent phase instrumentation.

## Independent review

`/root/t24_independent_review` independently matched all eight test counts, runtimes, log hashes, RSS pairs, connection peaks, fixture pins, and all 16 `rss.json` records to this worklog. It confirmed the 48–624 KiB range and that the checklist leaves prolonged soak and remaining fault breadth open. The reviewer could not repeat Docker inventory from its read-only environment; the coordinator's post-cleanup inventory returned no labeled integration containers.
