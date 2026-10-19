# Strangler follow-up: raw text bytes and binary defaults

Status: implemented and validated on the owned MySQL 8.4/PostgreSQL 16 fixture. Coordinator owns this T05/T10/T16 follow-up and shared index refresh. Existing Oct 8 FULLTEXT work is preserved.

## Source audit before coding

- My2pg HEAD: `a26e837d4b2436708d22d9e66376fcf75cdcd083`, with the existing uncommitted FULLTEXT implementation.
- Strangler HEAD: `fa317f5433bc63b0da83f47825beef37a6161d48`. Toolkit moved from `lib/m2p/` to `m2p/`; only `m2p/README.md` is locally modified. Read loader, patch, offline/native checks, load configuration, migration policy and sequence repair; no Strangler edits.
- `loader.clj` SHA-256 `2f6ea0ada9e2fe43456abbda6da5436963f94d672561693123f48a8cc74d0678` is unchanged from Oct 8.
- `pgloader.patch` SHA-256 `efd5de8dfaa903cb324366f8437892bd39005d2de5dac1cc743748e867bc88bb`; `check_loader.clj` SHA-256 `83dad259008200de0431c8b2e3f4ea5ca607ded935e40d20518a37dae61f1c2a`.
- New patch hunks select text explicitly mapped to bytea as binary, bypass charset decoding for it, initialize JDBC result encoding on every reader, and use PostgreSQL escape literals for bytea defaults. The native source test compares HEX values including NUL, C1 bytes, empty strings and NULL.
- Existing Rust code already retains first faults/nonzero exit, stops failed required DDL/hooks, preserves numeric tinyint states and UTC, and implements opt-in NULL-safe FULLTEXT. `postgres/sequences.rs` already uses source AUTO_INCREMENT, loaded MAX+1 and current called/uncalled state without rewinding, with ownership/options/privilege/exhaustion checks. Sequence changes are explicitly nontransactional; this is migration finalization, not Strangler's standalone all-schema repair utility.
- Strangler's table exclusions, epoch restoration of particular zero dates, and rounded price-tier fields are application policy, not new loader patch hunks. My2pg's strict explicit conversion contract remains in force; no automatic rounding or blanket zero-date substitution is introduced.

## Reference research (before implementation)

- XERJ `project-my2pg`, query `binary bytea encoding source`: noisy benchmark/fixture hits; narrowed to symbol `select_sql` and `default_sql`, both at current HEAD metadata. Read `src/mysql/mod.rs:347-420,551-593,653-683`, `src/plan/mod.rs:248-440,690-840`, `src/convert/mod.rs:285-385,580-632`, and adjacent `tests/cases/t10_binary_default.rs`, `t10_values.rs`, planner/conversion tests. Actual gap: text-to-bytea skips raw projection; textual defaults use UTF-8 metadata bytes instead of original source bytes.
- XERJ `project-pgloader`, symbol `convert-mysql-value`, pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `clojure/src/pgloader/source/mysql.clj:200-280` and `clojure/test/pgloader/source/mysql_test.clj:1-49`; compare with Strangler's new patch and `check_loader.clj:77-101`. Preserve raw bytes without adopting JDBC's intermediate X-prefixed strings.
- XERJ `ref-mysql_async`, query `character_set_results init`, pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`: returned connection init hooks rather than handshake. Local symbol search finds `src/conn/mod.rs:609-618` choosing UTF8MB4_GENERAL_CI for supported servers. Every My2pg reader uses a fresh initialized connection; keep native raw projections and test each reader's effective encoding rather than adding JDBC options.
- Existing default tests independently insert MySQL and PostgreSQL defaults. Extend that approach: retain a separate optional hex representation of CHAR/VARCHAR literal defaults during existing read-only metadata discovery. Do not reverse-decode already converted metadata or infer that a literal starting `0x` is binary.

## Chosen adaptation and validation

- [x] Raw binary projection for explicit text-to-bytea casts, shared by whole-table/range/verification readers.
- [x] Preserve source-encoded literal default bytes alongside decoded metadata; use them for bytea DDL, fail closed when required byte evidence is absent.
- [x] Accept pgloader's `binary-to-bytea` spelling as the existing byte-preserving transform; no extra conversion stage/dependency.
- [x] Unit checks for quoted identifiers, exact default bytes, empty/NULL distinction, missing/malformed evidence, transform compatibility/import.
- [x] Real CLI regression with serial and parallel readers, strict decoding override, all 256 bytes, Latin-1 text, empty/NULL and inserted defaults; independently compare MySQL HEX to PostgreSQL encode.
- [x] Run adjacent default/value/sequence coverage and ordinary checks; update parity/config docs.
- [x] Refresh project XERJ and verify new source/test/worklog coverage.

## Results

- The native test initially exposed fixture issues (prepared arbitrary-byte input needed `UNHEX` plus explicit Latin-1 conversion). Once corrected, it reproduced the actual bug before production edits: PostgreSQL stored UTF-8 expansions for source bytes `80..FF`. The independent HEX comparison failed despite a successful loader exit/counts/schema check.
- The planned content-verification check returned `unsupported`: `verify/canonical.rs` accepts a narrower subset without Binary. Kept that honest boundary; byte equality is asserted independently in this native regression, while the CLI runs counts/schema verification. No content-verifier feature expansion.
- `SourceColumn.default_bytes_hex` is optional/omitted when absent, preserving older catalog deserialization. Existing discovery fetches it in the same read-only default query. The planner requires it for character-to-bytea literal defaults rather than substituting decoded UTF-8. Ordinary text and native binary defaults retain their existing representation. No new dependency or copied reference implementation.
- `rtk run 'my2pg/bin/cargo test --offline --locked --all-targets --features artifact-worker-tests,native-import-tests -- --test-threads=1'`: passed all ordinary targets; fixture-dependent cases remained explicitly ignored in this command.
- `rtk run 'python3 -m unittest discover -s my2pg/tests/support -p "test_*.py"'`: 40 passed, including packaging and native inventory checks. The added test uses SQL applicable to 5.7/8.0/8.4; static inventory now has 158 MySQL cases, 135 5.7 run candidates and 23 source-feature exclusions (131 Linux-visible, 135 Darwin-visible). This is not runtime evidence for other MySQL versions.
- `cargo clippy --offline --locked --all-targets --features artifact-worker-tests,native-import-tests -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`: passed through RTK/local toolchain wrapper.
- Owned TLS fixture: `target/integration/my2pg-mysql84-pg16-c35b23fb1ed7`, MySQL **8.4.11**, PostgreSQL **16.15**, Linux/ARM64 database images; native macOS Rust test/CLI clients. Endpoint credentials remain only in ignored fixture files.
- Native command: `my2pg/bin/cargo test --offline --locked --test integration --features artifact-worker-tests,native-import-tests <selector> -- --ignored --nocapture --test-threads=1`, with the fixture's environment loaded privately. **14 passed** across `t10_text_bytea` (1), `t10_binary_default` (1), `t10_empty_string_default` (1), `t10_values` (2), `t10_mysql_projection` (1), `t16_import_native` (2), `t11_monotonic_sequences` (4), `t11_truncate_sequence_reset` (1), `t15_default_complete` (1). Full outputs are `<selector>-20261019.log` in that fixture directory.
- The final new-case rerun passed after adding an event assertion proving exactly two key ranges for the parallel run and zero for the serial run. Both modes check all 256 bytes, quoted names, empty/NULL, ordinary Latin-1 text, exact defaults from empty/populated source catalogs, literal `0x41` versus explicit hex decoding, and unchanged source row count.
- Worklog is exposed to Git with intent-to-add because the user's global ignore excludes `worklog/`; no commit or existing edits were staged. Reference pins and Strangler files remain unchanged.
- Project XERJ rebuild passed: 470 files, 1599 passages, 14 exclusions. Coverage explicitly includes the changed reader/planner, new native test and this worklog. Changed Markdown links resolve. The owned fixture was stopped successfully; failure-attempt database objects were removed with its disposable volumes.

## Limits

No production migration, broad version/platform rerun, performance claim or release build. Source/application-specific epoch restoration, price rounding and full `.load` compatibility remain outside these loader patch fixes. Existing sequence tests establish monotonic migration finalization, not an all-schema atomic repair command. Bytea content comparisons require an independent oracle until the product content verifier explicitly supports them.
