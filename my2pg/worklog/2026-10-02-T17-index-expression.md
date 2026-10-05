# 2026-10-02 — T17 — explicit functional-index expression override

- Status: in progress
- Agent/role: coordinator
- Task and specification links: [T17](../docs/agent-tasks.md#task-board), [T17 scope](../docs/scope-and-compatibility.md#feature-disposition), [configuration override](../docs/config-and-cli.md)
- Workspace/worktree: `/Users/renatibragimov/www/pg/my2pg`
- Base revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`
- Dependencies and their tested revisions: T10/T11 are integrated but retain wider M2 gates
- Claimed files: `src/model.rs`, `src/plan/mod.rs`, `src/plan/existing.rs`, `src/verify/indexes.sql`, `src/verify/mod.rs`, planner tests, one T17 native case, integration registration, harness registry, this worklog

## Intended result

Allow an operator to replace one MySQL functional B-tree index key expression with an explicit PostgreSQL `target_expression`. Planning and execution verification must agree on the exact target expression; absent policy must block before DDL; explicit omission remains available. This slice will not translate MySQL expressions or infer dialect equivalence.

## Reference research before coding

| Problem/query and prefix | Repository commit and file/line | Source/test behavior observed | Adaptation or rejection; our acceptance test |
| --- | --- | --- | --- |
| `index expression target_expression INDEX_UNSUPPORTED omission`; `project-my2pg` | Working-tree revision label `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `src/plan/mod.rs:1340-1354,1380-1438`, `src/verify/indexes.sql:1-17`, `src/verify/mod.rs:448-476`, `src/plan/existing.rs:516-535` | Catalog retains source index-part expressions, but planner blocks every expression part; expectations and verification currently model only ordinary columns. The existing expression-fragment validator and exact source-object override are reusable. | Keep target SQL operator-supplied and exact. Add an aligned optional expression expectation to index parts; compare `pg_get_indexdef` output during fresh-run verification and existing-target checks. One expression-bearing key per index is supported in this increment; unsupported shapes continue to require explicit omission. Native test must fail before target schema creation without policy, then verify a unique `lower(email)` expression index and observe PostgreSQL reject a case-folded duplicate. |
| `functional index expression lower email create index MySQL fixture`; `project-pgloader` | pgloader `231ab86778ca5ffd7de40878714760c8b4860cdf`; `clojure/src/pgloader/ddl/common.clj:294-340`, `clojure/test/pgloader/ddl_test.clj:98-116,141-145`, `clojure/tests/mysql/mytest/mytest.sql:240-245`, `clojure/tests/mysql/mytest/sql/04-expression-index.sql:1-8` | pgloader represents index keys as either plain columns or unquoted expressions, generates expression-index DDL, and tests both expression and mixed keys. Its MySQL fixture gates the functional index to MySQL 8.0.13+; the SQL assertion checks PostgreSQL catalog output. | Reuse the behavioral shape and a MySQL 8 functional-index case, but reject automatic translation of raw source expressions. Require the my2pg `target_expression`; verify PostgreSQL's catalog result and actual uniqueness enforcement. |
| `expression index functional index conversion postgres test`; `ref-paganel` | Paganel `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`; `crates/query-builder/src/ast/create_index.rs:1-31`, `crates/query-builder/src/renderer/create_index.rs:150-200` | Its query AST separates index key expressions from names and the PostgreSQL renderer emits a simple quoted-column index. No exact MySQL expression-compatibility rule was found. | Do not add an AST or translator for one supported override form. Preserve ordinary identifier quoting and pass only the already-validated explicit expression fragment. |

Direct implementation and adjacent-test review is complete before coding: `IndexPart` carries `column` or `expression`; `ObjectOverride.target_expression` is an already-validated single SQL fragment; fresh index verification currently rejects all expression indexes through `indexes.sql`; existing-target shape matching also assumes all keys are columns. The added model field must default when reading older serialized structures. RTK was used for every repository command; Ponytail full guidance was read and applied by reusing existing models/validation with no dependency or general SQL parser.

## Test plan

- Pure planner tests: no expression override blocks, an exact override is rendered and enters structural expectations, and `omit=true` excludes it. Multi-expression or unsupported index shapes remain blocked.
- Native MySQL 8/PostgreSQL case: create a unique MySQL functional index; require missing-policy failure before destination schema creation; rerun with `target_expression = "lower(email)"`; require normal verification to complete and the PostgreSQL catalog expression to match; prove uniqueness with a case-folded duplicate insert.
- Run focused planner tests, offline suite, fmt, strict Clippy, integration `--no-run`, exact ignored-case discovery, and the owned serial database lane. Root owns shared index refresh and final task/checklist updates.

## Changes

- `IndexExpectation` now retains aligned optional expressions, with serde defaults for older serialized structures. The planner accepts a single functional B-tree key only with an exact operator-supplied PostgreSQL expression; unsupported shapes still block and explicit omission remains intact.
- DDL emits the explicit key expression. New structural checks and existing-target checks compare `pg_get_indexdef` output against that expression; ordinary index columns retain their existing exact checks.
- Added a planner test and a mandatory native MySQL 8.0/8.4 case. The case proves fail-before-DDL without policy, verifies the created unique index, and checks that a case-folded duplicate insert fails with SQLSTATE `23505`.
- Documented the exact object ID and PostgreSQL deparse requirement, including the observed `lower((email)::text)` example.

## Verification

| Check | Exact command or artifact | Environment/revision | Result |
| --- | --- | --- | --- |
| Reference searches and source inspection | XERJ queries and direct `rtk run` reads listed above | XERJ project label `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; pgloader and Paganel pins as listed | Completed; evidence recorded before implementation. |
| Focused planner check | `bin/cargo test --offline --locked --lib functional_index_needs_explicit_target_expression_or_omission` | Darwin/ARM64; source working tree | 1 passed. |
| Native focused diagnosis | `tests/support/harness.py --start mysql80 pg16`; one ignored T17 case against owned fixture | MySQL 8.0.46 → PostgreSQL 16.15, ARM64 | First run exposed PostgreSQL's deparse `lower((email)::text)`; after matching that exact expression, focused case passed (1 passed). Fixture stopped and metadata records `stopped`. |
| Format, compile and lint | `bin/cargo fmt --all -- --check`; `bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings`; integration `--no-run` | Darwin/ARM64 | Passed after final code/doc adjustment. |
| Support harness tests | `python3 tests/support/test_harness.py` | Darwin/ARM64 | 7 passed, including MySQL 5.7 exclusion and MySQL 8.0/8.4 inclusion for this native case. |
| Full native lane | `python3 tests/support/harness.py mysql80 pg16` | MySQL 8.0.46 → PostgreSQL 16.15, ARM64; owned fixture `my2pg-mysql80-pg16-027e8b7e293b` | Passed 150 cases; 21 mandatory IDs exactly once, zero failed or unmet IDs, 17 filtered. The T17 expression case passed. Harness artifact: `target/integration/my2pg-mysql80-pg16-027e8b7e293b/rust-tests.json`; test log SHA-256 `a548ca80487021ef3327e08a58176f80c3f30ceea1960be4e0d4d32157c6bc3b`; fixture metadata says stopped. |
| Offline all-target Rust tests | `bin/cargo test --offline --locked --all-targets --all-features -- --test-threads=1` | Darwin/ARM64 | Passed, exit 0. The first sandboxed parallel run had local listener permission failures and an artifact-worker collision; the escalated serial rerun passed. |
| Diff hygiene | `git diff --check` | Current working tree | Passed after implementation. |

## Decisions and deviations

The single-expression override boundary keeps the existing `target_expression` contract precise. PostgreSQL can deparse an expression with explicit casts that were implicit in the submitted SQL; exact structural verification requires the operator expression to use the observed deparse form. The first native run caught this with `lower(email)` versus `lower((email)::text)`. After changing the override to the observed form, the focused native case passed. Multi-expression indexes can still be explicitly omitted, and no MySQL SQL is rewritten into PostgreSQL SQL.

## Limitations or blocker

This does not complete T17's spatial/PostGIS gate or all expression-index shapes. The accepted native lane covers one functional B-tree key with an explicit exact PostgreSQL expression; multi-expression and unsupported index shapes still need explicit omission or remain blocked. This does not establish PostGIS behavior, other server versions, or platform coverage.

## Handoff

- Completed acceptance cases: XERJ/reference research, planner policy, explicit DDL expression, focused native behavior, and full owned-lane acceptance.
- Artifacts and digests: full native acceptance and test-log digest are recorded above; the owned fixture is stopped.
- Files/revision ready to integrate: implementation and documentation remain in the shared dirty working tree at base label `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; no commit was created.
- Remaining work: finish the other T17 CHECK/index-shape and spatial/PostGIS gates, expand T22's required version/platform matrix, and continue T21's fixed-corpus benchmark only after dependencies and baseline corpus are accepted.
- Next task/owner: coordinator continues T17.

## Review

- Reviewer: coordinator self-review
- Findings and resolutions: PostgreSQL catalog deparses `lower(email)` as `lower((email)::text)` on the accepted fixture. The override, exact verifier, and docs now use that observed target form; the focused test and full required native lane pass.
- Task-board update/reference: T17 remains `in_progress`; this accepted slice is now recorded in `docs/agent-tasks.md` and `docs/checklists.md`.
