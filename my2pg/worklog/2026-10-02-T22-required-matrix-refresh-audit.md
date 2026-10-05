# T22 — required-case impact on existing matrix evidence

Status: read-only audit. Only this worklog was added; no harness, pin, matrix, task-board, checklist, Docker, database, or shared XERJ state was changed. RTK wrapped repository reads. Working-tree HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` (the recorded integration reports also identify dirty working-tree builds).

## Current required-case contract

Read `tests/support/harness.py:21–71`, `tests/support/test_harness.py:20–54`, `tests/support/matrix.py:14–22,34–81`, `tests/compose/images.json:1–59`, and `tests/compose/compose.yml`. `REQUIRED_CASES` now has 20 IDs, including `t13_external_mdl::cancellation_kills_reader_waiting_on_external_mysql_metadata_lock` at line 34. Darwin adds the macOS storage case (line 44), making 21 base IDs; Linux has 20. MySQL 8.0 and 8.4 add two common IDs (lines 48–51). PostgreSQL 16 adds one branch-specific PostGIS case, either the no-PostGIS refusal or the positive PostGIS behavior (lines 52–57); PostgreSQL 17/18 add neither. Thus current Darwin required counts are 21 for MySQL 5.7, 23 for MySQL 8.x→17/18, and 24 for MySQL 8.x→16 (plain or PostGIS). Corresponding Linux counts are 20, 22, and 23. The `required_cases` selector defaults MySQL 8.x to PG16 only when the PostgreSQL lane is omitted; explicit PG17/18 selectors do not inherit the spatial case.

Current pins at `tests/compose/images.json:1–58` cover MySQL 8.0/8.4 and PostgreSQL 16/17/18, all `linux/arm64`; the `mysql57` entry is absent. `matrix.py:34–81` requires a pin for each source/target version and matching platforms. `worklog/2026-10-02-T22-mysql57-pin-diagnostic.md:20–24` records the available MySQL 5.7 image as AMD64-only, with no matching AMD64 PostgreSQL pin contract on this host. Therefore all three MySQL 5.7 cells remain unavailable for native execution under the current single-platform registry. The other eight base version cells have pin metadata, although pin presence alone is not runtime evidence.

## Runtime evidence that needs refresh

Every accepted full lane below predates registration of the external metadata-lock cancellation case, so its old success cannot satisfy the current required-ID contract. Rerun each accepted cell serially with the current harness and confirm the exact new T13 ID is `ok` once and `unmet_required_case_ids` is empty.

| Accepted cell | Latest relevant prior full-lane evidence | Prior required IDs | Current Darwin count | Refresh status |
| --- | --- | ---: | ---: | --- |
| MySQL 8.4 → PostgreSQL 16 | [T22 required-case refresh](2026-10-02-T22-mysql84-pg16-t13-refresh.md:9–15,30–34): 154 cases, 24 IDs, including the new T13 case | 24 | 24 | Refreshed; no repeat required for this registration |
| MySQL 8.4 → PostgreSQL 17 | [PG17 lane](2026-10-02-T22-postgres17-lane.md:42–48): 138 cases, 11 IDs | 11 | 23 | Rerun required |
| MySQL 8.4 → PostgreSQL 18 | [PG18 compatibility closeout](2026-10-02-T22-pg18-not-null-catalog-compatibility.md:44–49): accepted full lane after catalog fixes; retained artifact has 11 required result entries | 11 | 23 | Rerun required |
| MySQL 8.0 → PostgreSQL 16 | [T22 required-case follow-up](2026-10-02-T22-required-case-followup.md:33–35) records the post-T10/T11 full suite at 149 cases and 20 IDs. The separate [T17 spatial run](2026-10-02-T17-spatial-postgis.md:30–31) also has 22-ID full runs for plain and PostGIS PG16 variants. | 20 (22 in the spatial-specific artifact) | 24 | Rerun both plain PG16 and PostGIS PG16 evidence if retaining both claims |
| MySQL 8.0 → PostgreSQL 17 | [T22 expanded lane](2026-10-02-T22-mysql80-pg17-lane.md:22–26): 149 cases, 18 IDs; later common registration evidence in [required-case follow-up](2026-10-02-T22-required-case-followup.md:33–35) reports 149 cases, 20 IDs | 20 | 23 | Rerun required |
| MySQL 8.0 → PostgreSQL 18 | [T22 expanded lane](2026-10-02-T22-mysql80-pg18-lane.md:24–28): 149 cases, 18 IDs; later common registration evidence in [required-case follow-up](2026-10-02-T22-required-case-followup.md:33–35) reports 149 cases, 20 IDs | 20 | 23 | Rerun required |

The older MySQL 8.0→16 first full attempt in [T22 MySQL 8.0 lane](2026-10-02-T22-mysql80-lane.md:32–46) failed two version assertions and is not an accepted lane; it is superseded as runtime evidence by later successful 8.0→16 records. The PG17 and PG18 MySQL 8.0 expanded lane artifacts each record 18 IDs before subsequent common registration work; those later 20-ID results still predate the new T13 ID. The current expected counts above come from the live selector rather than the historical prose.

The fresh MySQL 8.4→16 run is explicitly recorded as the post-registration refresh in `worklog/2026-10-02-T22-mysql84-pg16-t13-refresh.md:9–15,30–34`: 154 passing native cases, 24 applicable Darwin IDs, no failures or unmet IDs, and the new T13 cancellation case `ok` exactly once. Its scoped fixture stopped. This is the only accepted full lane found that already incorporates this registration.

## Scope and limits

The exact nine base cells are enumerated by `tests/support/matrix.py:14–15,70–81`. Current base runtime evidence is fresh only for 8.4→16. Accepted 8.4→17/18 and 8.0→16/17/18 evidence must be refreshed against the 21/23/24-ID Darwin contract before it is described as satisfying current required-case acceptance. MySQL 5.7→16/17/18 remains unpinned/unavailable on this host pending a reviewed native AMD64 runner and matching PostgreSQL images or an architecture-selecting registry design. NLS and PostGIS-positive variants are additional selectors, not replacements for the nine base cells. No Docker or database startup was performed for this audit.
