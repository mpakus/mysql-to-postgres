# 2026-10-04 — T22 bounded configuration-parser fuzz regression

- Status: bounded parser fuzz regression implemented and verified; broader T22 fuzz/soak gates remain open
- Owner: coordinator (temporary lease: new `tests/config_fuzz.rs` only)
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; shared tree is dirty
- Goal: add a deterministic, dependency-free, bounded fuzz regression for the executable TOML configuration parser without changing runtime behavior or the database harness.

## Reference research before coding

Read `docs/testing-and-performance.md` (bounded parser fuzzing requirement), `docs/reference-coding.md`, T22 ownership in `docs/agent-tasks.md`, RTK guidance, and Ponytail. XERJ local version is `v1.0.0-rc.80`; the project index revision is `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.

| Query / corpus | Pinned revision and source/test evidence | Finding and chosen adaptation |
| --- | --- | --- |
| `project-my2pg`: `bounded fuzz TOML config parser no panic robustness`; `config parser toml parse_config validation malformed tests` | Current indexed project snapshot. Direct source: `src/config/validation.rs:91-128` deserializes TOML, enforces the explicit `data_only` reset choice, validates fields, resolves paths and checks referenced files. Adjacent tests in `src/config/validation.rs:1008+` use `MINIMAL`, existing malformed-field cases, and `config(extra)` to exercise the parser. | No existing fuzz target was indexed or found by file search. Reuse the public `config::parse` contract and TOML dependency; add one isolated test target rather than alter parser behavior or existing task-owned source files. |
| `ref-dmt-rs`: `TOML configuration parsing validation error tests`; `serde TOML config deserialize parse config error tests invalid TOML` | `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; `crates/dmt-rs/src/config/mod.rs:19-45,164-183`, `crates/dmt-rs-cli/tests/cli_tests.rs:123-165`. | The reference parses YAML/JSON with Serde, validates after deserialization, and tests malformed syntax and missing fields. It has no bounded fuzz harness and uses different formats; do not copy its parser or error contract. |

## Acceptance design

Use a small standard-library deterministic generator (no new dependency) for arbitrary UTF-8-lossy inputs plus valid TOML skeletons with bounded mutations of numeric, boolean, enum, and string-like fields. Cap each input at 2 KiB and the corpus count so total parser work is bounded. Each case calls the public parser; any panic fails the test, while `Ok` and ordinary `ConfigError` are both valid outcomes. Keep the generated seed and case index reproducible. This is crash/robustness coverage, not a semantic oracle or real database test.

## Implementation and verification

Added `tests/config_fuzz.rs` as an automatically discovered, database-free test target. It runs 256 deterministic byte-derived inputs and 256 structured TOML mutations over numeric, boolean, enum and invalid-type values. Every parser input is capped at 2 KiB; generated seed `0x4d5932706746555a` and case order reproduce the corpus. The test asserts the parser returns normally; expected parse/validation errors are valid outcomes. No application behavior, dependency or shared config source changed.

- `rtk test ./bin/cargo test --offline --locked --test config_fuzz` — 1 passed; 0.02 seconds.
- `rtk test ./bin/cargo fmt --all -- --check` — passed.
- `rtk test ./bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings` — passed.
- `rtk test ./bin/cargo test --offline --locked --all-targets --all-features -- --test-threads=1` — 179 passed, 157 existing ignored, zero failed. The new target was discovered and passed.
- `rtk test python3 -m unittest discover -s tests/support -p 'test_*.py' -v` — 35 passed.
- `rtk test python3 tests/support/fixture_manifest.py` — 65 fixture case IDs and source/pin hashes verified.
- `rtk test python3 -m unittest tests/performance/test_runner.py -v` — 37 passed.
- `rtk test env CARGO_ABOUT=<pinned 0.8.4 binary> ... ./bin/check-third-party-notices.sh --check` — passed with only the existing Cargo package-metadata warning.
- `rtk run 'git diff --check -- tests/config_fuzz.rs worklog/2026-10-04-T22-config-parser-fuzz.md docs/checklists.md'` — passed.

This closes one bounded TOML parser crash-fuzz regression. It does not cover the COPY row encoder or legacy importer parser, does not establish a soak/RSS trend, and does not close the native AMD64 MySQL 5.7 or hosted CI gates. T22 remains in progress.
