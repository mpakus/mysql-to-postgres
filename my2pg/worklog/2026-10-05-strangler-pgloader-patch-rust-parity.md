# 2026-10-05 — Strangler pgloader patch parity review

- Status: reviewed; no Rust implementation gap found for the reliability changes; FULLTEXT fix intentionally not ported
- Agent/role: coordinator
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Project revision: `8316473e60f5149785af78fb987d7a63fdb851c0` (clean before this worklog)
- External patch reviewed read-only: `/Users/renatibragimov/www/strangler/lib/m2p/pgloader.patch`
- Reference revision: pgloader `231ab86778ca5ffd7de40878714760c8b4860cdf` (pinned, read-only)

## Reference research before implementation

- Project XERJ query: `project-my2pg "required DDL failure final outcome cancellation"`; result set reported indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, which predates the current checkout. I therefore treated it as a locator only and verified each claim against the current working tree at `8316473e60f5149785af78fb987d7a63fdb851c0`.
- Pinned reference XERJ queries: `project-pgloader "on-error-stop failed indexes fatal errors worker cancellation"` and `project-pgloader "prefetch reader writer exception end-of-data bounded queue"`, revision `231ab86778ca5ffd7de40878714760c8b4860cdf`.
- The user patch was read in full, along with Strangler's `lib/m2p/check_loader.clj`, `lib/m2p/README.md`, and `bin/m2p-load`. The patch changes fatal DDL/index/sequence handling, error-cause retention, cancellation and worker shutdown, reader terminal signaling, suppression of after-load commands following errors, strict CLI handling, and MySQL multi-column FULLTEXT DDL.
- Pinned pgloader source located the corresponding upstream paths in `clojure/src/pgloader/core.clj` and `clojure/src/pgloader/prefetch.clj`. My2pg's relevant original/adjacent behavior was inspected in `src/pipeline/mod.rs`, `src/pipeline/recovery.rs`, `src/cli.rs`, `src/plan/tests.rs`, `tests/cases/t12_required_ddl.rs`, `tests/cases/t13_concurrency.rs`, `tests/cases/t19_views_hooks.rs`, and `tests/contracts.rs`.
- RTK command wrapper was used. No standalone RTK or Ponytail skill file/command was available in the configured local skill roots; no code was changed, so this review did not claim a Ponytail code-edit workflow.

## Disposition

| Strangler patch behavior | Current My2pg evidence | Decision |
| --- | --- | --- |
| Required post-load DDL, indexes, constraints, sequences, and SQL hooks must fail the migration instead of being logged and skipped | The runner executes planned DDL transactionally and propagates failures into failed status/steps. `tests/cases/t12_required_ddl.rs` exercises a real post-COPY PostgreSQL DDL failure and verifies the durable failed report. `tests/cases/t19_views_hooks.rs` covers after-hook failure and suppresses after hooks when target preparation fails. | Already implemented. Preserve the distinction between acknowledged COPY data and later failed DDL; do not attempt to roll back earlier committed batches. |
| Parallel COPY/index failure must cancel sibling work, retain the originating fatal fault, and report shutdown/commit uncertainty honestly | `ExecutionRegistry` publishes and retains the first fatal fault; worker failure sends the stop signal, closes resource admission, cancels source/target work, and drains scoped workers. `Fault::retain_stronger` lets indeterminate commit evidence outrank ordinary failure. Unit and native cases in `src/pipeline/mod.rs` and `tests/cases/t13_concurrency.rs` cover first-fault retention, fatal reject-limit propagation, blocked-source cancellation, and acknowledged-prefix/no-replay behavior. | Already implemented with Rust async cancellation and scoped futures. Do not copy Clojure dynamic bindings or thread-specific implementation. |
| A reader error cannot strand a writer when a bounded handoff queue is full | Rust uses bounded Tokio channels and explicit cancellation notifications rather than a blocking JVM queue plus terminal sentinel. `tests/cases/t13_concurrency.rs` includes blocked source/target shutdown cases; inspect future pipeline changes against these cancellation invariants. | No direct port needed; retain the Rust channel/cancellation design. |
| Invalid/extra CLI options must not be interpreted as positional input | `clap` defines the command grammar and rejects unknown, conflicting, and extra arguments; `src/cli.rs` and `tests/contracts.rs` cover accepted controls and rejection behavior. | Already implemented. |
| Multi-column MySQL FULLTEXT can be converted to a GIN expression | `docs/scope-and-compatibility.md` explicitly says FULLTEXT has no blanket PostgreSQL equivalent and requires explicit omission or reviewed target SQL. Planner tests require explicit decisions for unsupported index forms (`src/plan/tests.rs`). | Intentionally do not port. The automatic GIN expression in Strangler's patch is application-specific and would overstate MySQL/PostgreSQL search equivalence in My2pg. |

## Result and verification

No Rust code or regression tests were added because the reliability guarantees are already part of the Rust runner and have targeted tests. The user patch remains untouched. Fresh targeted checks passed: `bin/cargo test --lib worker_fault_tests` (3 passed, 2 native-fixture tests ignored), `bin/cargo test --test contracts` (15 passed), and `bin/cargo test --lib unsupported_functional_index_shapes_require_explicit_omission` (1 passed). Native MySQL/PostgreSQL integration tests were not rerun. The XERJ project index was older than the inspected checkout, and that mismatch is recorded above.

If a future bug report identifies a missing failure-propagation case, add the smallest native test at the runner boundary and follow `docs/reference-coding.md`; do not import the Strangler patch wholesale.
