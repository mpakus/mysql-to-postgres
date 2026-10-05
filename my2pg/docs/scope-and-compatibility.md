# Scope and compatibility

This is the proposed v1 contract and distinguishes historical behavior from the compatibility choices below. **M1 is a working alpha; M4 is the complete v1 acceptance gate.** A feature listed for M2 or M3 is not optional merely because M1 works.

## Supported product

One migration direction: a network connection to **Oracle MySQL** as source, and **PostgreSQL** as target. The initial test contract is MySQL 5.7, 8.0, and 8.4 against PostgreSQL 16, 17, and 18. These are planned compatibility targets, not currently certified versions or a recommendation to deploy old MySQL releases.

MySQL 5.7 belongs in an isolated compatibility test lane. MySQL 8.4 and PostgreSQL 16 are the initial development pair. Detect server versions and recognizable variants before migration; report unknown variants/versions as unsupported until tested. MariaDB, TiDB, Vitess, and other compatible products are not implicitly covered by accepting `mysql://`.

## Feature disposition

| Capability | Delivery | Behavior |
| --- | --- | --- |
| Table/column discovery, exact names and column order | M1 | Separate original MySQL identifiers from destination identifiers throughout the plan |
| Full migration, schema only, data only | M1 full; M2 all modes | Each mode has explicit DDL, COPY, and sequence semantics |
| Basic numeric/text/date/binary values and NULL | M1 | Streaming without whole-table materialization |
| Full type/default/enum/set coverage below | M2 | Unknown or lossy mappings require an explicit policy |
| PK, UNIQUE, ordinary secondary indexes, FKs, comments | M2 | Rebuild after COPY where appropriate; required failures prevent success |
| Filters and table/schema rename mappings | M2 | Exact matches or explicit regex, original identifiers used for matching; update all references |
| Custom casts and built-in transforms | M2 | Ordered rules, first match wins, column or type selectors, typed guards |
| Multiple tables in parallel, bounded prefetch | M2 | Global connection and byte limits; cancellation is propagated |
| Data rejects, stop/continue policy, reports | M2 | Nonzero outcome for rejected data; no silent row loss |
| TLS verification, modern authentication, env/file credentials | M2 release requirement | Driver feasibility established in M0; no implicit TLS downgrade |
| Content verification and structural verification | M2 counts/schema; M3 content | Verification coverage and transformation choices appear in reports |
| `.load` import | M3 | Strict one-way import of the documented MySQL subset |
| CHECK, generated expressions, expression indexes | M3 | Tested expression subset or explicit target-expression override; otherwise block planning |
| `ON UPDATE CURRENT_TIMESTAMP` | M3 | Opt-in tested emulation or explicit omission; do not claim general trigger translation |
| Existing MySQL views copied as tables | M3 | Read existing views and copy their evaluated rows; no target SQL-view translation |
| Before/after PostgreSQL SQL files | M3 | Explicit operator code, visible in the plan; never executed during `check`/`plan` |
| Raw charset correction | M3 | Per-table/column override with strict decoding and tests for mislabeled data |
| Spatial values | M3 | Optional PostGIS target, preserve geometry and SRID; detect extension, never auto-install it |
| Integer PK range readers | M4 | Opt-in after correctness and memory gates; single-reader fallback is explicit |
| Linux/macOS binaries, container, machine-readable output | M4 | No Lisp, JVM, JDBC, or source server libraries required at runtime |

## Deliberate exclusions

Remove SQLite, SQL Server, PostgreSQL-as-source, CSV, COPY-file-as-source, fixed-width, DBF, IXF, archives, HTTP source downloads, S3/Redshift, and Citus distribution from the new product. PostgreSQL COPY remains the **target transport**; its name does not imply a supported source format. Negative tests must reject these sources before opening a target connection.

Also exclude source writes, inline source-view creation, arbitrary Lisp/Clojure transforms, plugin loading, old INI/mustache contexts, a web UI, a full-screen TUI, CDC/binlog replication, online cutover automation, and automatic interrupted-run resume. Do not copy the old generic `Source` protocol into Rust.

General stored routines, events, triggers, users/grants, partition layout translation, and SQL-view definitions are inventoried and reported for manual migration. In-scope tables are not advertised as preserving these behaviors. Unsupported table/index/column semantics block execution until explicitly overridden or acknowledged as omitted in configuration.

FULLTEXT, SPATIAL index definitions, prefix indexes, MySQL collations, and storage options have no blanket PostgreSQL equivalent. v1 must detect them and support explicit omission or a reviewed target SQL replacement. A simple GIN index is not automatically equivalent to MySQL full-text search. Never silently convert a prefix UNIQUE index into a different uniqueness rule.

## Type and value contract

The planner resolves a `ColumnPlan` once. DDL, transforms, encoding, defaults, and verification all use that same decision. User rules are ordered and run before built-in rules. A type-specific override may widen a type; data-dependent narrowing requires validation.

| MySQL type/behavior | Proposed default or required decision | Tests |
| --- | --- | --- |
| Signed TINYINT / SMALLINT / MEDIUMINT / INT / BIGINT | `smallint` / `smallint` / `integer` / `integer` / `bigint`; display width does not change range | Every min/max, display widths, negative values |
| Unsigned integers | Widen small types; unsigned INT → `bigint`; unsigned BIGINT → `numeric(20,0)` | `2^63`, `2^64-1`, zero, exact decimal round trips |
| AUTO_INCREMENT | Integer identity where representable; preserve values and source next-value lower bound | Empty/all-zero/negative-only tables, deleted high IDs, exhaustion, non-default increment/offset |
| Unsigned BIGINT AUTO_INCREMENT | Numeric data is supported; automatic generation beyond PostgreSQL sequence range needs an explicit policy | Never narrow the data or overflow a signed sequence silently |
| TINYINT(1) | Preserve integer by default; explicit boolean rule accepts only configured values | 0, 1, NULL, -1, 2; pgloader nonzero→true requires explicit transform |
| BIT(n) | `bit(n)` with exact width; optional BIT(1) boolean rule | Leading zeros, widths 1/8/64, defaults |
| DECIMAL(p,s) | `numeric(p,s)` with exact textual/decimal path; never intermediate float | MySQL maximum precision/scale, exponent rejection, sign, trailing scale |
| FLOAT / DOUBLE | `real` / `double precision` | Precision, finite extremes, explicit comparison policy |
| CHAR/VARCHAR/TEXT | Preserve declared lengths where meaningful; target `varchar(n)`/`text` | Trailing spaces, multibyte lengths, empty vs NULL, collation caveats |
| BINARY/VARBINARY/BLOB | `bytea`, retain every byte | NUL, all 256 byte values, padding, blobs larger than a batch limit |
| DATE | `date`; invalid/zero dates reject by default | Zero month/day, partial-zero dates, invalid calendar dates, NULL |
| DATETIME(fsp) | `timestamp(fsp) without time zone` | Preserve wall time, fractions, DST edge values; explicit cast required for timezone interpretation |
| TIMESTAMP(fsp) | Read with MySQL session timezone UTC; `timestamptz(fsp)` | UTC instants independent of machine timezone; fractions |
| TIME(fsp) | `interval` preserving sign and durations beyond 24 hours | ±838:59:59 and fractional values; optional `time` cast rejects out-of-range durations |
| YEAR | `smallint` preserving the numeric value | Zero, 1901, 2155 |
| JSON | `jsonb`, with documented semantic normalization | Unicode, nesting, large numbers, JSON null vs SQL NULL |
| ENUM | Named PostgreSQL enum, preserve order/labels; explicit `text` override | Empty labels, quotes, backslashes, commas, name collisions, invalid MySQL enum sentinel |
| SET | `text[]` with membership CHECK against declared labels | Empty set vs NULL, commas/quotes/backslashes, unambiguous source decoding |

Native MySQL8.4 evidence shows cached default metadata can truncate binary values at the first NUL and replace supplementary Unicode. The source inspector recovers present literal BINARY/VARBINARY/CHAR/VARCHAR/ENUM/SET defaults with guarded read-only `DEFAULT(column)` queries; expression/generated/view defaults are not evaluated. Independent default inserts prove the recovered values, rather than verification against the same plan alone.

ENUM/SET ordered label metadata has a separate server limitation: system catalog and SHOW CREATE output can replace supplementary characters with `?`. When selected labels contain a question mark in a source charset that may encode those characters, planning fails with `LOSSY_LABEL_METADATA`, including literal question marks that cannot be distinguished from replacement. Text casts and ineffective omission overrides cannot recover ordered labels. Unselected tables and proven BMP-only metadata remain usable. Lossless supplementary label recovery and actual column exclusion are not implemented; this explicit refusal prevents ordinal/mask conversion from silently using corrupted labels.
| Geometry | Optional preinstalled PostGIS with per-value SRID-aware EWKT encoding; never auto-install the extension | MySQL-supported 2D geometry types, empty GeometryCollection, NULL, and mixed SRIDs; missing PostGIS blocks before target DDL |
| Unknown type | Blocking diagnostic, with source object and override guidance | No implicit fallback to text |

These defaults deliberately differ from pgloader in boolean inference, display widths, durations, and timezone treatment. The `.load` importer must surface the differences and materialize explicit rules when it can reproduce a legacy choice.

MySQL's supported spatial storage formats use X/Y coordinates and permit an empty GeometryCollection; MySQL does not accept other empty geometry values or Z/M coordinates in its documented geometry formats. The spatial path preserves that source representation and each row's SRID when writing PostGIS EWKT. See [MySQL spatial data formats](https://dev.mysql.com/doc/refman/8.0/en/gis-data-formats.html).

SQL NULL, empty string, string `NULL`, string `\\N`, empty binary, and empty arrays remain distinct. Embedded NUL and invalid byte sequences reject by default for PostgreSQL text. Removal/replacement is an explicit transform with counters and verification expectations.

Defaults are typed literals or a supported expression, not arbitrary copied MySQL SQL. Preserve empty-string defaults. A zero-date-to-NULL rule must also reconcile a zero default and NOT NULL constraint; reject contradictory configuration before DDL. Collation-sensitive uniqueness and trailing-space comparisons require specific tests and declared target choices.

## Structure and naming contract

- Require a destination schema. Examples use `legacy`; the application is not tied to Rails or a particular database name. Fully qualify every generated PostgreSQL object reference.
- Preserve and quote identifiers by default. Optional downcase/snake-case maps must detect collisions before DDL, including collisions after PostgreSQL's identifier byte limit.
- Allocate deterministic index/type/constraint names from stable source identity, with UTF-8-aware truncation and a short digest. Do not use target OIDs for names.
- Preserve composite key order, unique vs ordinary indexes, FK actions, source/destination mappings, and nullability. Check references after include/exclude filters and renames; missing referenced objects are blocking by default.
- Introspect per-index parts as rows. Avoid comma-separated metadata aggregates that lose order, prefix length, quoting, or expression structure.
- Generated columns need a supported target expression and are omitted from COPY. Unsupported expressions require a target override or explicit materialization as an ordinary column.
- Schema-only applies all selected DDL and initializes identity metadata without COPY. Data-only validates existing column/type compatibility and preserves existing constraints/triggers; it never removes them automatically. Data-only requires an explicit `append` or `truncate` existing-target policy, and sequence adjustment is separately explicit.

## Scope completion

v1 is complete when every selected feature has either a passing acceptance case or a tested explicit rejection/override path, every supported server lane passes, and the benchmark report supports the released performance claims. A run with clean row counts but missing required constraints is incomplete.
