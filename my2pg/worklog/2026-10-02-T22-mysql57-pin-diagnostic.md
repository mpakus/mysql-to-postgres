# T22 — MySQL 5.7 pin and native-runner diagnostic

## Scope and source research

Read-only diagnosis of the `mysql57` matrix gap. No shared pin, matrix, harness, task-board, CI or index changes; no image pull, container start or database test. RTK wrapped repository and Docker commands. Project XERJ queries at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`: `project-my2pg "mysql57 missing pin legacy version matrix"` and `project-my2pg "image digest platform matrix pin"` returned the matrix validator/tests and previous T22 pin/availability worklogs; `project-pgloader "mysql 5.7 Docker image support"` had no relevant migration/test pin precedent. The project workflow is [reference-coding.md](../docs/reference-coding.md). An upstream code reference does not establish an image digest, so registry metadata and the project's own pin validator are the applicable evidence.

Read current `tests/support/matrix.py:14–15, 20–83, 137–189`, `tests/support/test_matrix.py:1–52`, `tests/compose/images.json`, `tests/compose/compose.yml`, and `tests/support/harness.py:267–313`. `validate_pins` requires one immutable digest and one platform per source/target version, validates every MySQL/PostgreSQL pair for matching platforms, and rejects unrecognized manifest entries. `harness.start` also requires each requested pair's platform to equal the local Docker daemon architecture. The matrix does not currently select separate image pins by runner architecture.

## Digest and platform finding

An authoritative MySQL 5.7 digest **can be established from current local evidence**, so lack of a digest is not the underlying blocker. Read-only local inspection:

- Docker daemon: Docker Desktop Linux `aarch64`; 18 CPUs, 7.746 GiB memory.
- Cached `mysql:5.7` RepoDigest: `mysql@sha256:4bc6bc963e6d8443453676cae56536f4b8156d78bae03c0145cbe47c2aad73bb`.
- `docker image inspect mysql:5.7`: `OS=linux`, `ARCH=amd64`, `MYSQL_MAJOR=5.7`, `MYSQL_VERSION=5.7.44-1.el7`.
- Read-only registry query `docker buildx imagetools inspect mysql@sha256:4bc6bc963e6d8443453676cae56536f4b8156d78bae03c0145cbe47c2aad73bb` confirms this is the official `docker.io/library/mysql` OCI index. It contains runtime manifest `sha256:dab0a802b44617303694fb17d166501de279c3031ddeb28c56ecf7fcab5ef0da` for **linux/amd64**, annotated version `5.7.44`, and an attestation manifest. It advertises no ARM64 runtime manifest. The [Docker Hub official-image page for mysql:5.7.44](https://hub.docker.com/layers/library/mysql/5.7.44/?tab=layers) independently lists the same index digest and linux/amd64 platform.

The current host cannot provide native MySQL 5.7 runtime evidence: it is ARM64, the exact registry index has only an AMD64 runtime image, and the owned harness explicitly rejects a host/image architecture mismatch. Emulation must not be relabeled native.

There is also a matrix-layout prerequisite. Adding this AMD64 MySQL pin alone would make all three `mysql57` × `pg16/pg17/pg18` rows fail platform pairing because the existing PostgreSQL pins are ARM64. On an AMD64 runner, the current manifest would still select those ARM64 PostgreSQL pins and the harness would reject them. To run the MySQL 5.7 row natively, the coordinator must choose one reviewed approach: provide matching AMD64 PostgreSQL 16/17/18 pins on a native AMD64 runner, or extend the manifest/matrix/harness contract to select architecture-specific pins while retaining the proven ARM64 variants. Do not replace ARM64 pins globally or add `mysql57` as ARM64.

## Current preflight evidence

`rtk run python3 tests/support/matrix.py --check --all` first exited 1 with `mysql57: missing or malformed pin` and a Cargo test-discovery build error. Repeating the exact Cargo offline compile succeeded; repeating matrix preflight then exited 1 with **only** `mysql57: missing or malformed pin`. Final preflight host is Darwin/arm64; it discovered 149 ignored cases, each Darwin required case exactly once, executed zero database tests, and reported `support_certified=false`. The three MySQL 5.7 lanes remain `pin_metadata_valid=false`, `runtime=not_run`, `compatibility=unproved`. JSON evidence: `/private/tmp/my2pg-T22-mysql57-pin-diagnostic.json`, SHA256 `fc2e1a6de0f944f5472a14fc892f46199ec2cb7bc081dc09488a7ea8fef9fa2d`. The earlier transient build failure was not reproducible and is not attributed to the pin gap.

## Handoff and limits

The pin material is sufficient for coordinator review of `mysql:5.7.44`, digest `sha256:4bc6bc963e6d8443453676cae56536f4b8156d78bae03c0145cbe47c2aad73bb`, platform `linux/amd64`, observed date `2026-10-02`. This worklog does not modify the canonical manifest or certify runtime compatibility. Required next prerequisites are a coordinator-reviewed platform strategy, matching native AMD64 runner/image pins (or platform-aware harness selection), and a version-specific MySQL 5.7 fixture/auth applicability audit before any 5.7 lane. No MySQL 5.7 database lane was run or claimed.
