# Source audit

## Checkout and evidence limits

The reference is `../../pgloader/`, commit `231ab86778ca5ffd7de40878714760c8b4860cdf`, titled `fix(v4): MySQL/MSSQL migration bugs found while capturing the Migration course (#1776)`. Its working tree was clean during the inspection.

The workspace parent was not a Git repository, and `my2pg/` did not exist. This planning pass created the sibling directory without changing pgloader or initializing another repository.

This is a **source inspection**, not a passing runtime certification. Clojure CLI, SBCL, Cargo, and rustc were not found on the active shell PATH. Java and Docker command paths were present; their runtime readiness was not established. No databases were started, no migration was run, and no benchmark was collected.

## Evidence map

Paths below are the starting points for implementation agents. Symbols identify the relevant logic within larger files.

| ID | Evidence | What it establishes |
| --- | --- | --- |
| S01 | [Top-level README](../../pgloader/README.md), [Clojure README](../../pgloader/clojure/README.md) | Separate Common Lisp v3 and Clojure/JVM v4 implementations, supported sources, and compatibility claims |
| S02 | [v4 specification](../../pgloader/clojure/pgloader-v4-spec.md), sections 1, 6–11 | Intended streaming pipeline, load DSL, batch transactions, rejects, and phases |
| S03 | [MySQL reference](../../pgloader/docs/ref/mysql.rst), [transform reference](../../pgloader/docs/ref/transforms.rst) | User-facing clauses, defaults, casts, filters, schema changes, and documented limitations |
| S04 | [v3 system](../../pgloader/pgloader.asd), [database migration](../../pgloader/src/load/migrate-database.lisp) | Common Lisp dependencies and schema/load/finalization flow |
| S05 | [v3 MySQL reader](../../pgloader/src/sources/mysql/mysql.lisp), [cast rules](../../pgloader/src/sources/mysql/mysql-cast-rules.lisp), [catalog SQL](../../pgloader/src/sources/mysql/sql/list-all-columns.sql) | Original MySQL behavior and ordered default casts |
| S06 | [v3 COPY recovery](../../pgloader/src/pg-copy/copy-retry-batch.lisp) | COPY context parsing and sub-batch recovery when row positions are unavailable |
| S07 | [v4 grammar](../../pgloader/clojure/src/pgloader/load_file/grammar.clj), [AST](../../pgloader/clojure/src/pgloader/load_file/ast.clj), [parser](../../pgloader/clojure/src/pgloader/load_file/parser.clj) | Accepted syntax and normalization, distinct from execution support |
| S08 | [v4 orchestrator](../../pgloader/clojure/src/pgloader/core.clj) | `run-command`, defaults, table workers, schema preparation, index scheduling, finalization |
| S09 | [v4 MySQL source](../../pgloader/clojure/src/pgloader/source/mysql.clj), [SQL](../../pgloader/clojure/src/pgloader/source/mysql.sql) | JDBC streaming, catalog queries, charset overrides, version detection, integer range readers |
| S10 | [v4 DDL](../../pgloader/clojure/src/pgloader/ddl/common.clj), [casts](../../pgloader/clojure/src/pgloader/cast.clj), [transforms](../../pgloader/clojure/src/pgloader/transforms.clj) | Actual schema/type mappings, defaults, enum handling, names, expressions, triggers, sequences |
| S11 | [Prefetch](../../pgloader/clojure/src/pgloader/prefetch.clj), [batch](../../pgloader/clojure/src/pgloader/batch.clj), [COPY encoder](../../pgloader/clojure/src/pgloader/copy.clj) | Queue, per-batch commits, retries, serialization, and memory knobs |
| S12 | [CLI](../../pgloader/clojure/src/pgloader/cli.clj), [logging](../../pgloader/clojure/src/pgloader/log.clj), [summary](../../pgloader/clojure/src/pgloader/summary.clj), [rejects](../../pgloader/clojure/src/pgloader/reject.clj) | Exit behavior, redaction, text/JSON/CSV summaries, reject file layout |
| S13 | [Clojure dependencies](../../pgloader/clojure/deps.edn), [Makefile](../../pgloader/clojure/Makefile) | Build dependencies and explicitly selected local unit suites |
| S14 | [CI workflow](../../pgloader/.github/workflows/clojure-integration-tests.yml), [suite Makefile](../../pgloader/clojure/tests/Makefile), [test README](../../pgloader/clojure/tests/README.md) | Actual test selection, Docker orchestration, expected-output generation, benchmarks |
| S15 | [MySQL suite](../../pgloader/clojure/tests/mysql/Makefile), [Compose](../../pgloader/clojure/tests/mysql/docker-compose.yml), [Dockerfile](../../pgloader/clojure/tests/mysql/Dockerfile) | MySQL 8.0 + PostgreSQL 16/PostGIS baseline and legacy authentication settings |
| S16 | [Unified MySQL load](../../pgloader/clojure/tests/mysql/mytest/mytest.load), [seed](../../pgloader/clojure/tests/mysql/mytest/mytest.sql), [assertion SQL](../../pgloader/clojure/tests/mysql/mytest/sql), [expected output](../../pgloader/clojure/tests/mysql/mytest/expected) | Concrete regression corpus and deliberate v3/v4 differences |
| S17 | [Parser tests](../../pgloader/clojure/test/pgloader/load_file/parser_test.clj), [batch tests](../../pgloader/clojure/test/pgloader/batch_test.clj), [MySQL tests](../../pgloader/clojure/test/pgloader/source/mysql_test.clj) | Useful unit cases and coverage gaps |
| S18 | [v3 fixtures](../../pgloader/test/mysql), [older full MySQL fixture](../../pgloader/clojure/tests/mysql-unit-full), [benchmark Makefile](../../pgloader/clojure/tests/bench/Makefile) | Additional edge cases and baseline timing machinery |
| S19 | [Agent guide](../../pgloader/AGENT.md), [issue ledger](../../pgloader/clojure/GITHUB-ISSUES.md), [TODO](../../pgloader/TODO.md) | Historical intent and claims that need current source/test verification |
| S20 | [Project license](../../pgloader/LICENSE), [fixture licenses](../../pgloader/test/README.md), [F1DB downloader](../../pgloader/test/mysql/download-f1db.sh) | Notices and dataset provenance to retain when reusing material |

## Actual migration flow

Both implementations follow the same useful outline:

1. Parse a load command, connection details, selection rules, casts, and options.
2. Introspect MySQL tables, columns, primary keys, indexes, and foreign keys.
3. Resolve destination names and types, prepare schemas/types/tables, and apply configured SQL.
4. Stream selected rows, transform values, and encode PostgreSQL COPY text.
5. Queue batches behind a size/count limit and commit each COPY batch separately.
6. Roll back failed batches, isolate rejected rows, and commit successful sub-batches.
7. Build keys/indexes, add foreign keys/checks, reset sequences, execute post-load SQL, and summarize.

The v4 orchestrator is 1,157 lines and contains source dispatch, connection setup, catalog changes, DDL, scheduling, and reporting. Its MySQL source uses streaming JDBC result sets and creates separate connections for range partitions. The Rust rewrite should preserve the useful flow while putting schema planning, transport, recovery, and presentation in separate modules.

## Findings that change the Rust plan

| Finding | Evidence | Consequence |
| --- | --- | --- |
| v4 is called a drop-in replacement, but INI/context behavior and other details differ | S01, S07 | Compatibility must be an explicit tested list, not a blanket claim |
| `no truncate` appears in the MySQL documentation but not in the inspected v4 database grammar | S03, S07 | Add an importer fixture for this exact difference and map it explicitly |
| `ALTER TABLE ... SET (...)` and `SET TABLESPACE` parse, while `apply-alter-table` documents that these actions are ignored | S07, S10 | Unsupported configuration must produce a diagnostic before target writes |
| Default table creation can issue `DROP TABLE ... CASCADE` unless `include no drop` is supplied | S08 `run-command`, S10 `drop-table-if-exists-sql` | Fail on existing objects by default; scoped replacement is explicit and never automatically cascades |
| `--dry-run` checks connections only | S08 | Rust `plan` must inspect metadata and show actual decisions/DDL without writing either database |
| Post-load DDL helper logs warnings and skips failures; CLI distinguishes fatal errors from row rejects | S08 `exec-post-ddl!`, S12 | Required index/FK/check/sequence failures and rejected rows must affect the final status |
| `on-error-stop` is bound in the orchestrator, but the batch retry path does not consult it | S08, S11 | Test stop behavior at the first failed COPY batch, including cancellation of concurrent workers |
| v4 has 25,000-row randomized batches, a 20 MiB threshold, and a four-batch queue per pipeline | S11 | Use deterministic limits and a global byte budget; include active reader/writer buffers and parallelism in memory accounting |
| Range splitting uses a single integer PK and converts endpoints to JVM `long` | S09 `mysql-partition-source` | Test unsigned 64-bit extrema, overflow, sparse ranges, last endpoints, and fallback paths |
| Introspection catches some metadata errors and substitutes empty lists | S09 `catalog` | Permission/query failure must be distinguished from an empty database or unsupported metadata version |
| Types/defaults are interpreted by casts and DDL; v4 maps unsigned auto-increment through `bigserial`, `TIME` to `time`, and `DATETIME` to `timestamptz` | S10 | Define range, duration, timezone, and sequence policies explicitly; use semantic expectations rather than copying every mapping |
| Generated/index expressions receive limited textual rewriting | S09, S10 | Use a small tested expression subset and explicit PostgreSQL overrides, not blind dialect substitution |
| MySQL enum/set handling, charset correction, binary values, and zero dates already have regression fixtures | S16–S18 | Preserve these as acceptance cases early |
| Existing-view materialization and creation of temporary source views are different operations | S03, S09 `create-view!`, S08 `materialize-views!` | Support reads of existing views later; exclude source DDL from the product |

These findings identify behavior and risks from code. They are not claims that every potential failure was reproduced against a running database.

## Test coverage inventory

There are **17** Clojure `*_test.clj` files with **186** `deftest` declarations. This is an inventory, not an assertion count or a passing test result.

- The workflow's unit command selects **8 namespaces**. The local `test-unit` Makefile selects **11**.
- `copy-test`, `summary-test`, and `load-file.ast-test` are among the namespaces omitted by both explicit lists.
- `source.mysql-test` and `log-test` are in the local list but absent from the CI unit command.
- The unified MySQL regression directory has **19 SQL assertion files** and **34 expected-output files**, including variants. Expected files are not 34 independent scenarios.
- `mysql57` is named by the suite Makefile and READMEs, but `clojure/tests/mysql57/` is absent in this checkout. It is also absent from the workflow's integration matrix.
- The `batch_test.clj` coverage checks accumulation and COPY SQL. Its real server error-context case is a placeholder; it does not establish database-backed retry correctness.
- The MySQL Compose suite uses `mysql:8.0` and `postgis/postgis:16-3.4`, with `mysql_native_password` enabled for the Common Lisp baseline. This does not test default modern MySQL authentication or plain PostgreSQL without PostGIS.
- Existing SQL assertions sometimes check presence rather than behavior: for example, the ON UPDATE case lists triggers. Rust tests must execute updates and check the resulting timestamps.
- Baseline regeneration can select v3 variants. Keep version-specific expectations and review intentional Rust differences individually.

The v4 specification also refers to a prior v1.1 specification for details; the inspected document is not a self-contained executable contract. The agent guide and issue ledger contain older paths and implementation claims. Resolve disagreements using current source, then reproducible tests.

## What to carry forward

Carry forward the MySQL regression scenarios, SQL catalog knowledge, ordered casts, COPY batching/recovery model, filters, explicit schema mapping, reject artifacts, and readable summaries. Reimplement them in Rust under the contracts in this plan.

Use synthetic MySQL fixtures as the always-available test foundation. Retain Sakila/F1DB as separately provisioned, provenance-tracked larger datasets. Do not import unrelated fixtures, driver stacks, historical setup recipes, or all old expected output merely to reproduce a directory tree.
