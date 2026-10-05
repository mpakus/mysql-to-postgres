# Configuration and console contract

This document describes the implemented configuration and console contract, including the supported `.load` import subset and standalone verification. Release-matrix, platform, benchmark, fault-injection, packaging, and independent-review gates remain open; maintainers can find the current evidence in the task board and completion checklist in the source checkout. A parser check validates configuration only and does not certify a database migration.

## One executable configuration

Use versioned TOML with strict unknown-field rejection. Prefer explicit environment-variable references for connection URLs. Do not expand arbitrary strings through shell evaluation or global text substitution.

See [mysql-to-postgres.toml](examples/mysql-to-postgres.toml). The configuration groups familiar pgloader concerns: source, target, migration options, table selection, casts, session parameters, verification, and reporting.

| Field/group | Contract |
| --- | --- |
| `version` | Required integer schema version; unsupported versions fail |
| `source.url_env`, `target.url_env` | Name of the environment variable containing the URL; missing/empty values fail with the variable name, never its secret value |
| `source.consistency` | Required `frozen` or `single_snapshot`; semantics in [architecture](architecture.md#source-consistency) |
| `source.tls_mode`, `target.tls_mode` | `verify_full` by default; explicit `disable` for local test endpoints; CA/client certificate fields added by the connection task |
| `target.schema` | Required destination schema; no implicit use of `public` or a global search-path change |
| `target.on_existing` | `error` default; explicit `recreate` or `truncate`; `data_only` requires explicit `append` or `truncate` with target preflight checks |
| `migration.mode` | `full`, `schema_only`, `data_only`; invalid combinations fail |
| `migration.identifiers` | `preserve` default, optional `downcase`/`snake_case`; collision check required |
| `migration.table_workers`, `index_workers` | Positive global concurrency limits; cannot bypass consistency policy |
| `migration.readers_per_table`, `max_key_span` | Readers default to 1. Multiple readers require a positive `max_key_span` and `source.consistency = "frozen"`; ranges cover integer key space, not an estimated number of rows. `max_key_span` defaults to unset, so readers remain opt-in until T20 acceptance. The legacy `rows_per_range` field is not reinterpreted. |
| `migration.batch_rows`, `batch_bytes`, `queue_batches`, `memory_bytes`, `max_row_bytes` | Positive bounded values validated together; memory means the retained application-data budget, not total process RSS |
| `migration.on_row_error`, `max_rejected_rows` | `stop` by default; `reject` requires a positive explicit limit; zero is valid only with stop |
| `migration.reset_sequences` | Default true for full/schema-only; explicit choice for data-only |
| `tables.include`, `exclude` | Exact source names; empty include means all base tables, excludes win |
| `tables.include_regex`, `exclude_regex` | Explicit regex lists; unsupported regex features fail instead of changing meaning |
| `[[tables.rename]]` | Exact source identity to target table/schema mapping; use the same mapping for all references |
| `[[cast]]` | Ordered selector, optional typed guards, target type, null/default/typemod policy, named transform |
| `[source.session]`, `[target.session]` | Validated session settings applied to every relevant connection; failure is fatal |
| `[[overrides]]` | Exact source object ID plus an explicit omission or target expression/DDL; record all effects in the plan/report |
| `[hooks]` | Lists of PostgreSQL SQL file paths for before/after execution; relative to the config directory, shown with SHA-256 digests in the plan; each UTF-8 file is limited to 1 MiB |
| `[verification]` | Counts/schema by default; optional complete content mode; report the actual scope |
| `[report]` | Output directory, console format, progress policy; one fresh directory per run |

CLI overrides are limited to operational controls such as output, verbosity, and worker limits. They take precedence over TOML and appear in the resolved plan. Do not add three competing ways to specify a cast. Set each endpoint through exactly one `url_env` or `url_file` reference; URL files must be owner-private (`chmod 600` on Unix). A target may also name a PostgreSQL `passfile`. Missing credentials never fall back to a different database user silently. Implicit legacy MySQL option-file discovery is outside v1.

Offline and runtime memory validation share the checked reservation calculation: one data pipeline requires `W + (queue_batches + 2) × B`, where `B` includes batch bytes and row positions, and `W` includes raw/converted/encoded row workspaces and bounded encoder scratch. The extra two batches cover active COPY/recovery and producer filling or blocked send. Individual reservations must fit `u32` and the total budget must fit semaphore capacity. Schema-only requires no COPY reservation. The current runner admits bounded parallel table workers for frozen sources, caps admission by this budget, and reports reductions; single-snapshot mode retains one original reader plus its separately admitted control connection. T13 shutdown/resource acceptance includes the real stalled-filesystem artifact-worker test documented in the checklist. This budget does not bound driver/TLS/allocator overhead or process RSS.

The configuration task must freeze a schema/fixture for every field used here, including nested casts and overrides. Target types and expressions need validation; values that look like SQL cannot bypass identifier quoting.

For a single-part MySQL functional B-tree index, set the override object's exact source ID to `<database>.<table>.<index-name>` and provide a PostgreSQL `target_expression`. The expression replaces that key part and must match PostgreSQL's deparsed `pg_get_indexdef` form for structural verification; spell out implicit casts when PostgreSQL adds them. Multiple functional parts and other unsupported index semantics remain blocked unless the whole index is explicitly omitted.

For example, PostgreSQL deparses `lower(email)` over a `varchar` key as `lower((email)::text)`, so use the explicit cast in the override:

```toml
[[overrides]]
object = "source.users.email_lower_unique"
target_expression = "lower((email)::text)"
```

## Strict pgloader importer

The implemented offline subset requires an explicit destination schema and a reviewed choice of my2pg type/default semantics:

```sh
my2pg config import existing.load --output migration.toml \
  --target-schema legacy --consistency frozen --use-my2pg-defaults
my2pg check migration.toml
```

`import-load` is an alias for the same importer. `--source-env` and `--target-env` name the credential variables (defaults `MY2PG_SOURCE_URL` and `MY2PG_TARGET_URL`); import does not resolve them or connect to databases. `--append-data-only` is an explicit opt-in for the corresponding legacy policy. Omitting the reviewed-default choice fails rather than silently reproducing legacy inference.

This is a one-way migration aid, not a second executor. It parses one MySQL `LOAD DATABASE` command, preserves source spans, emits normalized TOML, and produces a compatibility report. An unsupported clause or ambiguous translation fails the import without leaving a runnable partial configuration. An existing output file is not overwritten implicitly.

The private, atomically published TOML includes the compatibility report as comments; sanitized JSON is also printed unless `--quiet` is selected. Publication failures return nonzero. A directory-sync failure after publication can leave a complete output file, so inspect it before retrying. Input is bounded to 1 MiB and the parser applies finite token, nesting and guard-expansion limits.

Credentials in a legacy URI are never copied into generated files or printed. Emit environment-variable references and a redacted instruction identifying which variables the operator must set. URLs are percent-decoded once; pgloader-specific escaping requires dedicated fixtures. The [subset example](examples/pgloader-subset.load) is a positive parser fixture, not a claim that every clause in the upstream MySQL examples is supported.

| pgloader input | Import decision |
| --- | --- |
| `LOAD DATABASE FROM mysql://... INTO postgresql://...` | Accept one source/target; `pgsql://` target alias may normalize to `postgresql://` |
| JDBC URLs, alternate source schemes, multiple commands | Reject with a specific diagnostic; do not guess driver-parameter meanings |
| `WITH create tables, create indexes, foreign keys, reset sequences` | Translate to full migration and explicit selected behavior |
| `include no drop`, `no truncate` | Recognize both; choose fail-on-existing unless an explicit data-only append policy is requested, and report this safer semantic difference |
| `include drop`, `truncate`, `data only`, `schema only` | Translate into the corresponding explicit target/mode policy; never add CASCADE |
| `create no tables` | Translate to data-only with target validation, preserving target constraints; requires `--append-data-only` or explicit truncate, and reports difference from legacy constraint removal |
| `workers`, batch row/byte limits, `max parallel create index` | Map to clearly named TOML limits |
| `concurrency`, multiple/single reader clauses | Accept the supported single-reader choice; multiple readers await T20 acceptance and require an explicit frozen-source range config |
| `prefetch rows`, `batch concurrency`, `rows per range`, `chunk size` | Currently refuse ambiguous units/semantics; use explicit TOML `max_key_span` only after reviewing key-space units |
| Quote/downcase/snake-case identifiers | Translate, then perform collision detection during planning |
| INCLUDING/EXCLUDING exact names and supported regex | Translate against original source identity |
| `ALTER SCHEMA ... RENAME`, table rename/set schema | Translate to explicit mappings with FK-aware resolution |
| `CAST` type/column rules | Support ordered matching, signed/unsigned/default/null/extra guards, bounded precision/scale comparisons, keep/drop policies, allowlisted transforms |
| S-expression cast guards | Parse a finite comparison/and/or grammar; never evaluate Lisp |
| Charset decoding and named existing views | Currently refuse; translate only after the corresponding M3 feature passes |
| `BEFORE/AFTER LOAD DO` | Currently refuse pending T19; future accepted translation must export SQL files and include their digests in review output |
| `drop schema`, `disable triggers`, automatic index removal, table storage/tablespace clauses | Reject in v1; suggest an explicit reviewed SQL hook where appropriate |
| Source-view definitions, INI/context, external transforms, Citus, file/archive clauses | Reject as outside product scope |

Every accepted clause requires an assertion on normalized configuration **and** a behavioral test. Parser success alone is not compatibility. Legacy implicit type choices must be listed: boolean inference, integer display width, DATETIME timezone, TIME duration, defaults, and identifier casing. Emit explicit rules for supported legacy choices, or stop with a migration diagnostic.

Current subset limits: numeric guards apply only to explicit decimal/numeric TYPE rules; guarded COLUMN rules are refused. An unguarded TYPE rule explicitly excludes AUTO_INCREMENT columns, matching the legacy matcher. Name/default matching is exact and case-sensitive, and the report discloses that difference. Legacy URI escapes, ambiguous reader/prefetch/range units, hooks, views and charset decoding are refused until their matching execution features are accepted. The importer has focused fixture, CLI and planner coverage; importer-driven native migration compatibility remains an open acceptance gate.

## Commands

These commands describe the current interface. Maintainers should consult the source checkout's task board and worklogs for native database lanes and release gates that remain unverified:

```text
my2pg check migration.toml
my2pg plan migration.toml --output text
my2pg plan migration.toml --output json > plan.json
my2pg run migration.toml
my2pg run migration.toml --output json > events.jsonl
my2pg verify migration.toml --run-dir reports/run-<id> --mode content
my2pg config import old.load --output migration.toml --target-schema legacy --consistency frozen --use-my2pg-defaults
```

- `check`: local syntax, fields, supported URI schemes, and policy consistency. No database connection and no secret output.
- `plan`: read-only connections and metadata inspection, chosen DDL/types/names, exclusions, warnings, estimated sizes, resource limits. A JSON plan is an inspection artifact; v1 does not execute a saved stale plan.
- `run`: fresh preflight and migration. Always write the final report if the output device permits. The plan records effective options and destination identity.
- `verify`: read-only comparison using the explicit completed run's `plan.json` and `report.json`. The report binds the plan and password-free source/target endpoint identities. Use the same endpoints that ran the migration; password rotation is allowed. It scans the MySQL source and PostgreSQL target again, so keep the source dataset unchanged since the run. It cannot reattach to the original `single_snapshot` or recover an append baseline, so append count verification reports unsupported and complete content verification of nonempty append targets remains unsupported.
- `config import`: offline translation with compatibility diagnostics. No database access.

## Console behavior

Proposed TTY display:

```text
my2pg  source: shop@mysql:3306  target: warehouse@postgres:5432 / legacy
phase: copy     tables: 7/24     committed: 4,210,000     rejected: 0
throughput: 182,000 rows/s       encoded: 37 MiB/s       elapsed: 00:23
orders          copying       2,140,000 rows      total: estimated
order_items     building keys 3,960,000 rows
customers       complete        184,000 rows
```

The final display reports copied rows, rejected rows, failed tables/DDL, verification scope, elapsed time, and report paths. It must distinguish `complete`, `partial`, `failed`, `cancelled`, and `indeterminate` outcomes. Do not show 100% based on estimated MySQL row counts or treat completed COPY as a completed table while indexes are running.

TTY mode uses a compact multi-table progress display, refreshed at most a few times per second. Non-TTY mode emits ordinary phase/table lines without cursor control. Support `NO_COLOR`, `--color auto|always|never`, `--progress auto|always|never`, `--quiet`, and `--verbose`. Preserve readable ASCII output and terminal-width truncation. Do not log every row.

`--output json` writes only versioned JSON events to stdout; human diagnostics go to stderr. `check`/`plan` emit one JSON document, while `run` emits JSON Lines with a final outcome event. A renderer failure or closed pipe triggers controlled shutdown; it must not disappear while leaving an unobserved migration running.

Diagnostics include a stable code, stage, qualified object, optional field/span, and actionable next step. Raw values, SQL error DETAIL payloads, passwords, and full connection strings stay out of ordinary logs. Debug mode does not disable redaction.

## Artifacts and status

Create an exclusive per-run directory containing `plan.json`, `report.json`, optional event log, and reject files. Use safe generated table IDs for filenames and preserve a source-name lookup in the report. Keep artifacts private to the local user where supported. Never derive a path directly from a database identifier.

For server-rejected encoded rows, persist COPY data plus structured reason/locator metadata. For conversion failures, persist bounded raw typed values with binary represented losslessly and an explicit encoding. Those records are evidence for a later correction; v1 does not automatically replay them. Flush reject/report output and surface I/O failure.

### Manual restart after a partial run

Read `report.json` before starting again. Confirm the outcome (`partial`, `failed`, `cancelled`, or `indeterminate`), committed and rejected counts, failed steps, unresolved work, verification coverage, and artifact paths. Preserve the run directory and reject files while investigating; they are the evidence needed to decide what to correct.

my2pg does not resume a run or replay reject files. Correct rejected source data or configuration first, then rehearse the change. The safest restart is a new empty destination with the default `on_existing = "error"`. To reuse the same destination, inspect the saved plan and report, take any required backup, and explicitly choose a reviewed `recreate` or `truncate` policy. `recreate` is limited to selected objects and fails when outside dependencies prevent a safe drop; `truncate` clears selected target data. Neither choice resumes from the last committed batch: the next run starts from the beginning.

For an `indeterminate` commit outcome, first reconcile the affected target table against the source and report. Never retry the uncertain batch automatically or treat the committed row count as proof that the batch is absent. Use `plan` to inspect the fresh target state and resolve the uncertainty before choosing a restart policy.

| Exit code | Meaning |
| --- | --- |
| 0 | Requested operation completed, required work and requested verification passed, zero rejected rows |
| 1 | Execution failure, required DDL failure, artifact failure, or uncertain commit outcome |
| 2 | Configuration, unsupported feature/version, connection/preflight, or planning failure before migration writes |
| 3 | Migration finished with rejected rows, without a stronger failure |
| 4 | Verification found differences or could not cover its requested scope, without an execution error |
| 130 | Operator cancellation, unless an indeterminate commit requires the stronger failure status |

When multiple outcomes occur, execution/indeterminate failure wins, then cancellation, then verification differences, then rejected rows. Preserve all details in `report.json`. A schema-only success identifies its mode and zero copied rows; it is not presented as a data migration success.
