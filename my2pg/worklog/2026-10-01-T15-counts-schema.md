# 2026-10-01 — T15 — actual counts and schema verifier

- Status: in_progress
- Role: G lease (config_console); coordinator owns report/config/model wiring.
- Claimed files: src/verify/, tests/cases/t15_verify*, this worklog.
- Base revision: 6e9d7a4 plus shared in-progress implementation.
- Dependencies: T01/T03/T05/T06/T07 production connections and plan; full schema completeness requires T11 typed expectations.
- Guidance: RTK and Ponytail remain active; existing drivers and PostgreSQL catalogs, no SQL parsing or generic driver trait.

## Reference research (before coding)

| Query / prefix | Pin and original file/tests read | Finding / adaptation |
| --- | --- | --- |
| verification schema counts / project-my2pg | 6e9d7a4 docs/agent-tasks.md:121, architecture.md:137 and tests/performance VERIFY; full contracts read | Same open snapshot, source original identity, append baseline + committed; counts are not content equivalence; independently test count and structure mismatches |
| verify schema count / ref-paganel | ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc engine-verify/src/verifier.rs:69–175; engine-tests/src/schema/verify_objects.rs:1–150 and verify/mod.rs:1–85 | Receipts and Merkle state, keyed traversal, cascade/enum/generated/rename checks. No state database or copied code; our M1 verifier reports counts/schema only, T18 complete content remains separate |
| COUNT verification / ref-dmt-rs | 4e8015f7e841dbdf9df01e953aeb3948dfb3199a docs/upsert-update-detection-bug.md:1 and drivers/mysql/reader.rs:830–850 | Documented count-only early success misses updates; query_first missing result becomes zero in source helper. Reject missing count results; equal-count value mutation demonstrates scope rather than claiming content validation |

Requested coordinator freeze typed expected indexes/FKs/checks/sequence/default semantics before claiming complete schema coverage. Do not infer an expected schema by parsing generated SQL, or hide missing coverage behind success.

## Intended behavior and checks

Read source counts in the supplied live SourceConnection, target qualified counts and catalog state; check run accounting and target baseline, ordered columns/types/null/generated/identity, keys and constraint/index validity. Report explicit Different/Unsupported/Error; schema_only checks structure and zero copied rows. None is NotRun. Tests must change actual target count/columns/constraints and prove source snapshot semantics; T18 content tests remain open.

## Verification

Implemented counts_and_schema with existing SourceConnection/TargetConnection, original source and qualified target identity, checked append baselines, row accounting, exact ordered column/type/null/generated/identity flags, default presence, table/column comments, PK order, ordinary index columns/direction/uniqueness/primary/validity, FK endpoints/actions/enforcement/deferral, and invalid-index/unvalidated-constraint detection. Missing complete typed structure expectations, unknown default/generated/CHECK equivalence, unknown full sequence bounds and requested content scope return Unsupported. Identity existence and known next lower-bound/range failures are detected but not advertised as complete sequence coverage.

Actual owned fixture project: my2pg-mysql84-pg16-be6d70a9aef6; Oracle MySQL 8.4.11 / PostgreSQL 16.15, linux/arm64. Image pins: MySQL sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242, PostgreSQL sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea. Exact fixture/environment manifest retained under target/integration/my2pg-mysql84-pg16-be6d70a9aef6/connections.json.

Commands through RTK:

- my2pg/bin/cargo test --locked --test integration t15_verify -- --ignored --nocapture, with the owned env.sh: 2 cases passed, no failures. Includes equal-count byte mutation (honestly still counts/schema), actual missing/invalid structure and count changes, unique descending index/FK deferral, missing append baseline, schema-only/None/content scopes, unknown sequence coverage and invalid next value.
- Snapshot case creates a synthetic admin-only fixture, starts the production reader's read-only consistent snapshot, inserts another row through the separate admin, and proves verification still counts one; after ending the snapshot it detects two. This is fixture setup, not product source writes. Both synthetic source/target objects removed.
- my2pg/bin/cargo test --locked --lib verify:: — pure type alias/modifier/timezone case passed.
- my2pg/bin/cargo clippy --locked --all-targets -- -D warnings: passed after two owned style warnings corrected.
- Owned files formatted with pinned Rust 1.99.0 rustfmt.

Tested file SHA-256: src/verify/mod.rs f01770dbebd8cf6695f5ac7080196c65ef8706e25302272ae87e77b2a1fb75f8; tests/cases/t15_verify.rs 4b66ad0c9682e12f90f6dc8210559d88b8c94d7d400e75b6d5b6d439d8896006. Shared model remains coordinator-owned; no workers changed it here.

## Handoff and remaining coverage

API: counts_and_schema(source, target, plan, reports, baseline) async Result<VerificationReport,VerifyError>; errors retain only static category/SQLSTATE, never database message values. Root wired module and integration discovery. Typed SchemaExpectations model added by root, ready for D's population.

M1 no-identity/no-unresolved-expression fixtures can obtain complete counts/schema evidence. Full T15 remains open until T11 supplies complete default/generated/CHECK/sequence and data-only constraint/trigger preservation expectations, live pipeline persistence/verification integrates, and CLI fault cases pass. T18 content is explicitly unsupported here; counts cannot certify changed values. Only this worker's fixture pair was stopped successfully; existing services and other workers' fixtures untouched.
