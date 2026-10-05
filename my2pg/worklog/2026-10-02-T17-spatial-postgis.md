# 2026-10-02 — T17 — spatial values and optional PostGIS

- Status: complete for the reviewed MySQL spatial subset
- Agent/role: coordinator
- Task and specification links: [T17](../docs/agent-tasks.md#task-board), [scope](../docs/scope-and-compatibility.md#feature-disposition), [checklist](../docs/checklists.md#m3-compatibility)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Base revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`
- File lease: coordinator owns source planner/reader/converter/verifier integration and native harness registration for this increment.

## Reference research before coding

| Query and prefix | Pinned revision and inspected files | Finding | Adaptation and acceptance proof |
| --- | --- | --- | --- |
| `spatial geometry SRID PostGIS source type plan verifier`; `project-my2pg` | Working snapshot labeled `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `docs/scope-and-compatibility.md:32,44,74`, `src/plan/mod.rs:133-205`, `src/mysql/mod.rs:546-581`, `src/convert/mod.rs:500-520` | Contract requires optional PostGIS, per-value SRID preservation, extension detection without installation. Geometry is present in `ValueKind` but has no MySQL type mapping, the reader selects the raw geometry value, and conversion explicitly errors that fidelity is a later implementation. Target catalogs already collect installed extension names. | Map MySQL spatial types to generic PostGIS `geometry`, fail planning before DDL if `postgis` is absent, project source values to EWKT including each row's SRID, and preserve NULL. Test target absence/no auto-install and full native spatial migration with empty geometry, multiple shapes/dimensions and SRIDs. |
| `MySQL geometry spatial SRID PostgreSQL PostGIS`; `project-pgloader` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; `clojure/src/pgloader/source/mysql.clj:91-98,252-268`, `clojure/src/pgloader/transforms.clj:124-158`, `clojure/src/pgloader/ddl/common.clj:116-125`, source and transform tests `clojure/test/pgloader/source/mysql_test.clj:18-32`, `clojure/test/pgloader/transforms_test.clj:67-72`, `clojure/test/pgloader/ddl_test.clj:68-75` | pgloader rewrites spatial source columns with `ST_AsText`; DDL chooses PostGIS `geometry`. Its transform tests cover one 2D LINESTRING conversion, and a separate POINT parser assumes the MySQL internal 4-byte SRID prefix but emits a plain PostgreSQL point. The source SQL test verifies only the `ST_AsText` expression. This does not prove SRID retention, broad geometry types, or PostGIS extension refusal. | Reuse the read-only source-server projection and optional target type. Reject pgloader's SRID-dropping/plain-point conversion. Emit EWKT by joining `ST_SRID` with `ST_AsText`, use per-row SRIDs, and rely on PostgreSQL/PostGIS geometry input rather than parsing or translating WKB on the client. Independent native queries must compare `ST_AsEWKT` and `ST_GeometryType` values after migration. |

Ponytail guidance and RTK usage were already established for this workspace. Before implementation, inspect the exact current target extension preflight, COPY/verification projections and integration image-pinning rules. The project XERJ source index and pinned pgloader corpus were searched; no external source code will be copied.

## Test plan

- Unit tests cover type mapping, null/empty EWKT conversion and geometry shapes without exposing input data in errors.
- Database-free/native case discovery requires the missing-extension case on MySQL 8.0+/PostgreSQL 16 and the positive case on the explicit `pg16-postgis` lane.
- Missing PostGIS fails during planning before destination schema creation and never runs `CREATE EXTENSION`.
- The PostGIS-enabled target migrates POINT, LINESTRING, MULTIPOLYGON, MULTIPOINT, geometry collections, the supported empty GeometryCollection and NULL. Independent target SQL checks type, 2D dimensions, SRID, and `ST_Equals` against source EWKT.
- MySQL's documented spatial storage format supports X/Y and only empty GeometryCollection; Z/M input was rejected by the source server while discovering the native fixture contract. T17 therefore preserves the source's accepted 2D model, and does not claim Z/M support.
- Generic T18 row hashing does not canonicalize geometry. T17's spatial native case performs its own independent PostGIS shape/SRID oracle; generic content verification for geometry remains a distinct T18/T22 follow-up rather than a claim of this task.

## Native results

- `python3 tests/support/harness.py mysql80 pg16-postgis`: passed 152 tests; all 22 required IDs occurred exactly once, including the positive PostGIS spatial case. Environment: MySQL 8.0.46 and PostgreSQL 16.11 with PostGIS 3.4.4. Artifact: `target/integration/my2pg-mysql80-pg16-postgis-92ad46eb5e1c/rust-tests.json`; captured harness log SHA-256 `86f10308a75a82d0e494df947f71fa291aad0fa03cd333833b8d5342d8e4654e`.
- `python3 tests/support/harness.py mysql80 pg16`: passed 152 tests; all 22 required IDs occurred exactly once, including the missing-PostGIS refusal case. Environment: MySQL 8.0.46 and PostgreSQL 16.15 without PostGIS. Artifact: `target/integration/my2pg-mysql80-pg16-f67efb842cf0/rust-tests.json`; captured harness log SHA-256 `4f26831e85ce3c6e2a277adfa4a1f7bb8de0e2c954c47bd3b5ea4f521329194d`.
- `bin/cargo test --offline --locked --all-targets --all-features -- --test-threads=1`: passed on the privileged rerun after one unrelated descriptor-count test transient in the initial run; the standalone rerun of that test passed. Full result: 155 integration tests, 17 passed, 138 ignored; all other applicable targets passed.
- `bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings`, `bin/cargo fmt --all -- --check`, and `python3 tests/support/test_harness.py`: passed (7 Python tests).
- `git diff --check`: pending final documentation integration check.

## Changed behavior and limitations

- MySQL spatial columns are planned as generic PostGIS `geometry`; planning emits `POSTGIS_REQUIRED` before target schema creation if target catalog does not list the preinstalled extension.
- The MySQL reader projects `CONCAT('SRID=', ST_SRID(column), ';', ST_AsText(column))`, retaining row-specific SRID and SQL NULL. The encoder accepts only strict `SRID=<digits>;<WKT>` and passes it to PostGIS through COPY.
- Added an immutable arm64-only test image pin for PostgreSQL 16 + PostGIS 3.4.4; this pin is test infrastructure, not a runtime app dependency. The image reports PostgreSQL 16.11. The plain PostgreSQL lane confirms the application never creates the extension.
- No code was copied from pgloader; its text conversion path was used as a contrast, and its old POINT path would lose SRID.
- This evidence covers MySQL 8.0.46/PostgreSQL 16 only. Other source/target release lanes and platform variants remain T22 work.

## Handoff

Implementation and both required native paths are complete. T17 can close for its planned behavior set; T22 retains full matrix/platform certification, and T18 retains generic content canonicalization for geometry.
