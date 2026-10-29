# Strangler migration lessons in My2pg

Reviewed on 2026-10-29 against Strangler branch `TBDEV-5749-mysql-to-postgres`, revision `486a44e2933d0bd72c1ec411f520a470e3f738cf`. Source evidence includes `m2p/mysql-to-postgres.load`, `m2p/migration_policy.rb`, `m2p/compare.rb`, `lib/tasks/legacy/prepare.rake` and the disposable loader integration checks. My2pg implements applicable behavior directly in Rust; no JVM, patched JAR or Rails boot is needed. The source checkout's `worklog/2026-10-29-strangler-migration-lessons.md` records this comparison and validation; earlier FULLTEXT and binary parity evidence remains in the October 8 and 19 worklogs.

| Strangler behavior | Rust behavior |
| --- | --- |
| Loader exits nonzero on fatal errors or recovered/rejected rows | Run reports select a nonzero exit for incomplete migrations; rejected rows are never success |
| Fatal DDL, index, constraint, sequence or after-load SQL errors | Required steps fail the run; earlier acknowledged COPY batches remain accurately reported |
| Preserve the first exception across cancelled COPY/index workers | Per-run failure registry retains the originating fault; indeterminate commit evidence takes precedence |
| Full reader queue must not lose terminal failure; writer failures unblock readers | Bounded async channels, cancellation and scoped worker shutdown replace JVM queues/sentinels |
| Propagate on-error-stop and connection settings into workers | Every Rust worker receives resolved options; stop is the default, and source UTC is established on connections |
| Skip after-load SQL on failed/rejected loads | After hooks require clean data/DDL completion |
| Reject unknown options, extra JAR/positional arguments; accept `-v` | Strict clap grammar and the existing verbosity alias |
| Disable JDBC tinyint boolean inference and JVM timezone conversion | MySQL raw values preserve numeric states; TINYINT(1) stays integer by default; TIMESTAMP sessions use UTC |
| NULL-safe, single/multiple-column FULLTEXT → GIN | Built-in `migration.fulltext = "simple_gin"`; preserves terms beside NULLs and separates column boundaries; plans disclose changed search semantics |
| Preserve encrypted/serialized bytes stored in VARCHAR/TEXT | Explicit `bytea` casts use raw MySQL binary projection in whole-table, range and verification readers; charset overrides never decode these bytes |
| Valid bytea defaults, including empty strings | Character defaults retain their original source bytes separately from decoded metadata; typed PostgreSQL escape literals preserve NUL, high bytes, backslashes and empty values |
| Set result encoding on each JDBC reader | Native MySQL connections use utf8mb4; text is read as raw bytes and strictly decoded locally, while bytea bypasses decoding |
| Repair sequence floors without rewinding | Existing required Rust finalization respects source AUTO_INCREMENT, loaded MAX+1 and current called/uncalled state; rejects changed ownership/options, missing privileges and exhaustion |
| Restore captured zero dates without changing genuine NULLs or reviving discarded records | Explicit `zero-dates-to-epoch` converts complete zero dates during COPY and in literal defaults; NULL remains NULL, partial/invalid dates fail, and reports count actual replacements |
| Compare the approved source transformation against exact target timestamps | Content mode supports DATE, DATETIME and TIMESTAMP through microseconds; target NULL/sentinel and timestamp changes remain detectable |
| Keep long reconciliation work outside the final cutover pause | Known content-verifier coverage gaps now block planning before COPY; fresh migrations can select complete content verification up front |

## Fresh zero-date migration without a restore pass

Strangler's pgloader flow captures source keys, loads zero dates as NULL, then updates only captured target cells to `1970-01-01 00:00:00+00`. That preserves soft deletion but needs an extra source scan, a private key manifest and target UPDATE/trigger handling. For a **fresh my2pg run**, choose the same sentinel directly for each reviewed column:

```toml
[[cast]]
source_table = "products"
source_column = "discarded_at"
target_type = "timestamptz(6)"
transform = "zero-dates-to-epoch"

[verification]
mode = "content"
```

Only `0000-00-00` or an entirely zero date/time, including zero fractional seconds, is replaced. A date becomes `1970-01-01`; a wall timestamp becomes midnight without a timezone; `timestamptz` becomes UTC midnight. Genuine NULLs remain NULL, valid dates remain exact, and values such as `2024-00-03` or `0000-00-00 00:00:01` fail. The rule also maps a complete zero literal default, retaining NOT NULL when applicable. `drop_default = true` remains available when the application should have no default. Genuine source NULLs still cannot enter a NOT NULL target.

This is an explicit semantic choice: original zero dates and genuine epoch values converge. Keep a source backup if their original distinction matters. The raw driver exposes zero components, so this rule avoids the capture/restore passes for those freshly copied columns; no elapsed-time speedup has been benchmarked. It does not repair an earlier import where NULL provenance was already lost. Keep Strangler's reviewed prepare/restore flow for its existing pgloader path.

Content mode compares full rows, including keys and duplicate multiplicity. Only source values receive the configured transform; the target is compared exactly. A sentinel changed to NULL, a genuine NULL changed to a sentinel, or a one-microsecond timestamp edit causes a difference. Sessions use ISO dates and UTC, and fractional padding is normalized without rounding. Float, JSON, enum/set, bit and interval content remain outside the current canonical subset; requested unsupported coverage fails planning before target writes. Use counts/schema plus an independent reconciliation for such a plan, or extend the verifier with explicit tests before claiming complete content equality.

## Application decisions kept explicit

| Strangler lesson | My2pg decision |
| --- | --- |
| Preserve source decimal precision/scale and integer flag states | Already built in; no float intermediate for DECIMAL, no implicit TINYINT(1) boolean conversion |
| Round two selected price-tier columns to `numeric(15,2)` | No automatic rounding: this application-approved lossy policy is not a general type mapping; narrowing remains checked |
| Distinguish selected tables from recoverably archived exclusions | Keep exact filters, exclusion reports and an operator preservation manifest; filtered rows are not migrated rows |
| Separate PostgreSQL `mysql` and `public`, preserve Rails migration ledgers | Explicit target schema and application deployment/ledger work; my2pg does not alter Rails history |
| Validate target writability, sequence floors, rejects and required structures | Use the preflight/reconciliation runbook; a completed COPY or green row count alone is insufficient |
| Baseline plus CDC/final delta for a short writer pause | Operator cutover responsibility; CDC, online cutover and automatic resume remain outside this app's scope |

Retain the authoritative source until reconciliation and recovery rehearsal finish. Run imports against a writable primary, keep PostgreSQL application writers fenced, inspect exclusions and rejects, and validate the source AUTO_INCREMENT lower bound as well as loaded IDs. After PostgreSQL accepts application writes, simply switching the application back to MySQL is not a safe rollback.

## Existing loader parity

For a Strangler byte-bearing text column, use the existing cast mechanism:

```toml
[[cast]]
source_table = "orders"
source_column = "cc_number"
target_type = "bytea"
```

Repeat only for the selected columns that contain bytes. `transform = "binary-to-bytea"` is also accepted, including by the supported `.load` importer. It is a byte-preserving alias; a plain `bytea` cast already performs the correct read. `hex-to-bytea` remains a separate explicit operation for textual hexadecimal content. Ordinary text columns still use strict charset conversion. My2pg does not infer these policies from application table/column names.

Defaults inspected before this metadata extension lack the original character bytes. Reinspect the source before planning a text-to-bytea default; the planner refuses to guess from decoded strings. Live runs always inspect fresh metadata.

Enable the FULLTEXT replacement in your normal TOML:

```toml
[migration]
fulltext = "simple_gin"
```

See [configuration and matching search queries](config-and-cli.md). The decision is explicit because PostgreSQL's `simple` dictionary replaces MySQL FULLTEXT behavior. Exact-object omissions override this mapping. Nontext target columns or unsupported FULLTEXT shapes are rejected before target mutation.

Strangler-specific table exclusions, the `mysql` destination schema, column casts, zero-date policy and the warehouse `'00'` default hook remain migration configuration/application responsibilities. My2pg never infers them from names and never writes to the MySQL source. Its `.load` importer accepts the new named epoch transform, but remains a strict documented subset; the full Strangler load file includes unsupported clauses and is not a drop-in configuration.

The FULLTEXT regression runs through the real CLI on owned TLS fixtures, then checks live GIN indexes, query results, EXPLAIN usage, existing-target binding and saved-run drift detection. The binary regression runs serial and parallel readers and compares every byte using independent source `HEX()` and target `encode()` queries, including inserted defaults discovered with/without source rows. Since the 2026-10-27 verification follow-up, built-in content verification also supports bytea and its approved transforms. The real CLI regression verifies content, detects same-length byte corruption, and succeeds again after restoration. Counts/schema alone do not establish byte equality. Existing pipeline/hook/CLI regression suites cover the reliability behavior.
