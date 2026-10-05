# 2026-10-05 — T21 large-profile failure evidence reconciliation

- Status: first-attempt failure stage corrected; a later correctness-only retry passes the 10-million-row narrow-integer profile; signal-9 cause remains unknown
- Agent/role: coordinator / evidence audit
- Task and specification: [T21 task board](../docs/agent-tasks.md#task-board), [large-profile benchmark contract](../docs/testing-and-performance.md#proving-faster)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Tested source/index revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Database artifact: `target/integration/my2pg-mysql84-pg16-62f0cae44040`, MySQL 8.4.11/PostgreSQL 16.15, owned fixture state recorded as stopped

## Research and inspection

- XERJ project query: `target_oracle target_canonical_query ORDER BY id stream_digest database memory`; current indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` returned `tests/performance/runner.py:280-342` and `:405-444` for target canonicalization and target-oracle execution, plus adjacent runner tests.
- A second project query, `T21 benchmark phase event measurement COPY index verification global clock`, returned the fail-closed T21 comparator and prior phase-feasibility audits. Pinned `project-pgloader` queries returned the original Clojure timing summary and index/COPY overlap analysis at pgloader revision `231ab86778ca5ffd7de40878714760c8b4860cdf`; this documentation reconciliation changes no loader or benchmark code.
- Read the retained `worklog/2026-10-02-T21-benchmark-runner.md:352-359`, actual loader/container logs, `tests/performance/runner.py:280-342,405-444`, `tests/performance/test_runner.py` oracle tests, and `tests/compose/compose.yml:1-49`.

## Evidence and correction

The old worklog and testing guide said the 10-million-row run exited during the MySQL source-oracle query. The retained artifact contradicts that description:

- `loader.log` shows pinned pgloader copied all 10,000,000 rows in 15.619 seconds, completed its normal index/constraint steps, and exited through its summary path.
- `containers.log` says PostgreSQL backend PID 554 was terminated by signal 9 while running `SELECT id::text || ',' || bucket::text || ',' || measure::text || ',' || payload::text FROM t21_pgloader_v4_pinned_02405c30.t21_integer_rows ORDER BY id`. This is the target canonical-content oracle used after the loader, not the MySQL source oracle.
- The fixture config declares MySQL at 1 CPU/1 GiB and PostgreSQL at 1 CPU/512 MiB. That limit is relevant context, but the artifact does not report a kernel OOM reason. The cause of signal 9 remains unknown.
- `containers.log` SHA-256: `998ac36fed00cc6b3806072b1453d62ad4473eed9e83af25d225c81c81169d9e`.
- pgloader `loader.log` SHA-256: `ab70628a2718e5eff574e34d7b6cbf08aed089ecf6b3e196e004723640d3ca75`.
- Raw pgloader log SHA-256: `6e086ec19f2c1ceaaddcbde911add1d36937d0e4e2d7efd31e9d78c5aa860e6c`.

I corrected the previous worklog statement and `docs/testing-and-performance.md`. The first attempt remains incomplete and does not identify the kill cause or implicate either loader. A later retry passed the same 10-million-row narrow-integer correctness oracle with increased PostgreSQL headroom; see the [retry evidence](2026-10-05-T21-large-profile-retry.md). No source/test code or default fixture limit changed. The retry used intentionally unequal database resource caps and is not matched benchmark evidence.

## Validation and handoff

- Re-read the three retained logs and matched the failing SQL against the current `target_oracle` path.
- No database workload was run during this audit; it corrects the stage attribution only.
- Next T21 step: measure the separate multi-GiB `memory` profile only after its own capacity plan. Keep timing ineligible until phase and resource controls are comparable.
