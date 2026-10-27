# Strangler pgloader patches in My2pg

Reviewed again on 2026-10-19 against Strangler `m2p/`, revision `fa317f5433bc63b0da83f47825beef37a6161d48`. The toolkit moved from `lib/m2p/`; `loader.clj` is unchanged since the Oct 8 audit. My2pg implements the loader fixes directly in Rust; no JVM, patched JAR or wrapper is needed. The source checkout's `worklog/2026-10-19-strangler-binary-parity.md` records the new patch hashes, reference searches and test results; `worklog/2026-10-08-strangler-loader-builtins.md` retains the earlier FULLTEXT/reliability evidence.

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

Strangler-specific table exclusions, the `mysql` destination schema, column cast choices, zero-date policies, the warehouse `'00'` default hook and Rails discarded-record restoration remain migration configuration/application responsibilities. In particular, the new Strangler zero-date-to-epoch restoration and price-tier rounding are not automatic My2pg behaviors: lossy numeric casts still require a supported explicit policy, and invalid dates fail unless an accepted transform is configured. My2pg never writes to the MySQL source. Its `.load` importer is a strict documented subset; the full Strangler load file still includes unsupported clauses and is not a drop-in configuration.

The FULLTEXT regression runs through the real CLI on owned TLS fixtures, then checks live GIN indexes, query results, EXPLAIN usage, existing-target binding and saved-run drift detection. The binary regression runs serial and parallel readers and compares every byte using independent source `HEX()` and target `encode()` queries, including inserted defaults discovered with/without source rows. Since the 2026-10-27 verification follow-up, built-in content verification also supports bytea and its approved transforms. The real CLI regression verifies content, detects same-length byte corruption, and succeeds again after restoration. Counts/schema alone do not establish byte equality. Existing pipeline/hook/CLI regression suites cover the reliability behavior.
