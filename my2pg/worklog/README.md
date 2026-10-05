# Worklog

This directory records implementation evidence and handoffs. The [task board](../docs/agent-tasks.md) is the status index; individual entries contain the proof.

Use one file per task attempt or meaningful review, named `YYYY-MM-DD-TNN-short-description.md`. Use [TEMPLATE.md](TEMPLATE.md). A later correction gets a new entry that links to the earlier one; do not rewrite historical results to look successful. Keep credentials, production rows, full environment dumps, and private connection URLs out of these files.

Record the tested code revision, exact commands, relevant runtime versions, result counts, artifact paths/digests, limits, and next dependency. Distinguish `passed`, `failed`, `not run`, and `blocked`. A test whose prerequisite is absent is not passed.

Before coding, record [reference research](../docs/reference-coding.md): project/reference query, pinned commit, original file and line, relevant test, finding and chosen adaptation. Keep unsuccessful searches visible. Ask the coordinator to refresh a stale corpus instead of changing shared indexes from a worker.

Suggested task states: `planned` → `claimed` → `in_progress` → `review` → `done`, with `blocked` available when a specific prerequisite prevents progress. The integrator updates the board after reviewing evidence. Each worker writes its own log to avoid concurrent edits to one shared journal.

For a contract change, add a short decision section: old behavior, new behavior, reason, affected tasks, and fixtures changed. For a benchmark, retain the machine-readable results in a referenced artifact location and summarize its settings and spread here.

## Entries

- [2026-10-01 — source analysis and rewrite planning](2026-10-01-analysis.md)
- [2026-10-01 — XERJ and reference coding setup](2026-10-01-reference-coding.md)
- [2026-10-01 — current reference snapshot refresh](2026-10-01-reference-refresh.md)
- [2026-10-01 — existing/sequence/shutdown checkpoint](2026-10-01-M2-existing-sequence-shutdown-checkpoint.md)
- [T11 — monotonic sequence adjustment and preserved-state verification](2026-10-01-T11-monotonic-sequences.md)
- [T11 — constraint-trigger preservation](2026-10-01-T11-constraint-trigger-preservation.md)
- [T11 — deferred-key preservation and native COMMIT regression](2026-10-01-T11-deferred-key-preservation.md)
- [T10 — native supplementary label metadata investigation](2026-10-01-T10-label-recovery.md)
- [T15 — safe COPY failure diagnostics](2026-10-01-T15-copy-diagnostics.md)
- [T01 — Rust foundation and shared contracts](2026-10-01-T01-foundation.md)
- [T02 — fixtures, harness and baseline outcomes](2026-10-01-T02-fixtures-harness.md)
- [T03/T05 — MySQL transport and driver proof](2026-10-01-T03-mysql-transport.md)
- [T04 — strict configuration](2026-10-01-T04-config.md)
- [T06 — planner and encoder](2026-10-01-T06-planner-encoder.md)
- [T07 — PostgreSQL transport](2026-10-01-T07-postgres-transport.md)
- [T08 — sequential pipeline](2026-10-01-T08-pipeline.md)
- [T09 — actual CLI acceptance](2026-10-01-T09-cli-alpha.md)
- [T10 — bounded encoder preparation](2026-10-01-T10-bounded-preparation.md)
- [T15 — artifacts and console](2026-10-01-T15-artifacts-console.md)
- [T15 — counts and schema verifier](2026-10-01-T15-counts-schema.md)
- [T15 — sequence verification](2026-10-01-T15-sequence-verifier.md)

See the task board for reviewed integration status; preparation and passing narrow tests do not complete later milestone gates.
