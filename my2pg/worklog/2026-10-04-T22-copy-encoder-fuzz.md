# 2026-10-04 — T22 bounded COPY encoder fuzz regression

- Status: bounded encoder regression implemented and verified; broader T22 fuzz/soak gates remain open
- Owner: coordinator (new `tests/copy_fuzz.rs` only)
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; shared tree is dirty
- Goal: exercise bounded COPY text escaping with deterministic generated rows and exact independent bytes, without changing runtime behavior or the database harness.

## Reference research before coding

Read `docs/reference-coding.md`, `docs/scope-and-compatibility.md`, `docs/agent-tasks.md`, and the COPY/property-fuzz requirements in `docs/testing-and-performance.md`. XERJ is v1.0.0-rc.80. The project index is pinned to source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.

| Query / corpus | Pinned revision and source/test evidence | Finding and chosen adaptation |
| --- | --- | --- |
| `project-my2pg`: `COPY text encoder escaping NULL tabs newline backslash byte roundtrip tests`; `max_row_bytes memory row workspace` | Current project snapshot. `src/convert/mod.rs:157-221, 176-219` escapes COPY text and premeasures/limits rows; public `encode_row_limited` and public plan/value models permit an integration test. `src/convert/tests.rs:39-65, 95-138` covers NULL/empty/literal `\\N`, control escapes, bytea, exact capacity and size boundaries. `tests/cases/t06_plan.rs:94` and `tests/cases/t10_values.rs:71` are production-adjacent consumers. | Keep application sources and the existing task-owned dirty unit-test file untouched. Add an automatically discovered integration target that checks generated valid UTF-8 fields, exact escaped bytes, NULL distinction and the admitted-size boundary via the public API. |
| `project-pgloader`: `copy text escaping null backslash newline retry batch`; `escape-copy-text format-row COPY text` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `clojure/src/pgloader/copy.clj:33-72`, `clojure/test/pgloader/copy_test.clj:1-47`, and `src/pg-copy/copy-format.lisp:1-115`. Clojure escapes backslash, tab, newline, carriage return, backspace and form feed; tests cover NULL, short/extra rows and cast values. Lisp separates already-escaped from regular text and emits separators/NULL itself. | Adapt only the COPY TEXT escaping and NULL/row framing cases. Do not copy legacy cast dispatch, Clojure's missing-column padding or extra-column truncation: my2pg requires exact resolved row arity. Do not modify either reference checkout. |
| `ref-rust-postgres`: `copy text format escaping null tab backslash encoding` | Rust Postgres pin `1084ca8f5b5302e161892f2fa40abf71b4060c10`; results were `postgres-protocol/src/escape/mod.rs` and binary COPY writer/tests. | Reject these as implementation references: SQL literal escaping and binary COPY are different formats. PostgreSQL remains a target and my2pg's behavior is validated independently against the COPY TEXT contract. |

RTK 0.49.0 is used to inspect/search files and run checks. Applied the Ponytail skill: no new dependency or production abstraction; add only the smallest independent, deterministic integration test needed by T22.

## Acceptance design

Use a fixed-seed standard-library generator to form bounded UTF-8 strings from ordinary ASCII, multibyte characters, COPY metacharacters, and supported control characters. Independently construct expected PostgreSQL COPY TEXT bytes from those strings, including field delimiters, row terminator, and SQL NULL's `\\N` representation. For each generated row, require exact bytes at the exact maximum, rejection at one byte below, and output capacity equal to the encoded length. This is bounded escape/framing regression coverage, not an external property-testing campaign or database round-trip proof.

## Implementation and verification

Added `tests/copy_fuzz.rs`, an automatically discovered integration target using only the public encoder and plan/value types. Its fixed-seed corpus has 512 rows, each assembled from at most 47 bounded tokens and kept under 1 KiB after escaping. Coverage includes UTF-8, empty fields, SQL NULL versus literal `\\N`, all COPY TEXT control escapes including vertical tab, field separators, row terminators, exact allocation length, exact admitted size, and one-byte-over rejection. The expected bytes are assembled in test code without calling the production encoder. No runtime source, existing dirty unit-test file, dependency, or reference checkout changed.

- `rtk run './bin/cargo test --offline --locked --test copy_fuzz'` — 1 passed.
- `rtk run './bin/cargo fmt --all -- --check'` — passed.
- `rtk run './bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings'` — passed.
- `rtk run './bin/cargo test --offline --locked --all-targets --all-features --quiet -- --test-threads=1'` — final integrated run: 163 passed, 157 ignored, zero failed; includes this target and the parser targets added in the same T22 increment.
- `rtk run 'git diff --check -- tests/copy_fuzz.rs worklog/2026-10-04-T22-copy-encoder-fuzz.md docs/checklists.md'` — passed.
- `python3 bin/reference-index.py project-my2pg` — passed; refreshed project index contains 429 files and 1,515 passages, with 14 recorded exclusions.

This closes bounded COPY row escaping/framing regression coverage. It is not a PostgreSQL round-trip proof and does not cover importer parser fuzzing, soak/RSS trends, native MySQL 5.7 AMD64 runtime, hosted CI, or release readiness. T22 remains open.
