# 2026-10-02 — T11 — Existing-target comment drift

Status: native acceptance passed; overall T11 remains open. Scope is only a new existing-target comment-drift case and this worklog. No shared planner or runner files are in scope.

## Reference research

- Project query: `project-my2pg "existing target column comment mismatch append TARGET_COMMENT_INCOMPATIBLE"` (XERJ revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`). The search located `src/plan/existing.rs` around lines 241–301 and existing-target policy tests. Direct source review confirmed column comments are compared at lines 276–282 and table comments at lines 333–338. Both emit `TARGET_COMMENT_INCOMPATIBLE`; comparison runs before index/constraint binding and before execution. The native existing-policy test covers matching table comments in its composite structure case, but no native test was found for comment drift.
- pgloader query: `project-pgloader "column comment target comment compare PostgreSQL existing table"` (pinned revision `231ab86778ca5ffd7de40878714760c8b4860cdf`). `src/sources/mysql/mysql-schema.lisp:58–84` reads table and column comments from the MySQL catalog, and `src/sources/mysql/sql/list-all-columns.sql:8–17` selects both comments with source columns. `src/pgsql/pgsql-create-schema.lisp:601–632` emits target COMMENT statements for table and column comments. This confirms comments are structural migration inputs; it does not define an existing-target mismatch policy.
- Paganel query: `ref-paganel "comment schema introspection existing table planner"` (pinned revision `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`). `crates/engine-planner/src/builder/analyzers/schema.rs:48–102` compares existing columns and plans missing-column additions, but does not compare comments. This is not a suitable behavior to copy because this project's T11 contract requires comment fidelity and its data-only policy validates an existing target before COPY.
- Contract reviewed: `docs/scope-and-compatibility.md`, “Structure and naming contract” and data-only policy; `docs/agent-tasks.md`, T11; `docs/checklists.md`, outstanding T11 structure checklist. Existing data-only targets must preserve their own contents and structures; a source comment mismatch is a blocking incompatibility, not a mutation request.
- RTK guidance was applied for searches and source reads (`rtk run`, `rtk --help`). No Ponytail executable or Ponytail skill file is installed/discoverable in the available local skill roots; I will keep the change minimal and isolated in line with the project's smallest-complete-solution guidance.

## Chosen adaptation and acceptance

Keep the current planner policy: a nonmatching requested table or column comment blocks Append/data-only planning. Add an independent native case in a new test module. It creates a MySQL table/column with comments and a PostgreSQL existing target with different comments plus a sentinel row. Running the CLI must fail with `TARGET_COMMENT_INCOMPATIBLE` before COPY; the target row and both original target comments must remain unchanged. No production fix is planned unless the native case demonstrates the guard is bypassed.

## Verification

- Coordinator registered the case at `tests/integration.rs` (`t11_existing_comments`); that shared file is outside this lease.
- File-specific `rustfmt --edition 2024 --check tests/cases/t11_existing_comments.rs` passed.
- `bin/cargo test --test integration t11_existing_comments --no-run` passed, compiling the ordinary integration target.
- After shared verifier work stabilized, the focused native command `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration t11_existing_comments -- --ignored --nocapture --test-threads=1` passed: 1 passed, 0 failed. It observed both mismatch diagnostics and confirmed target sentinel data and comments were preserved. The owned pair `my2pg-mysql84-pg16-4ab98a95dcc0` was stopped via its exact `connections.json`; recorded state is `stopped`.
- `bin/cargo test --offline --locked --lib plan::tests` passed 26/26.
- `bin/cargo fmt --all -- --check` passed after shared formatting completed.
- Strict command `bin/cargo clippy --offline --locked --all-targets --features artifact-worker-tests,native-import-tests -- -D warnings` is blocked by three Clippy findings in shared `src/verify/content.rs`, outside this lease: `too_many_arguments` on `verify_content` and `verify_content_inner`, plus `collapsible_if`. No T11-case lint was reported before the library lint failure; do not edit the shared T18 file from this lease.
- Broader T11 matrix and policy gates remain open.
