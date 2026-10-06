# MySQL to PostgreSQL rewrite plan

Prepared on **2026-10-01** against pgloader commit `231ab86778ca5ffd7de40878714760c8b4860cdf`.

Build `my2pg/` beside `pgloader/` as an independent Rust application. Keep MySQL migration behavior worth preserving; remove the multiple-source framework, JVM/Common Lisp build systems, and unrelated features from the new application.

## Reading order

| Document | Purpose |
| --- | --- |
| [Source audit](source-audit.md) | What the specs, implementations, tests, and documentation actually contain |
| [Scope and compatibility](scope-and-compatibility.md) | Supported structures, conversion rules, intentional differences, and exclusions |
| [Architecture](architecture.md) | Modules, data flow, transactions, consistency, memory, and failure handling |
| [Configuration and console](config-and-cli.md) | TOML contract, `.load` importer, commands, output, and exit codes |
| [Tests and performance](testing-and-performance.md) | Existing fixtures to reuse, missing tests, verification, and benchmark gates |
| [Implementation plan](implementation-plan.md) | Ordered milestones, dependencies, risks, and cleanup |
| [Codex tasks](agent-tasks.md) | Task IDs, file ownership, execution waves, and agent briefs |
| [Reference coding](reference-coding.md) | Installed XERJ, pinned repositories, source search, and required research before coding |
| [Checklists](checklists.md) | Evidence required to finish tasks, milestones, and a release |
| [Worklog](../worklog/README.md) | How to record progress and hand work between agents |
| [Development](development.md) | Runnable build, offline validation and disposable database commands |
| [Release builds](releases.md) | Linux/macOS archives, Docker export, experimental Windows compilation and checksums |

## Decisions proposed by this plan

1. **Only MySQL → PostgreSQL.** MariaDB and other MySQL-compatible products are outside the supported product contract. Detect recognized variants and reject them explicitly.
2. **One Cargo package**, with a small library and CLI binary. Concrete MySQL and PostgreSQL modules; no source plugins or generic database framework.
3. **TOML is the executable configuration.** A one-way `.load` importer converts a documented MySQL subset and reports unsupported clauses. One normalized configuration drives execution.
4. **Correctness before throughput.** First demonstrate a real end-to-end migration, then add full type coverage, recovery, bounded concurrency, and measured optimization.
5. **Safe, visible behavior.** Explicit destination schema, fail on existing objects by default, read-only planning, required DDL failures reflected in the exit status, and no automatic replay after an uncertain commit.
6. **An ordinary CLI with live progress.** TTY progress, readable log output when redirected, and a stable JSON event stream. A full-screen terminal application is unnecessary for the first release.

These documents define the full implementation contract. Use the task board and worklogs for current implementation evidence. The [example TOML](examples/mysql-to-postgres.toml) parses and can be checked offline after supplying its credential references. Executing a migration requires reachable endpoints, supported selected objects and a target satisfying the current planner gates; an example file alone is not runtime evidence.

## Current completion boundary

Static analysis and [XERJ setup](../worklog/2026-10-01-reference-coding.md) are complete. The [current source checkpoint](../worklog/2026-10-01-M2-existing-sequence-shutdown-checkpoint.md), `d6d00e7`, passes126 ordinary and121 fresh development-pair database cases. It integrates positive existing policies, monotonic identity handling and owned panic shutdown alongside the [earlier parallel/graph/import work](../worklog/2026-10-01-M2-parallel-graph-import-checkpoint.md). The child artifact writer is tested preparation awaiting runner integration. The [reference refresh](../worklog/2026-10-01-reference-refresh.md) records index verification; M2–M4 fidelity, fault, compatibility, server/platform matrix, performance, packaging and independent release gates remain open. The [analysis worklog](../worklog/2026-10-01-analysis.md) describes the original planning snapshot.
