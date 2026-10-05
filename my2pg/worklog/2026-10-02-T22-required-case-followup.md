# T22 — mandatory discovery for identity, snapshot and combined storage cases

Status: discovery follow-up only. Lease: harness.py, matrix.py and their database-free unit tests plus this new log. No pins, registrations, Rust cases, task board, indexes or database lifecycle changes.

## Reference research before code

RTK/Ponytail guidance and reference-coding workflow applied. Project HEAD/index label `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; current registered Rust originals read directly because the index snapshot can precede new cases. Fresh XERJ `project-my2pg / REQUIRED_CASES MACOS_CASES matrix discovery` returned matrix137–189 and preflight worklog: aggregate discoveries cannot mask missing named acceptance, and Darwin cases must not be mandatory on Linux. Read originals harness20–29/353–372, matrix17–21/137–189, complete adjacent Python tests, integration registrations and exact cfg/ignore/function declarations in t11_identity_edges84–107/173–203, t13_snapshot_consistency18–42 and t12_uncertain_storage2/78–79. The two identity cases and snapshot case are generic; combined actual COMMIT/ENOSPC case is file-level macOS-only.

XERJ `ref-rust-postgres / cfg feature runtime tests` at pin `1084ca8f5b5302e161892f2fa40abf71b4060c10` returned postgres-openssl/src/lib.rs1; read1–45 and neighboring tokio-postgres/tests/test/main.rs20–25. Feature-conditional examples/runtime modules can disappear from successful builds. Reuse explicit compilation/list discovery already implemented; no copied code or expanded dependencies.

Chosen adaptation before code: append the two exact T11 and one T13 IDs to the existing generic REQUIRED_CASES tuple; define a small macOS-only tuple for the combined T12 ID and one harness required_cases(system) helper used by runtime validation. Matrix reuses that macOS tuple with its three existing Darwin fault IDs, so generic/host-specific lists cannot diverge. Actual runtime results still require exactly one ok, not merely listing or aggregate passes. Add independent expected IDs/platform tests and malformed/missing/ignored/duplicate result regressions. Run parser/unit/compile/help checks and actual offline matrix compile/list-only command; preserve its four missing-pin errors. No database cases run.

## Implemented and frozen checks

Changed only harness.py, matrix.py, test_harness.py, test_matrix.py and this log. Generic harness acceptance now requires10case IDs; Darwin additionally requires the combined T12 case (11total). Linux excludes that file-level macOS case. Runtime evidence records the actual host and its applicable required results. Matrix inherits the same single combined-case constant, retaining its existing three macOS fault cases and separate NLS target: Darwin requires15 IDs (generic10+NLS1+macOS4); Linux requires11 (generic10+NLS1). Existing pin validation/missing-pin behavior is unchanged. A listed case is never counted as a passed runtime case.

RTK-wrapped checks:

- `python3 -m unittest discover -s tests/support -p 'test_*.py'`: all19 passed (7harness/12matrix). New tests independently assert the three exact generic IDs and exact macOS ID/platform membership. Mocked whole-harness runs on Linux/Darwin cover all-ok, missing, ignored, failed and duplicate required outcomes despite an unrelated aggregate100passes; verify host/required-result evidence, fail-closed unmet IDs and exactly-once scoped cleanup. Existing matrix NLS/macOS and pin failures remain green; explicit missing combined case fails on Darwin.
- `python3 -m doctest tests/support/harness.py`: all15 existing parser/artifact-selector examples passed.
- `env PYTHONPYCACHEPREFIX=/private/tmp/my2pg-T22-followup-pycache python3 -m py_compile` on the four owned Python files: passed.
- `bin/cargo fmt --all -- --check`: passed, no Rust edits.
- `bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings`: passed0.35s, briefly waited for the discovery compilation lock.
- `git diff --check`: passed.
- Actual `python3 tests/support/matrix.py --check --all > /private/tmp/my2pg-T22-required-case-followup.json`: **expected exit1**, exactly missing mysql57/mysql80/pg17/pg18 pins. Offline compilation/list-only discovery succeeded:135ignored native cases across Cargo-reported artifacts; all15applicable required IDs found exactly once. All9lanes still present; zero database tests executed/support_certified=false. No new pin, CI or support claim.

Lease frozen for coordinator review; no databases, disk images or containers were started or altered, and no index query/rebuild occurred after implementation. Root owns coherent native execution and index refresh. T22 version/platform runtime acceptance stays open.

## Coordinator follow-up — T11 exhaustion gate

After this original 18-Darwin-ID discovery increment, the coordinator promoted `t11_identity_exhaustion::native_runner_refuses_source_auto_increment_beyond_smallint_sequence_max` to the generic required set. The seven support-harness tests still pass and explicitly assert this ID on Linux and Darwin. Fresh MySQL 8.0.46 → PostgreSQL 16.15/17.11/18.6 full serial lanes each passed 149 cases; each `rust-tests.json` records 19 Darwin-required IDs, the exhaustion case exactly once as `ok`, and no unmet IDs. See [registration and lane results](2026-10-02-T11-exhaustion-required-registration.md#native-lane-acceptance). This is a current ARM64 host result; MySQL 5.7 and other platform/runtime, NLS, physical-storage, fuzz/soak and CI gates remain open.

## Coordinator follow-up — T10 corrupted-label refusal

The coordinator also made `t10_enum_metadata::supplementary_enum_set_labels_are_indistinguishable_from_question_marks_in_catalogs` mandatory on all hosts. Seven support-harness tests pass, and fresh MySQL 8.0.46 → PostgreSQL 16.15/17.11/18.6 full serial lanes each passed 149 cases with 20 Darwin-required IDs exactly once. Both the lossy-label refusal and T11 exhaustion IDs are `ok`; no required results are unmet. This preserves fail-closed behavior against MySQL's corrupted ENUM/SET label metadata and does not claim label recovery. Full artifacts and remaining T10 scope are in [T10 required-case evidence](2026-10-02-T10-lossy-label-required-registration.md#native-lane-acceptance).
