# T22 PostgreSQL 18 ARM64 lane

## Before the pin change

Lease: only `tests/compose/images.json` and this worklog. No harness, matrix, CI, task-board, reference-index or production changes. RTK proxy wraps reads and checks; Ponytail guidance favors the existing harness and a single immutable pin. Working source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, with concurrent T11/T18 changes; this is not a clean release revision.

Read `docs/reference-coding.md`, the T22 lane contract, `tests/support/matrix.py`, `tests/support/harness.py`, its adjacent discovery tests, `tests/compose/images.json` and `tests/compose/compose.yml`. XERJ queries: `project-my2pg 'pg18 immutable platform digest native matrix Compose' -k 3 --full 1000` returned the matrix-preflight worklog, matrix validator and `docs/testing-and-performance.md:61`; `ref-rust-postgres 'docker postgres 18' -k 1 --full 1000` returned original `docker-compose.yml:1–9` at `1084ca8f5b5302e161892f2fa40abf71b4060c10`. The reference uses mutable `docker.io/postgres:18`: useful version evidence, insufficient pin provenance. Our matrix needs separate, validated platform pins and executed native cases; adding a pin cannot certify a lane.

Primary official sources read: [official image README](https://github.com/docker-library/docs/blob/master/postgres/README.md), [official image tags](https://github.com/docker-library/official-images/blob/master/library/postgres), and the actual Docker Hub registry manifests/config below. PostgreSQL 18 changed image PGDATA to `/var/lib/postgresql/18/docker` and VOLUME to `/var/lib/postgresql`. Existing Compose only binds TLS files and does not mount the pre-18 `/var/lib/postgresql/data` path or override PGDATA. Keep Compose unchanged: image-provided anonymous volume initializes the owned database, and scoped `docker compose down -v` must remove it after runtime.

Registry verification on 2026-10-02, without pulling or starting a container:

```sh
rtk proxy docker buildx imagetools inspect postgres:18-alpine --raw
rtk proxy docker buildx imagetools inspect postgres:18.6-alpine3.24 --raw
rtk proxy docker buildx imagetools inspect postgres@sha256:89f747171c4b0af0eacf5984550060be79786dbe286eb60cfa691d79d1e8b23f --raw
rtk proxy docker buildx imagetools inspect postgres@sha256:89f747171c4b0af0eacf5984550060be79786dbe286eb60cfa691d79d1e8b23f --format '{{json .Image}}'
```

Both tag indexes have SHA256 `77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873`, each with exactly one linux/arm64/v8 entry. Its child digest is `sha256:89f747171c4b0af0eacf5984550060be79786dbe286eb60cfa691d79d1e8b23f`. The separately fetched child's **2680 raw bytes hash to that exact digest**, its media type is `application/vnd.oci.image.manifest.v1+json` and it has no `manifests` array. This is a platform image manifest, not the parent index or an attestation. Config digest: `sha256:d7a8005067f55a2f8ea369e767545fd467b1943a4f0f1a48b7dc44f06c366109`. Image config confirms linux/arm64/v8, `PG_MAJOR=18`, `PG_VERSION=18.6`, the PGDATA/VOLUME paths above, entrypoint `docker-entrypoint.sh`, CMD `postgres` and SIGINT stop signal.

Registry annotations identify exact tag `18.6-alpine3.24`, official build source `https://github.com/docker-library/postgres.git#e00e1bd34ec5c8a8e7ad89b273b3d42efaf6d5bc:18/alpine3.24`, source revision `e00e1bd34ec5c8a8e7ad89b273b3d42efaf6d5bc`, platform creation `2026-09-17T21:28:27Z`; config creation is `2026-09-17T21:30:59.665677387Z`. Remote evidence retained at `/private/tmp/my2pg-t22-pg18-{index,exact-tag,arm64-manifest,arm64-config}.json`.

Chosen adaptation: add only `pg18`, official child-digest image, platform `linux/arm64`, exact observed tag `postgres:18.6-alpine3.24`, date `2026-10-02`, kind postgres and version prefix `18.`. Preserve all old pins and assertions.

Test plan: Python unit/doctest checks and database-free `matrix.py --check --all`, expecting the existing missing mysql57/mysql80/pg17 pins to remain errors. Then, only after coordinator schedules stable T11/T18 files, execute one serial unique-owned `mysql84 pg18` harness lane, preserve all raw failures and required-case evidence, independently inspect actual server versions and cleanup. No unrelated containers are reused or stopped. All other version lanes and release/platform certification remain open.

Coordinator explicitly deferred database runtime until T18 freezes and T11 finishes its current native case. Offline validation is authorized now.

## Offline evidence after the single pin

- `rtk proxy python3 -m unittest discover -s tests/support -p 'test_*.py'`: 19/19 pass.
- `rtk proxy python3 -m doctest tests/support/harness.py`: exit 0.
- `rtk proxy git diff --check`: exit 0.
- `rtk proxy python3 tests/support/matrix.py --check --all > /private/tmp/my2pg-T22-pg18-preflight.json`: expected exit 1, **only** missing mysql57, mysql80 and pg17 pins. Both mysql84→pg16 and mysql84→pg18 pin metadata validate on linux/arm64. Offline Cargo build/list succeeds, discovers 138 ignored cases and every required Darwin/NLS case exactly once (15). No database test executed. Evidence SHA256: `10a4bc5e39042da47f0857f5b01cc6a09eb92e425ff32ffedd1f90745b23c531`.

Runtime remains deferred pending the coordinator's serial-lane go-ahead. PostgreSQL 18 coverage is currently pin/discovery preparation only.

## First native lane: real incompatibility, not accepted

Coordinator released the serial lane only after its mysql84→pg16 full harness passed 138 cases and scoped cleanup completed. Ran **once**:

```sh
rtk proxy python3 tests/support/harness.py mysql84 pg18 \
  > /private/tmp/my2pg-T22-pg18-native.stdout \
  2> /private/tmp/my2pg-T22-pg18-native.stderr
```

Harness exit 1; requested Cargo test child exit 101. Unique pair `my2pg-mysql84-pg18-98d457d19752` initialized successfully with actual Oracle MySQL **8.4.11** and PostgreSQL **18.6**, native linux/arm64 pins and loopback endpoints. All 14 independent seed/TLS/auth checks passed, including MySQL caching_sha2_password and PostgreSQL clientcert HBA facts. Existing Compose/image PGDATA setup worked. Pulled local image reports the exact pinned RepoDigest, linux/arm64 and `/var/lib/postgresql` volume declaration.

The first library suite ran 3 cases: `postgres::tests::native_catalog_union_uses_one_read_only_repeatable_snapshot` passed; both `pipeline::worker_fault_tests::native_runner_panic_after_ack_retains_prefix_and_closes_sibling` and `pipeline::worker_fault_tests::native_runner_panic_preserves_registered_unread_source_backend` failed at `src/pipeline/mod.rs:2291`, before their intended panic path, because DataOnly planning rejected four `TARGET_CONSTRAINT_UNSUPPORTED` observations. SchemaOnly setup/verification had succeeded. PostgreSQL 18's new NOT NULL constraint catalog representation is a likely cause, **not yet proved** by this stopped fixture. `src/postgres/observed_constraints.sql:4` exposes actual contype; planner's finite constraint classifier at `src/plan/existing.rs:690–750` rejects unknown kinds. Reported the native defect to coordinator; no production/test/assertion changes under this lease.

Cargo stops at the failed library target, so later integration/driver targets **did not run**. `rust-tests.json` correctly retains 1 passed case, both FAILED mandatory panic cases and every other mandatory case missing; no lane acceptance or 138-case PG18 claim. Cargo preparation succeeded, test-only artifact bridge points to the actual production executable, executable SHA256 `6f9723b58974f3f2224f48a43e4ea3f2181c77e6475689d8adb465dc3955aa5d`. Build revision remains `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, dirty integrated working tree.

Artifacts: `target/integration/my2pg-mysql84-pg18-98d457d19752/{connections.json,rust-tests.json,rust-tests.log,rust-build.json,seed-checks.json,containers.log}` and the two panic helper report trees. Preserve the failed run; do not overwrite or weaken it. `rust-tests.log` recorded SHA256 `a0b308f2dae56d286dd357fd345a165bfdd9e279fb731542e0a0060b6958d75b`; captured stdout SHA256 `0f35687ecd0b9f2ed1e70317e01cdba15af5319119f8a4c4e6dd8c8003f025d8`, stderr `62656dba048eb3dea2f59e422978274ee11196b1d10cccc59fd1ef4dfc0e43b1`.

Cleanup: harness `finally` executed scoped Compose `down --volumes --remove-orphans`; saved state is **stopped**. Independently queried `docker ps -aq --filter label=com.docker.compose.project=my2pg-mysql84-pg18-98d457d19752`: exit 0, empty. No unrelated containers were stopped or reused. Containers log SHA256 `73aa9a38cafb4b6301f8629341bd80006437bbb04748779eb593b4c2ef73ffab`, stopped connection metadata SHA256 `e6209d4b8400691952d8476ad6d384d69ac77a2e60ef6c14eb6ca8911da118e7`.

Current matrix: mysql84→pg16 has coordinator's complete development proof; mysql84→pg18 is an **executed failing lane**, with successful image/startup/14 seed checks and only 1/3 library cases passing. mysql57/mysql80/pg17 pins remain absent, leaving seven matrix cells unexecuted here. Full release/platform/locale/benchmark certification remains open. Pin and worklog frozen for coordinator review; further native runs require repaired production compatibility and a new serial go-ahead.

## Second native lane after coordinator's first NOT NULL correction

Coordinator authorized one full rerun after integrating a finite `contype='n'` planner rule with units/Clippy/compile passing. Command unchanged except preserved output paths `/private/tmp/my2pg-T22-pg18-native-rerun.{stdout,stderr}`. Fresh unique pair `my2pg-mysql84-pg18-04587ca298c4` again proved MySQL 8.4.11/PostgreSQL 18.6 and 14 seed checks. Preparation exit 0; newly built production executable SHA256 `060bfe9fc502d9f2ffb4194fba3a3d33b7c97011a24c2d698f6a37ef674d0d98` differs from first run. Source remains dirty revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.

Result remains **FAILED**: harness exit 1, Cargo exit 101, library 1 passed/2 failed/0 ignored. Same two mandatory panic IDs failed at pipeline:2291 on four `TARGET_CONSTRAINT_UNSUPPORTED`; the read-only union snapshot passed. Later targets and all other mandatory cases were not executed. Preserve first and second failures separately.

Exact observable constraint evidence limitation: emitted errors retain only diagnostic codes; helper publishes successful SchemaOnly plans/reports, but the rejected DataOnly inspection/planning objects are not written by current code. Therefore no raw `TargetConstraintPart` observations can truthfully be recovered from this stopped run. No extra fixture was started to invent evidence. Read-only inspection of current `supported_not_null_constraint` at existing.rs:834–877 shows it requires `match_type='s'`, `update_action='a'`, `delete_action='a'`, despite these being FK-specific fields. Actual non-FK catalog values may differ; this is a concrete **unproved hypothesis**, handed to coordinator for native catalog capture, not a reason to relax assertions. No production changes made.

Scoped cleanup completed; metadata state **stopped** and independent exact-project `docker ps -aq` exit 0/empty. Retained `target/integration/my2pg-mysql84-pg18-04587ca298c4` with rust-tests.json SHA256 `f07c62590b6bd937d7425b363fe4d28cbb249a2d6209d75e2860207e1ff9971b`, rust-tests.log `0794b13a335b6f346596b9c89c79109e963c21b817dd5b90b6c23003f0cce932`, stopped connections.json `22275750494f5b7e61e81c8763ad1eaa5beddab5da78069cf08fa2a5f4cfa9b2`, containers.log `83dd7ffd46a1dfcb2f8efc90c8f060e478992452394df9cb43606eccd7f86ba3`. Captured stdout `c94e8a930a7755c845786dfe024d67d76f3799e7c8157af696f279acd325c646`, stderr `c63d45619788300bfb1cad84e86e609a0d4a9f1fbd1075311acf486d7a3ed0ba`. Files frozen; PG18 remains unaccepted and matrix gaps unchanged.

## Coordinator runtime acceptance update (2026-10-02)

The failed runs above are preserved. Coordinator captured exact PostgreSQL 18 `contype='n'` rows and applied two narrow compatibility changes recorded in [the catalog compatibility worklog](2026-10-02-T22-pg18-not-null-catalog-compatibility.md): validate NOT NULL rows structurally during existing Append, and recognize the owned `n` kind during whole-table recreation. Focused T13 native cases passed. A complete PG18 run first found a separate relation-recreation allowlist gap; after its correction, the full lane passed.

Accepted native ARM64 lanes each ran all 138 cases with exit code 0, no failed IDs and no unmet mandatory case IDs:

- MySQL 8.4.11 → PostgreSQL 18.6: `target/integration/my2pg-mysql84-pg18-277a344607d5/rust-tests.json` and `rust-tests.log`.
- MySQL 8.4.11 → PostgreSQL 16.15: `target/integration/my2pg-mysql84-pg16-0616894ae3e7/rust-tests.json` and `rust-tests.log`.

Both pairs were stopped by the ownership-checked harness. This accepts PG18 for the exercised MySQL 8.4 source lane; the wider T22 version/platform matrix, NLS, cross-platform faults, fuzz/soak, CI and release gates remain open.
