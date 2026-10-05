# 2026-10-01 — T16 — importer-driven native compatibility

Status: native acceptance investigation/implementation lease, not full T16 acceptance. Own new tests/cases/t16_import_native.rs and this log. Production parser/config/transport/planner/executor/verifier remain peer/coordinator-owned. Actual manifest is tests/fixtures/upstream-manifest.json, not mysql-manifest.json; no manifest edits proposed without coordinator agreement. Unique fixture lifecycle required.

## Reference research before coding

RTK/Ponytail guidance and reference-coding workflow applied. Fresh XERJ project-my2pg query `import native compatibility defaults casts` at indexed/live db28c459a8a3caf45b07e9e72bcbc9fd3194c762 returned initial importer and CLI worklogs plus config-and-cli. Read originals: scope/T16/IMPORT requirements, documented subset, config/import.rs token/guard/options/filter/rename/session/default/publication code, cli/main dispatch, all18 adjacent importer tests (including classified13 load inputs and18 selected parser behaviors), current CLI/native policy tests and target/source models.

Pinned project-pgloader query `CAST type when extra auto_increment` at231ab86778ca5ffd7de40878714760c8b4860cdf returned command-cast-rules.lisp:1, cast_test.clj:121 and DDL common helpers. Read original Lisp parser1–125 and adjacent Clojure cast tests121–240: legacy implicit tinyint display-width boolean/NUL transformations differ from reviewed my2pg defaults; explicit type/column transforms and guards must produce actual values. Read original test/mysql/f1db.load: localhost/options/socket-default endpoints are intentionally rejected under strict TCP/credential refs, not silently normalized. No reference code/fixture copied; new synthetic native schema and independent expectations exercise contracts.

Chosen cases before implementation:

1. Actual `config import` with percent-encoded synthetic credentials, ordered finite casts/guards/transforms, include/exclude regex/exact names, source/table/schema rename, supported session settings and resource options. Assert private secret-free no-clobber normalized artifact. Only CA/report paths may be added to the runnable copy for the disposable TLS environment; all imported semantic choices must remain unchanged. Real offline `check` with unreachable credential refs proves no DB access. Real read-only `plan` must leave source/target inventories unchanged. Real `run` then independently assert values/types/default INSERT/PK/UNIQUE/FK/index/comments/identity-next, without using planner/verifier as the oracle. Deliberate defaults: tinyint(1) without explicit transform remains numeric, unsigned max stays exact, DATETIME wall time and TIME duration remain distinct.
2. Import-driven existing-policy decisions: default fail-on-existing preserves target OID/data; explicit data-only append preserves constraints/indexes/default/trigger and earlier rows; truncate preserves OID/structures and replaces content; scoped full recreate changes selected OID while leaving outside sentinel intact. Schema-only then data-only must be exercised through imported configs, with explicit reset policy; unknown generation/execution policies may expose genuine production gaps rather than be weakened.
3. Unsupported/ambiguous hooks/reader/display-width guards/legacy URIs/multiple commands and selected original13 load files through actual import CLI. Every failure must leave no runnable partial artifact or staging residue, preserve an existing private artifact/symlink and target sentinel OID/data, and disclose no credential substrings. No claim of native SQL/data coverage for all upstream datasets; classification gaps remain explicit.

Prepare tests independently while C/F artifact lifecycle integration is active. Native execution starts only at a compile-stable peer interface. Root registers cases; no root module/Cargo/index changes by this lease.

## Renewed worker research before edits

RTK/Ponytail guidance reread. Fresh project-my2pg query "importer native CAST auto_increment" returned importer.rs241 and this log at db28c459a8a3caf45b07e9e72bcbc9fd3194c762; live HEAD agrees, later peer edits are not represented by the snapshot. Read importer original TYPE/COLUMN guard/action matcher700–885, ImportOptions/report/API1–105, adjacent importer tests1–325, config CLI subset45–85 and current existing-policy native case1–180. Fresh project-pgloader "CAST type when extra auto_increment" at231ab86778ca5ffd7de40878714760c8b4860cdf returned Lisp parser1 and Clojure cast_test121; originals1–125/121–215 read. Their implicit tinyint boolean/NUL policy differs from reviewed my2pg defaults. Use new synthetic cases and independent SQL observations, not copied expected output.

Adaptation: actual import CLI emits the only semantic configuration; runnable copy adds just fixture CA paths and report output location. Environment refs name the actual owned harness endpoints, while legacy input contains synthetic percent-encoded credentials. Assert unsigned maximum, explicit transforms, ordered exact decimal guards, unguarded TYPE exclusion of identity, filters/renames, DATETIME wall time/TIME duration, defaults, comments and keys through native SQL. Second case exercises imported schema-only -> explicit append -> truncate -> selected recreate, retaining OIDs/trigger identity where required and outside sentinels. Both include strict import/secret/no-clobber/read-only boundaries. No credentials/environment mutation in the parent test process, no parser/runtime/model changes.

Use existing owned harness --start/--stop and ignored native test registration; a missing requested endpoint is failure, never skip. CLI children have finite test timeout and kill-on-drop, and only synthetic paths/fixtures are mutated. Record genuine unsupported/failing semantics instead of changing imported rules to obtain green.

## Native execution and source-catalog blocker

Implemented two ignored native cases in `tests/cases/t16_import_native.rs`; coordinator registered them behind `native-import-tests`. Owned pair `my2pg-mysql84-pg16-6f805bd1e9e8` runs MySQL 8.4.11 / PostgreSQL 16.15. Actual credentials are provided through child environment references; synthetic legacy credentials are checked for absence in generated TOML and CLI output. Runnable TOML adds only fixture CA paths/report location. Target mutations are confined to `t16_import_values` and `t16_import_policy`; source mutations use synthetic `T16Import*` tables. Existing `public.users` sentinel/OID is independently captured. No production edits or shared fixture changes.

Compilation and final strict all-target/all-feature Clippy pass. Both native cases fail (0/2) before migration because production source catalog inspection refuses the literal empty default on `source.T16ImportParent.empty_value`: `SourceError::Metadata("literal default source.T16ImportParent.empty_value")`. Catalog inspection reads all tables before selection/casts, so the second case is blocked by the first case's unrelated table too. Explicit imported DROP DEFAULT cannot bypass catalog acquisition. Neither failure is a skip/pass. Downstream migration value/default/identity/constraint checks and existing-policy execution checks remain unexecuted; full T16 native acceptance is open.

An initial independent PostgreSQL oracle used `$1::regclass`, which requires a regclass parameter rather than a text Rust argument; corrected the test oracle to `$1::text::regclass::oid`. Subsequent failures reproduce the production source issue, with no imported semantics or native assertions weakened.

Production `src/mysql/mod.rs:350–395` recovers literal CHAR/VARCHAR/ENUM/SET defaults using `CONVERT(DEFAULT(alias.column) USING utf8mb4)` over a false RIGHT JOIN, then requires a non-NULL decoded value. The new test probes distinguish direct DEFAULT semantics, server expression behavior, driver decoding and empty-table behavior. The populated Parent has two rows and `empty_value VARCHAR(20) NOT NULL DEFAULT ''`; enum `state` has default `alpha`.

Exact independent server queries/results (all run through the production MySQL driver; HEX and COALESCE are independent server-side witnesses):

```sql
SELECT HEX(DEFAULT(empty_value)),HEX(DEFAULT(state))
FROM T16ImportParent LIMIT 1;
-- ("", "616C706861")

SELECT HEX(CONVERT(DEFAULT(s.empty_value) USING utf8mb4)),
       DEFAULT(s.empty_value) IS NULL
FROM T16ImportParent s LIMIT 1;
-- ("", 0)

SELECT HEX(CONVERT(DEFAULT(s.empty_value) USING utf8mb4)),
       HEX(CONVERT(DEFAULT(s.state) USING utf8mb4))
FROM T16ImportParent s RIGHT JOIN (SELECT 1 AS anchor) a ON FALSE;
-- (SQL NULL, "616C706861")

SELECT DEFAULT(s.empty_value) IS NULL,
       CONVERT(DEFAULT(s.empty_value) USING utf8mb4) IS NULL,
       COALESCE(HEX(CONVERT(DEFAULT(s.empty_value) USING utf8mb4)),
                'SERVER_SQL_NULL')
FROM T16ImportParent s RIGHT JOIN (SELECT 1 AS anchor) a ON FALSE;
-- (0, 0, "SERVER_SQL_NULL")

SELECT HEX(COLUMN_DEFAULT),IS_NULLABLE
FROM information_schema.COLUMNS
WHERE TABLE_SCHEMA='source' AND TABLE_NAME='T16ImportParent'
      AND COLUMN_NAME='empty_value';
-- ("", "NO")

SELECT CONVERT(DEFAULT(s.empty_value) USING utf8mb4),
       COALESCE(CONVERT(DEFAULT(s.empty_value) USING utf8mb4),'SERVER_SQL_NULL'),
       COALESCE(DEFAULT(s.empty_value),'SERVER_SQL_NULL'),
       HEX(DEFAULT(s.empty_value))
FROM T16ImportParent s RIGHT JOIN (SELECT 1 AS anchor) a ON FALSE;
-- (SQL NULL, "SERVER_SQL_NULL", "SERVER_SQL_NULL", SQL NULL)
```

For a separate owned zero-row `T16DefaultProbeEmpty(value VARCHAR(20) NOT NULL DEFAULT '') ENGINE=InnoDB`, the same false-join `IS NULL / CONVERT IS NULL / COALESCE(HEX(...))` query returns `(0,0,"SERVER_SQL_NULL")`; probe table was dropped before inspection. The direct and metadata observations prove the legitimate empty-string default. Server-side COALESCE of bare DEFAULT and CONVERT returns the fallback, so this is a server expression/false-outer-join discrepancy, not a Rust string-decoding failure or an empty-table-only problem. `IS NULL` returns contradictory zero predicates in this setting; do not use these predicates as recovery proof. No general production fix is claimed by this test-only lease.

Commands/check evidence:

- `bin/cargo check --offline --all-targets --features native-import-tests`: passed.
- `bin/cargo clippy --offline --all-targets --all-features -- -D warnings`: passed after final probe.
- `bin/cargo test --offline --features native-import-tests --test integration t16_import_native:: -- --ignored --nocapture --test-threads=1`: failed 0/2 at the exact source Metadata condition.
- Final exact first case with `--ignored --exact --nocapture`: failed 0/1 with all server observations above; log `target/integration/my2pg-mysql84-pg16-6f805bd1e9e8/t16-import-native-sql-null-probe.log`.

Preserved evidence: pair `connections.json` (private fixture credentials; do not publish), initial/second/third/final probe logs in its artifact directory, and latest private imported/runnable config inputs under `t16-import-values-40184-1790911605447660000`. The actionable prerequisite is lossless production literal-default extraction that works on legitimate empty-string defaults for both populated and empty tables. Coordinator/source owner must resolve it before rerunning these unchanged importer acceptance cases.

Native pair lifecycle: final probes captured; `harness.py --stop .../my2pg-mysql84-pg16-6f805bd1e9e8/connections.json` completed with exit 0. Only this owned pair was stopped; evidence directory remains. Test and worklog are frozen for review; no T16 native success or runtime policy fidelity claim.
