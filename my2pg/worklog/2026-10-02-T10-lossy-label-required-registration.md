# 2026-10-02 — T10 lossy ENUM/SET label guard registration

Status: complete for mandatory registration and refreshed MySQL 8.0/PostgreSQL 16–18 native gates. The planner guard and native test are existing behavior and were not changed; lossless recovery remains unsupported and T10 remains open overall.

## Reference research before edit

- Read `docs/reference-coding.md`, the T10 task row in `docs/agent-tasks.md`, ENUM/SET policy in `docs/scope-and-compatibility.md`, and current checklist/T22 mandatory-case policy.
- XERJ project query: `project-my2pg "LOSSY_LABEL_METADATA supplementary ENUM SET required native acceptance"`, index revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It located `src/plan/mod.rs:630-660`, the native `t10_enum_metadata` case, and the independent source investigation worklog.
- XERJ pinned pgloader query: `project-pgloader "ordered ENUM SET labels mysql column type metadata"`, pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. `clojure/src/pgloader/ddl/common.clj:541-565` parses full ordered labels from `COLUMN_TYPE`; that path assumes the metadata string is faithful. MySQL's server-side substitution evidence in `worklog/2026-10-01-T10-enum-label-metadata.md` disproves that assumption for supplementary characters, so no pgloader parsing behavior is adapted.
- Read originals: `src/plan/mod.rs:630-660`, `tests/cases/t10_enum_metadata.rs:1-186`, its `tests/integration.rs` registration, `tests/support/harness.py:20-48`, and `tests/support/test_harness.py:13-40`. The native test proves supplementary ENUM/SET labels become indistinguishable from literal `?`, demonstrates loss across result charsets and SHOW CREATE, verifies the native value/ordinal discrepancy, and requires `LOSSY_LABEL_METADATA` before target schema creation. The case already passed in the recent MySQL 8.0.46 → PostgreSQL 16.15/17.11/18.6 full logs, but its ID is not mandatory.
- RTK wrapped reads and checks. Ponytail full guidance is active; minimal change is to add one cross-platform required ID and one expected-membership assertion, with no new logic or tests of the native case itself.

## Chosen adaptation and proof

Promote `t10_enum_metadata::supplementary_enum_set_labels_are_indistinguishable_from_question_marks_in_catalogs` to the generic required set. It uses only MySQL/PostgreSQL fixture services and a tested server-supported charset matrix; no host-specific mechanism applies. Run the seven support-harness tests and fresh full serial MySQL 8.0.46 → PostgreSQL 16.15, 17.11, and 18.6 lanes. Each result must include the exact case once as `ok`, without unmet IDs. This gate protects the explicit fail-closed boundary; it does not claim lossless recovery of supplementary labels.

## Native lane acceptance

- `python3 tests/support/test_harness.py`: all seven harness tests pass, including generic Linux/macOS membership and fail-closed missing/ignored/failed/duplicate result checks.
- Fresh serial MySQL 8.0.46 → PostgreSQL 16.15, 17.11, and 18.6 runs each passed 149 cases and all 20 Darwin-required IDs exactly once. Both this T10 guard and the T11 exhaustion case are recorded once as `ok`; all runs have empty `unmet_required_case_ids` and `connections.json` state `stopped`.
- PG16 artifact `target/integration/my2pg-mysql80-pg16-88081e06d907/`, test log SHA-256 `8032c42b6698b72db3b2a2fa57de0121e26a98c5f64446ff3fb5eb9b6a2b0b05`; PG17 artifact `target/integration/my2pg-mysql80-pg17-a729a31bfaae/`, log SHA-256 `bcd6be4c8710d2e1ffdc97ddedcbfb35c150dcade5088610dbd08ecfaa79da66`; PG18 artifact `target/integration/my2pg-mysql80-pg18-dc30c9dcb070/`, log SHA-256 `503f13e7943842a4cc0335d74b1810e6c3b6cf2ade07409e7e26fea1530f1a8e`. All use Cargo.lock SHA-256 `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23` and test binary SHA-256 `0b527485d354a391ddee85501d5a6ff02111044e9cd5bd211c14e5623959cc74`.
- This registration protects the explicit refusal against corrupted source label metadata; it does not implement lossless supplementary-label recovery or close T10.
