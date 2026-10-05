# 2026-10-04 — T22 bounded legacy-import parser fuzz regression

- Status: bounded importer-parser regression implemented and verified; T22 platform/soak gates remain open
- Owner: coordinator (temporary lease: new `tests/importer_fuzz.rs` only)
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; shared tree is dirty
- Goal: exercise the strict MySQL `.load` tokenizer/parser with deterministic bounded inputs, without changing import behavior or writing files from the parser test.

## Reference research before coding

Read `docs/reference-coding.md`, `docs/scope-and-compatibility.md`, `docs/agent-tasks.md`, the `IMPORT` and fuzz requirements in `docs/testing-and-performance.md`, the current parser, adjacent tests, and the T16 importer worklog. XERJ is v1.0.0-rc.80; the project corpus represents source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.

| Query / corpus | Pinned revision and source/test evidence | Finding and chosen adaptation |
| --- | --- | --- |
| `project-my2pg`: `legacy importer parser bounded .load escaping tests`; `malformed and adversarial input bounded importer` | Current project snapshot. `src/config/import.rs:111-325` bounds text to 1 MiB, NULs, nested comments to depth 16 and tokens to 8,192; `parse` is public. `tests/importer.rs:1-34, 380-535` has independent grammar/error-span, unsafe-form, nesting and input-bound tests; `worklog/2026-10-01-T16-load-import.md` records the strict one-way MySQL subset. | Add a separate automatically discovered integration target using the public `parse` API and deterministic bounded inputs. Assert termination through normal `Result` return and valid source spans on errors; no file publishing, environment reads, network or parser behavior changes. |
| `project-pgloader`: `parse-load-file parser errors directives`; `LOAD DATABASE parser comments quotes malformed` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `clojure/src/pgloader/load_file/parser.clj:1-86`, `clojure/src/pgloader/load_file/grammar.clj` database and clause productions, and `clojure/test/pgloader/load_file/parser_test.clj:1-125`. The upstream parser composes a broad multi-source grammar, strips comments/handles templates, then transforms an AST; adjacent tests cover MySQL commands, clauses, comments and malformed syntax. | Use only the source as grammar/test-case context. Reject its template expansion, generic CSV/multisource grammar and AST/runtime semantics; fuzz my2pg's smaller closed MySQL-only grammar and preserve its error-span contract. No upstream code or fixtures are copied. |

RTK 0.49.0 is used to query/read files and run checks; the repository documents RTK command use but supplies no standalone RTK skill. Applied Ponytail: use the existing parser API, standard-library deterministic generation, no dependency, and one new test target.

## Acceptance design

Run a fixed bounded corpus split between arbitrary lossy-UTF-8 byte inputs and structured mutations of a finite valid MySQL `LOAD DATABASE` command. Cap every source string well below the parser's 1 MiB ceiling and bound the case count. Success and ordinary import errors are both valid results. For each error, assert its start/end are ordered and remain within the input's UTF-8 byte length. This is crash/span regression coverage, not proof that every accepted clause is semantically correct or that file publication is safe; existing importer tests own those behaviors.

## Implementation and verification

Added `tests/importer_fuzz.rs` as an automatically discovered integration target using `config::import::parse`. Its fixed-seed corpus runs 256 arbitrary byte-derived lossy-UTF-8 strings and 256 grammar-shaped mutations covering limits, regex filters, names, target types, guards and long values. Every input is at most 2 KiB; the test performs no file publication, environment lookup or database/network work. Successful parses and ordinary errors are valid; every error span is checked against the original string. No parser behavior, dependency, reference checkout or existing dirty importer test file changed.

- `rtk run './bin/cargo test --offline --locked --test importer_fuzz'` — 1 passed.
- `rtk run './bin/cargo fmt --all -- --check'` — passed.
- `rtk run './bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings'` — passed.
- `rtk run './bin/cargo test --offline --locked --all-targets --all-features --quiet -- --test-threads=1'` — final integrated run: 163 passed, 157 ignored, zero failed.
- `rtk run 'git diff --check -- tests/importer_fuzz.rs worklog/2026-10-04-T22-importer-parser-fuzz.md docs/checklists.md'` — passed.
- `rtk run 'python3 bin/reference-index.py project-my2pg'` — passed; final project index contains 431 files and 1,518 passages, with 14 recorded exclusions.

This closes bounded crash/span regression coverage for the importer parser. It does not close semantic matrix coverage, native MySQL 5.7 AMD64, hosted CI, other platform runs, soak/RSS trends, or release readiness. T22 remains open.
