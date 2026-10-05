# 2026-10-05 — T23 — current Linux/AMD64 application-container smoke

- Status: AMD64 application image smoke passed under emulation; native Linux release gate remains open
- Agent/role: coordinator / integrator
- Task and acceptance links: [T23 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Host: Darwin/ARM64 with Docker Engine 29.8.0 (`aarch64`)
- Application image platform: `linux/amd64` under emulation
- Database platform: pinned `linux/arm64` MySQL/PostgreSQL images

## Verification

Ran `DOCKER_DEFAULT_PLATFORM=linux/amd64 rtk test './tests/run-container-smoke.sh'`. The current-source multi-stage image built and ran as Linux/AMD64. The smoke migrated the fixture into MySQL 8.4.11 → PostgreSQL 16.15 with verified TLS. The image reported `my2pg 0.1.0`, defaulted to UID/GID `10001:10001`, and contained no Java, SBCL, or Clojure executables.

The migration report says `status=complete` and `verification.status=complete`, with one table checked and no differences. The independent fixture queries confirmed three committed rows, exact binary bytes (`00015c09ff`), exact decimal value (`1234567890123456789012.12345678`), and the UTF-8 value `emoji 😀`.

Result artifact: `target/container-smoke/babaf626d034/result.json`. The application image ID is `sha256:985cfae05320d207e157384baf8b7718b8f464249727050dede8ad0cfd307011`; the embedded binary SHA-256 is `497043b5299ccf141fa79324f9c156bccdb71863648808e1eea258af1d485c15`. The report SHA-256 is `11fc66744e0ab505673f8bb54877d9e3f11e32f265051f24c4f0f9d3323f0be6`.

## Cleanup and limits

The result remains `passed` after the smoke's cleanup routine. The fixture `connections.json` records `state=stopped`, no integration-labeled containers remained, and Docker confirmed the temporary image had been removed. This is an AMD64 application container running under emulation on a Darwin/ARM64 host; MySQL and PostgreSQL ran as ARM64 containers. It does not prove native Linux-host execution, native AMD64 database lanes, MySQL 5.7 coverage, a distributed binary, or release publication.

## Handoff

The release checklist now distinguishes the AMD64 application-container smoke from native Linux/AMD64 evidence. Refresh the `project-my2pg` XERJ index after this documentation/worklog update. No pinned reference index or checkout was changed.
