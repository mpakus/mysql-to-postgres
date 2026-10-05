# 2026-10-02 — T22 — current required-case lane refreshes

Status: in progress. These runs replace older lane evidence that predates the generic T13 external-MDL required case. Each entry records one owned, stopped pair.

## MySQL 8.4.11 → PostgreSQL 17.11

- Host: Darwin/ARM64; pinned containers: `linux/arm64`.
- Command: `rtk run './tests/run-integration.sh mysql84 pg17'`.
- Result: exit 0; 154 native cases passed; zero failed IDs; all 23 current applicable required IDs passed exactly once. The new `t13_external_mdl::cancellation_kills_reader_waiting_on_external_mysql_metadata_lock` ID is `ok` once.
- Fixture project: `my2pg-mysql84-pg17-bfa81331d678`, versions MySQL 8.4.11/PostgreSQL 17.11; metadata says `stopped`; an exact Docker Compose project-label query returned no running container IDs.
- Artifacts: `target/integration/my2pg-mysql84-pg17-bfa81331d678/`.
- `rust-tests.json` SHA-256: `2bae1d12c6dd85b48619096ad54a3f1a700c93af55dedb6f6e50d66f0faac6ba`.
- Test log SHA-256: `a817bac8d3d6e97a47053bd0560414117a15927e7617a26396de2c3a4a059360`.
- Binary source revision in the harness evidence: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty; harness SHA-256 `abb5ea6cd66f57ea65f2504c2286a864095ec4ed4a33693e169c3da0a6a1da60`.

## MySQL 8.4.11 → PostgreSQL 18.6

- Host: Darwin/ARM64; pinned containers: `linux/arm64`.
- Command: `rtk run './tests/run-integration.sh mysql84 pg18'`.
- Result: exit 0; 154 native cases passed; zero failed IDs; all 23 current applicable required IDs passed exactly once. The new `t13_external_mdl::cancellation_kills_reader_waiting_on_external_mysql_metadata_lock` ID is `ok` once.
- Fixture project: `my2pg-mysql84-pg18-04f9841218c4`, versions MySQL 8.4.11/PostgreSQL 18.6; metadata says `stopped`; an exact Docker Compose project-label query returned no running container IDs.
- Artifacts: `target/integration/my2pg-mysql84-pg18-04f9841218c4/`.
- `rust-tests.json` SHA-256: `9382b4b2e8633926d65566103e057ac37f51c68d09b755d4a9d316965b6eba04`.
- Test log SHA-256: `018f9e92ef45b7e73f8287259d433fb98d44169390d415bd6161287b37095f96`.
- Binary source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty; harness SHA-256 `abb5ea6cd66f57ea65f2504c2286a864095ec4ed4a33693e169c3da0a6a1da60`.

## MySQL 8.0.46 → PostgreSQL 16.15 without PostGIS

- Host: Darwin/ARM64; pinned containers: `linux/arm64`.
- Command: `rtk run './tests/run-integration.sh mysql80 pg16'`.
- Result: exit 0; 154 native cases passed; zero failed IDs; all 24 current applicable required IDs passed exactly once. The external-MDL cancellation case and the required PostGIS-absent refusal are each `ok` once.
- Fixture project: `my2pg-mysql80-pg16-d6fab22283ce`, versions MySQL 8.0.46/PostgreSQL 16.15; metadata says `stopped`.
- Artifacts: `target/integration/my2pg-mysql80-pg16-d6fab22283ce/`.
- `rust-tests.json` SHA-256: `eec509672a47328746e2a17e35d45f4ae241245ab07d476e7dd1231d3f9255bf`.
- Test log SHA-256: `088b89aac5b6c76a319854aa528724b2be1ff2d250036be3b67eab26ba6e7ed4`.
- Binary source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty; harness SHA-256 `abb5ea6cd66f57ea65f2504c2286a864095ec4ed4a33693e169c3da0a6a1da60`.

## MySQL 8.0.46 → PostgreSQL 17.11

- Host: Darwin/ARM64; pinned containers: `linux/arm64`.
- Command: `rtk run './tests/run-integration.sh mysql80 pg17'`.
- Result: exit 0; 154 native cases passed; zero failed IDs; all 23 current applicable required IDs passed exactly once. The new T13 external-MDL cancellation ID is `ok` once.
- Fixture project: `my2pg-mysql80-pg17-82df1b0b961d`, versions MySQL 8.0.46/PostgreSQL 17.11; metadata says `stopped`.
- Artifacts: `target/integration/my2pg-mysql80-pg17-82df1b0b961d/`.
- `rust-tests.json` SHA-256: `5a0df33d485c53625c185fd29581b54aa990c48d0b1b826c2c4a8d30d85e7ea4`.
- Test log SHA-256: `7aaf9ac47343fa605fc3c0fa7b66a4c57734f0cc553d499a2a0792b124aa3288`.
- Binary source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty; harness SHA-256 `abb5ea6cd66f57ea65f2504c2286a864095ec4ed4a33693e169c3da0a6a1da60`.

## MySQL 8.0.46 → PostgreSQL 18.6

- Host: Darwin/ARM64; pinned containers: `linux/arm64`.
- Command: `rtk run './tests/run-integration.sh mysql80 pg18'`.
- Result: exit 0; 154 native cases passed; zero failed IDs; all 23 current applicable required IDs passed exactly once. The new T13 external-MDL cancellation ID is `ok` once.
- Fixture project: `my2pg-mysql80-pg18-92eb15768edc`, versions MySQL 8.0.46/PostgreSQL 18.6; metadata says `stopped`; an exact Docker Compose project-label query returned no running container IDs.
- Artifacts: `target/integration/my2pg-mysql80-pg18-92eb15768edc/`.
- `rust-tests.json` SHA-256: `c8ed60652bae3aa78686b71213ff85c4553a07fb4e2920e7dc4d4b342f260119`.
- Test log SHA-256: `3a56959ec11b90824e27d5351f5a429da0471460634fe5f67e87e0d722add492`.
- Binary source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty; harness SHA-256 `abb5ea6cd66f57ea65f2504c2286a864095ec4ed4a33693e169c3da0a6a1da60`.

## MySQL 8.0.46 → PostgreSQL 16.11 with PostGIS

- Host: Darwin/ARM64; pinned containers: `linux/arm64`.
- Command: `rtk run './tests/run-integration.sh mysql80 pg16-postgis'`.
- Result: exit 0; 154 native cases passed; zero failed IDs; all 24 current applicable required IDs passed exactly once. The T13 external-MDL case and PostGIS-positive `t17_spatial::native_spatial_values_preserve_shape_empty_null_and_srid_with_postgis` each report `ok` once.
- Fixture project: `my2pg-mysql80-pg16-postgis-79cc4e64f214`, versions MySQL 8.0.46/PostgreSQL 16.11; metadata says `stopped`; an exact Docker Compose project-label query returned no running container IDs.
- Artifacts: `target/integration/my2pg-mysql80-pg16-postgis-79cc4e64f214/`.
- `rust-tests.json` SHA-256: `18da770655ad18d26e2df8d1e3ab2f300f004648dc0e4dddad7ca8df43be6460`.
- Test log SHA-256: `7ff74b84807aecd3d0e3711b8d31e9f16d4247da99cab6d55cb3ae5f1b4dfac8`.
- Binary source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty; harness SHA-256 `abb5ea6cd66f57ea65f2504c2286a864095ec4ed4a33693e169c3da0a6a1da60`.

## Remaining refresh cells

All six pinned MySQL 8.0/8.4→PostgreSQL 16/17/18 base cells are now refreshed, and the separate MySQL 8.0→PostgreSQL 16 PostGIS-positive cell also passes. The read-only [matrix refresh audit](2026-10-02-T22-required-matrix-refresh-audit.md) records why older results were stale. MySQL 5.7→PostgreSQL 16/17/18 remains unavailable without a reviewed AMD64 runner and matching pins. NLS, fault, broader platform, fuzz/soak and CI acceptance remain open.
