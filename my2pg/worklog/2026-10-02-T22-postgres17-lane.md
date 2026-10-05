# T22 PostgreSQL 17 native ARM64 lane

## Reference research and pin verification before editing

Exclusive lease: `tests/compose/images.json` plus this worklog. No harness/matrix/CI/test/source/board/index changes. Applied RTK proxy and Ponytail guidance: reuse existing strict discovery and owned lifecycle, add one independently verified pin. Live/index label `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; dirty integrated worktree includes the coordinator's PostgreSQL 18 NOT NULL correction and T18 content verifier. References remain immutable/read-only.

Read AGENTS, reference-coding.md, scope-and-compatibility.md:7–9, agent-tasks.md T22/dependencies/current follow-ups, checklists.md, testing-and-performance.md:68–81. Full scheduled contract remains MySQL 5.7/8.0/8.4 × PostgreSQL 16/17/18; native development passes do not certify unexecuted source versions/platforms. Fresh XERJ queries through `bin/reference-search.py`: `project-my2pg 'T22 pg17 pinned images matrix required cases' -k 3 --full 1300` returned T22 matrix-preflight, required-case follow-up and PG18 lane research at the above revision. `ref-rust-postgres 'docker postgres' -k 2 --full 900` returned README and original docker-compose.yml at pin `1084ca8f5b5302e161892f2fa40abf71b4060c10`. Read that original compose and adjacent docker/sql_setup.sh: upstream uses mutable postgres:18, fixed port and fixture auth setup; do not adopt its image pin, global setup or TLS credentials. No reference code copied.

Read actual `tests/support/matrix.py:1–210`, harness.py ownership/start/version guards:120–220 and build/runtime/discovery/finally cleanup:327–405, adjacent `test_matrix.py` metadata/parser/error/cfg tests and `test_harness.py` isolation/missing/ignored/duplicate checks, compose images/volumes. Existing strict checks reject absent pins, wrong native architecture, invalid versions, absent requested cases and failed runtime. Preserve all of them.

Official registry read commands (all exit 0, no containers started):

```sh
rtk proxy docker buildx imagetools inspect postgres:17-alpine --raw
rtk proxy docker buildx imagetools inspect postgres:17.11-alpine3.24 --raw
rtk proxy docker buildx imagetools inspect postgres@sha256:0b2882c46a2b8d8e148431b333394348cf4d7f15aec6df7f17d51dc641e56cfa --raw
rtk proxy docker buildx imagetools inspect postgres@sha256:0b2882c46a2b8d8e148431b333394348cf4d7f15aec6df7f17d51dc641e56cfa --format '{{json .Image}}'
```

Floating and exact-tag indexes have identical SHA256 `b0f9560a2de083e2cc7382e75f808c7381a32852a7ec49117deedb300e552b24`; exactly one linux/arm64/v8 child is `sha256:0b2882c46a2b8d8e148431b333394348cf4d7f15aec6df7f17d51dc641e56cfa`. Independently fetched **2869 raw child bytes hash to that digest**; media type is OCI image manifest with no manifests array, not index/attestation. Config descriptor: `sha256:25a89970a83255ae96484d709005bb35aa71f7455f941bc1634514f31c5d5c62`, size 8746. Config verifies linux/arm64, `PG_MAJOR=17`, `PG_VERSION=17.11`, `PGDATA=/var/lib/postgresql/data`, VOLUME `/var/lib/postgresql/data`, config created `2026-09-17T21:31:46.755310538Z`.

Manifest annotation exact version `17.11-alpine3.24`, created `2026-09-17T21:29:09Z`, official source revision `2603e26e245e558218728ee14e0a42dcb020dc7f` with [immutable official Dockerfile](https://github.com/docker-library/postgres/blob/2603e26e245e558218728ee14e0a42dcb020dc7f/17/alpine3.24/Dockerfile). Read primary Dockerfile lines50–52/version and186–189/PGDATA/VOLUME and [official image README](https://github.com/docker-library/docs/blob/master/postgres/README.md). PostgreSQL17 retains the pre-18 data path; Compose does not override PGDATA or bind a conflicting path, only TLS files. Let the image supply its owned anonymous data volume and keep scoped `down --volumes` cleanup. Raw registry evidence retained `/private/tmp/my2pg-t22-pg17-{index,exact-tag,arm64-manifest,arm64-config}.json`.

Chosen adaptation: only add `pg17` with official child digest, linux/arm64, exact provenance tag `postgres:17.11-alpine3.24`, prefix17., kind postgres and date2026-10-02. No source applicability/assertion changes. Test plan: existing Python parser tests/doctest, offline all-lane compile/list with expected remaining mysql57/mysql80 missing pins, then full owned `harness.py mysql84 pg17` once, preserve failures, versions, actual mandatory counts and scoped cleanup. Primary PG17 image config alone is not a runtime pass.

Read-only resource preflight: Docker host aarch64, 18CPUs, 8,317,267,968bytes RAM. `docker ps` lists only six untouched Strangler services; no active my2pg harness. Coordinator assigned this serial lane; do not reuse or stop sibling services. Each owned pair has MySQL1GiB/PostgreSQL512MiB limits; benchmark timings are not claimed.

## Offline preparation

After the single pin edit, Python support unit suite **19/19 pass**, harness doctest exit0 and git diff--check exit0. `rtk proxy python3 tests/support/matrix.py --check --all > /private/tmp/my2pg-T22-pg17-preflight.json` exits1, **only** mysql57/mysql80 missing pins; all three mysql84→PostgreSQL16/17/18 pairs validate native linux/arm64 metadata. Cargo build/list succeeds with139 ignored case IDs and all15 Darwin/NLS required IDs exactly once. No database tests execute in preflight. Evidence SHA256 `f424f63b1898be04c063e062c5da8ae781da374d9f413a25699fea02d0da0031`.

Coordinator confirmed no concurrent owned fixture and released serial runtime. Full unmodified command started with preserved raw output:

```sh
rtk proxy python3 tests/support/harness.py mysql84 pg17 \
  > /private/tmp/my2pg-T22-pg17-native.stdout \
  2> /private/tmp/my2pg-T22-pg17-native.stderr
```

Fresh pair `my2pg-mysql84-pg17-a17185d1846b` reports actual native MySQL8.4.11/PostgreSQL17.11; all14 independent seed/auth/TLS checks passed. Full serial suite is in progress; no acceptance claimed until exit/status/required-case/cleanup evidence exists.

## Complete native result and cleanup

Full owned harness **PASSED**, harness/Cargo/preparation exit0: **138 passed, zero failures, zero ignored among invoked cases**. Suite breakdown: library3, driver11, integration124; other targets correctly contain no invoked ignored native cases. Native integration suite elapsed220.31s. Required runtime acceptance IDs all **11** executed exactly once with status `ok`, none missing: T10 empty default, both T16 importer cases, both T15 durable cases, both T13 panic cases, both T11 identity-edge cases, T13 original-snapshot case and macOS T12 uncertain-storage case. Exact per-ID statuses and command are retained in rust-tests.json. Offline139 versus native138 is expected: all-feature preflight additionally lists the separate `nls-integration` target; generic native command does not enable/run the NLS case. No PostgreSQL17 localized-message or Linux fault equivalence claim.

Actual servers: Oracle MySQL **8.4.11**, PostgreSQL **17.11**, native linux/arm64, official selected platform digests. All14 seed/auth/TLS checks passed including MySQL default caching_sha2_password and PostgreSQL clientcert HBA. Local pulled image inspection independently confirms exact PG17 RepoDigest `postgres@sha256:0b2882c46a2b8d8e148431b333394348cf4d7f15aec6df7f17d51dc641e56cfa`, linux/arm64 and volume `/var/lib/postgresql/data`. Source/build revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, dirty integrated working tree; production executable SHA256 `4e15695cb4f3facdfe627b8491f26e6c151550f3b8a1629f8738a4164863f9da`. This is exact development-lane proof, not a packaged release certification.

Cleanup: harness validated project ownership then completed scoped Compose `down --volumes --remove-orphans`; saved state **stopped**. Independent `docker ps -aq --filter label=com.docker.compose.project=my2pg-mysql84-pg17-a17185d1846b` returned exit0/empty. No unowned Strangler resources were changed. Retained owned evidence directory `target/integration/my2pg-mysql84-pg17-a17185d1846b`:

- rust-tests.json SHA256 `1f640396b05aafc234e1f629515b3255075843e1f31804d5ea2643714a6f78e8`; log SHA256 `485948bf0027709ef56668cb9b4a6397af2cef06ae21e862123c8642eeb50ddc`.
- stopped connections.json SHA256 `a1f1df20763926da8be29f4eeb78d8a84156a6ce44e9301c9bfe5056740f1d29`; containers.log `790245d8fba7df556e0ded1d3100a4ee8ce7a9e5db818895066f7b048e6fd4e4`.
- seed-checks.json `5b3a0df54525562e282cbe23e84c501d052cc5a82a9a0dda3cf0ac73777ea531`; raw stdout `/private/tmp/my2pg-T22-pg17-native.stdout` SHA256 `245e7abe3841cc3ea1863e706360a8a49345e21966d9b1052191b5335310f953`, stderr counterpart `1562cfe18f00223f52c317bfeb61359b2a035aeed67e498ef977245e0af74a36`.

Matrix progress: MySQL8.4 now has native full-suite evidence against PostgreSQL16/17/18 (16/18 by coordinator,17 by this run). mysql57/mysql80 pins and six source-version cells remain absent/unexecuted; broader native architecture, CI, NLS, soak/fuzz/fault/performance and release gates remain open. No fixture relaxation, unsupported source-version pass or board/index update. Only the PG17 pin and this log changed; lease frozen for coordinator review.
