# T15 — durable reports, console and verification acceptance

- Status: complete for the T15 task contract
- Agent/role: coordinator
- Task and specification links: [T15](../docs/agent-tasks.md#task-board), [M2 checklist](../docs/checklists.md#m2-reliable-core)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`

## Native acceptance

The current full serial MySQL 8.4.11 → PostgreSQL 16.15 run passed 152 cases with no failures and all 22 required IDs exactly once. Its T15 cases all passed: real CLI plain/JSON/TTY/NO_COLOR/quiet/verbose diagnostics; acknowledged COPY metrics; durable artifact admission and CLI reject accounting; supported source defaults and unknown-default non-evaluation; live ENUM catalog drift detection; count/schema, identity-state and exact default verification; and verification on the supplied snapshot. The exact fixture pins, stopped state and suite evidence are in [the T22 lane record](2026-10-02-T22-mysql84-pg16-current.md). The test ledger is `target/integration/my2pg-mysql84-pg16-480d50ec1704/rust-tests.log`.

The full all-target/all-feature Rust suite also passes 17 integration tests with 138 fixture-dependent tests ignored in that non-harness invocation; its T15 artifact writer protocol tests pass. The native harness independently executes the database-dependent T15 cases rather than counting ignored cases as passed. Strict Clippy, formatting, and all seven support-harness tests pass.

The production path routes plan, initial/final reports and reject writes through the bounded artifact worker, reserves artifact memory before target mutation, preserves only acknowledged rejection accounting, and surfaces durable writer failures. The native cases exercise the real CLI and verifier. T12's remaining physical-storage synchronization fault proof is tracked separately; this closeout makes no physical-fsync-failure claim.

T15 can close at its stated acceptance. The grouped M2 milestone stays open for T11 policy/structure breadth, T12 physical-sync/recovery breadth, and T13 resource/shutdown gates; T22 still owns cross-platform matrix acceptance.
