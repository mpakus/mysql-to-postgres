# 2026-10-02 — coordinator — T22 checklist reconciliation

Status: documentation reconciled to retained matrix evidence; T22 remains open.

## Evidence reviewed

- Read the release checklist's T22 summary and the full [current required-case lane refresh report](2026-10-02-T22-current-required-lane-refreshes.md).
- The report records successful, stopped fixture runs for all six pinned MySQL 8.0/8.4 → PostgreSQL 16/17/18 base cells and the separate MySQL 8.0 → PostgreSQL 16 PostGIS-positive cell. Each run has its own artifact hashes and exact applicable required-case count.
- The two SingleSnapshot metadata-lock tests are applicable to the reviewed MySQL 8.4 → PostgreSQL 16 lane, which passed 156 cases and all 26 required IDs at the reviewed source snapshot. Other lanes retain their documented applicable counts.
- The checklist incorrectly said five base cells and the PostGIS-positive lane still needed the SingleSnapshot cases. Updated that statement to match the recorded lane applicability and results; no test evidence or task status was changed.

## Remaining T22 gates

MySQL 5.7 lanes still lack a reviewed AMD64 runner and matching image pins. NLS on current source, physical-sync/fault breadth, broader platform coverage, fuzz/soak, and CI acceptance remain open. Existing lane results do not close those gates.

## Validation

- `rtk run 'git diff --check'` — passed.
- `rtk run 'python3 -m unittest tests.support.test_package'` — passed, 3 tests.
- `rtk run 'git diff --check'` — passed.
- Coordinator-owned `rtk run 'python3 bin/reference-index.py project-my2pg'` — passed; refreshed index contains 380 files, 1,328 passages and 11 exclusions.
- Representative XERJ query `all six current required lane refresh PG16 PostGIS SingleSnapshot` returned this worklog, the current lane-refresh report and the updated checklist at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
