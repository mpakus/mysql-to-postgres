# 2026-10-02 — T20 range config contract

Status: complete. This worklog records the coordinator-owned config contract, native registration, and final PG16/17/18 lane acceptance; T21 performance evaluation remains open separately.

## Reference research before coding

- Read `docs/reference-coding.md`, `docs/implementation-plan.md:67`, `docs/architecture.md` range/consistency sections, `docs/config-and-cli.md`, and T20 in `docs/agent-tasks.md`. The plan explicitly says to avoid the legacy `rows_per_range` semantics until sizing is benchmarked.
- RTK wrappers were used. Ponytail's full-mode guidance was previously read for this task and is applied: extend the existing config interface and validation seam, add focused boundary tests, and avoid a new abstraction/dependency. No separate RTK `SKILL.md` was present in the declared skill roots; the repository records that limitation.
- XERJ query: `project-my2pg "migration options validation unknown fields readers_per_table range" -k 10 --full 700`, indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It located `src/config/mod.rs` migration options and `src/config/validation.rs` validation tests. Direct inspection confirms `rows_per_range` is currently required positive and defaults to 100,000, while `readers_per_table` defaults to one and `SingleSnapshot` disallows multiple readers.
- A pinned-reference query is not applicable to this local serde/config compatibility choice; the relevant contract, source and tests are all in this project. No external code is being adapted.
- Chosen contract: add optional `migration.max_key_span`, default `None`, so range reads remain opt-in. Reject `Some(0)`. More than one per-table reader requires a positive configured span and `source.consistency = "frozen"`; keep `SingleSnapshot`'s existing single-reader restriction. A configured span with one reader is accepted and inactive. Do not reinterpret or remove `rows_per_range` in this compatibility slice.

## Test plan

- Config tests prove default omission, positive parsing, zero rejection, multiple-reader requirements, and frozen-source enforcement.
- F's scheduler/native RANGE tests independently prove key-space coverage and fallback behavior; config acceptance alone does not count as T20 acceptance.

## Implementation evidence

- Added `MigrationOptions.max_key_span: Option<u64>` with default `None`; existing `rows_per_range` stays unchanged.
- Validation rejects zero spans, requires a configured span for multiple readers, and rejects multiple readers unless the source is frozen. Single-reader configurations may carry an unused span.
- Documented units and compatibility behavior in `docs/config-and-cli.md`.
- `bin/cargo test --offline --locked config::validation::tests` passes (12 tests); `bin/cargo fmt --all --check` and `git diff --check` pass. An initial attempt to pass two Cargo test-name filters at once was invalid CLI syntax and was rerun with the full config validation module filter.
- At the time this config slice was implemented, native range acceptance remained with F; the coordinator acceptance section below now records the completed end-to-end gates.

## Coordinator native registration research before edits

- XERJ query: `project-my2pg "register native test mandatory required IDs integration harness range readers" -k 8 --full 650`, indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It located existing registration conventions in `tests/integration.rs` and mandatory native ID discovery in `tests/support/harness.py`; the current harness requires the T17 CHECK case on all platforms and a distinct macOS-only list only for the physical-sync fault case.
- Pinned `ref-paganel` query: `"movement lanes exact row counts no duplicates migration"`, revision `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`. Read `references/paganel/crates/engine-tests/src/movement/lanes.rs:1-43` and `configs/verify/payment_lanes.ppl:1-18`: their lane acceptance checks row parity and distinct-key count. We adapt an independent exact source/target multiset oracle for signed and unsigned boundary rows; Paganel's lane test does not cover numeric endpoints or fallback cases.
- Directly inspected `tests/integration.rs` module declarations and `tests/support/harness.py:21-46`. Chosen registration: add the T20 ignored native case to the integration target and make its exact ID mandatory on every host; no platform-specific split is needed for its MySQL/PostgreSQL fixture behavior.

## Native integration evidence

- Focused case `t20_ranges::integer_ranges_preserve_exact_signed_unsigned_and_fallback_multisets` passed on MySQL 8.0.46 → PostgreSQL 16.15 (1 passed). It preserved signed `i64::MIN/MAX`, unsigned `u64::MAX`, exact duplicate multisets, and reported fallback for empty, one-span, and no-PK tables.
- The first focused run exposed an oracle defect: PostgreSQL sorted text-mapped values lexically while the source sorted numeric values. XERJ evidence and the implementation/test read are recorded in F's T20 worklog; the correction sorts both exact text vectors in Rust, preserving duplicates without assuming compatible DB ordering. The corrected native rerun passed.
- Full serial lane passed on Darwin/ARM64, MySQL 8.0.46 → PostgreSQL 16.15: 149 invoked cases, 0 failures, all 18 mandatory IDs exactly once. Artifact `target/integration/my2pg-mysql80-pg16-615a324ef913/`; log SHA-256 `8978b135a40b38db801d60ccbb52976b99494e4e6581dd48276dc002e6bf7f91`; test binary SHA-256 `b45ee5d8a989ec1e6920b5bcffaedf02e34cdda486b96f6fe2d2402f93ada68b`; Cargo.lock SHA-256 `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`. Fixture state is stopped and harness JSON reports no unmet required IDs.
- The same lane's focused T17 CHECK override test passed on the same version pair (1 passed). Full PG17/PG18 lanes and default Rust unit regression checks remain outstanding.
- Follow-up expanded `tests/cases/t20_ranges.rs` with text/composite primary-key fallback initially failed during planning at the text-PK collation gate (`COLLATION_SEMANTICS`) before migration. T20 now explicitly omits only the fixture text primary-key column's collation and asserts that the acknowledged warning is reported. XERJ query and chosen adaptation are recorded in F's T20 worklog.
- Fresh serial MySQL 8.0.46 → PostgreSQL 16.15 lane passed after that correction on Darwin/ARM64: 149 invoked cases, 0 failures, all 18 mandatory IDs exactly once, including expanded T20 and T17 CHECK. Artifact `target/integration/my2pg-mysql80-pg16-9ee8967c4eae/`; test binary SHA-256 `0b527485d354a391ddee85501d5a6ff02111044e9cd5bd211c14e5623959cc74`; log SHA-256 `6bfc8a4c4c38e9a7f1988a14ffff8486901c6be7900a8d006e1319bd62a0c6de`; Cargo.lock SHA-256 `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`. `rust-tests.json` records exit 0, no failed cases, no unmet required IDs; the owned fixture was stopped by the harness.
- Matching refreshed lanes passed for PostgreSQL 17.11 and 18.6. Each invoked 149 cases, passed all 18 required IDs exactly once, reported no failures/unmet IDs, and stopped its owned fixture. PG17 artifact `target/integration/my2pg-mysql80-pg17-5d3f9463531f/`, log SHA-256 `75ae814d68dcab6a3c294aed2e8c50238d518770979fa3986eeb0d5010da68e7`; PG18 artifact `target/integration/my2pg-mysql80-pg18-b85710bfc1fb/`, log SHA-256 `f0326207663daef46316d0b356a61eccf02e6162dc561f93ed3ee5ee6382f36c`. Both use the same Cargo.lock and test binary SHA-256 listed above.
- The earlier PG16 result at `...615a324ef913` is superseded by the expanded-fixture run above. Full offline Rust tests pass (118 library tests and 13 ordinary integration tests; native database tests are separately run in the three lanes), strict all-target/all-feature Clippy passes, and formatting, the seven harness tests, and `git diff --check` pass. The first sandboxed Rust-suite attempt hit denied local socket binds in existing auth tests; rerunning with local-socket permission passed.
- T20 acceptance is complete for the configured integer range behavior and fallback contracts. T21 profiling/performance evidence remains a separate open task.
