# 2026-10-02 — T17 — explicit ON UPDATE omission behavior

Status: native behavior case implemented and registered; focused compile/Clippy passed. Awaiting serial native execution before acceptance.

## Reference research before test code

- Read `docs/reference-coding.md`, T17 in `docs/agent-tasks.md`, the generated/ON UPDATE contract in `docs/scope-and-compatibility.md:80-96`, and `[[overrides]]` in `docs/config-and-cli.md:25-33`.
- XERJ query: `project-my2pg "ON UPDATE CURRENT_TIMESTAMP generated expression omission override policy" -k 8 --full 100`; indexed source revision label `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. The search returned the T17 blocking branch in `src/plan/mod.rs:846-861` and override/collation behavior in `src/plan/tests.rs:541-580`; I read the original planner and adjacent policy tests.
- XERJ query: `project-my2pg "ObjectOverride omit on_update field semantic diagnostic target_expression" -k 8 --full 100`; same index label. It located the `ObjectOverride` model in `src/config/mod.rs:205-212`, planner `omitted` at `src/plan/mod.rs:76-80`, and current test coverage. Read those sources and the plan construction around `src/plan/mod.rs:564,820-861`.
- Pinned pgloader query: `project-pgloader "ON UPDATE CURRENT_TIMESTAMP timestamp MySQL PostgreSQL create column default tests" -k 8 --full 100`; pin `231ab86778ca5ffd7de40878714760c8b4860cdf`. Read `clojure/src/pgloader/ddl/common.clj:603-630` and adjacent `clojure/test/pgloader/ddl_test.clj:385-405`. pgloader generates a shared function and BEFORE UPDATE trigger that assigns `now()` to every selected column. That is design evidence, but it is not copied: the existing Rust contract requires exact emulation or an explicit omission, and this trigger does not distinguish an explicit same-value assignment from automatic update semantics.
- MySQL native schema behavior will be asserted directly with `mysql::inspect` and the CLI, following T10/T11 acceptance cases. No cloned code or fixture is adapted.
- RTK wrappers were used for searches/reads. Ponytail guidance/executable remains unavailable in the checked roots; no execution is claimed.

## Coordinator review note before tightening the oracle

- Re-read the registered native case and its source/target timestamp reads. The existing test asserted only that the MySQL timestamp was nonempty, so it did not independently prove that the copied PostgreSQL value matched it.
- The fixture sets the MySQL session timezone to `+00:00`; compare the copied value's leading timestamp text with the source's `CAST(... AS CHAR)` value, allowing PostgreSQL to append its timezone suffix. This is a minimal assertion-only correction.
- Read and applied `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`: reuse the existing native case and standard string assertion, add no helper or dependency. RTK is installed as the `rtk` command and all shell reads/checks use `rtk run`.
- The T11 truncate-reset case is now registered in `tests/integration.rs`; its owner reports focused no-run compilation, exact discovery, and strict Clippy passed. It awaits the serial native lane.

## Implementation and compile evidence

- Added and registered `tests/cases/t17_on_update_override.rs` as `t17_on_update_override`. It exercises inspector metadata, rejection before target schema creation without policy, successful copy with the exact omission override, preservation of the UTC source timestamp and insertion default, and absence of an implicit update trigger/behavior.
- Tightened the copy oracle to compare the MySQL UTC timestamp string against the PostgreSQL timestamp prefix. Qualified the `to_regclass` lookup with safely quoted schema/table identifiers so the mixed-case table name is resolved as created.
- Checks passed after these changes: `cd my2pg && bin/cargo fmt --all`; `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test integration t17_on_update_override --no-run`; `bin/cargo clippy --offline --locked --features artifact-worker-tests,native-import-tests --test integration -- -D warnings`.
- At initial implementation handoff, no native fixture execution had occurred; acceptance was contingent on a passing owned TLS lane and required-case registration. Both conditions are now satisfied in the section below.
- Coordinator registration decision before harness edit: explicit ON UPDATE omission is a planned T17 compatibility contract, so add its exact test ID to the mandatory native-case registry. The harness enforces exactly one pass per required ID; compile-only results remain insufficient for acceptance.
- Coordinator verified Ponytail guidance at `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md` and applied its minimum-change rule; the native behavior case adds no helper or dependency. RTK commands are run through `rtk run`.

## Native acceptance

The full owned serial MySQL 8.0.46/PostgreSQL 16.15 lane passed **146 invoked tests, zero failures** on Darwin/ARM64. This T17 behavior case ran exactly once and passed: MySQL `ON UPDATE` metadata is detected; absence of an explicit omission fails with `ON_UPDATE_POLICY` before destination schema creation; the exact omission override migrates the row and insertion default; the UTC source timestamp is preserved; and an unrelated update creates no automatic timestamp change or trigger. Evidence is shared with the T11 run at `target/integration/my2pg-mysql80-pg16-ac14bec5040c/rust-tests.json` (SHA256 `346406f98ae3ea7f76fcec31f69810b3325da457e641f0a638e98f0376e9c50f`) and `.log` (SHA256 `38e241bdeb57feb606c06bff38abf1c4f96cbf48a673e2ab0ec0441259edb603`); owned fixtures stopped. This closes only the explicit ON UPDATE omission behavior. Generated/check/index expression coverage and spatial policy remain open under T17.

## Chosen behavioral proof

Add an ignored, fresh-database CLI test for a MySQL `TIMESTAMP(6) DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6)` column. Prove the source inspector sees the update attribute. With no override, require an `ON_UPDATE_POLICY` failure before creating the destination schema. Then add the exact `.on_update` omission override and rerun: require successful schema/data migration, preservation of the copied source timestamp and `CURRENT_TIMESTAMP(6)` insertion default, and no target update trigger or automatic timestamp change after updating another column. This proves explicit omission is honored while avoiding a claim of transparent emulation. The native test will remain outside the mandatory-case list until its focused lane passes; coordinator owns its root registration and later required-case decision.

## Coordinator precision correction

The later full serial lane exposed that PostgreSQL `updated_at::text` trims a trailing zero in fractional seconds, while MySQL's `CAST(... AS CHAR)` prints six digits. The test oracle now uses `to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US')` for both pre-update and post-update values. The exact T17 case passed in the final MySQL 8.0.46/PostgreSQL 16.15 full run: 147 invoked cases, zero failures, all 16 required native IDs exactly once. Final lane details and artifact hashes are recorded in [the generated-column T17 worklog](2026-10-02-T17-generated-column-policy.md).
