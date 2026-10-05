# 2026-10-05 — T23 — current-source Linux/ARM64 container smoke

- Status: current container migration passed; distributed/native-Linux release gate remains open
- Agent/role: coordinator / integrator
- Task and checklist: [T23 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty

## Method and result

Ran `rtk run './tests/run-container-smoke.sh'`. The runner built the current source as a uniquely tagged container, checked its Linux runtime metadata and non-root default user, asserted that `java`, `sbcl`, and `clojure` are absent, then migrated the owned TLS-enabled fixture over `verify_full` MySQL and PostgreSQL connections.

- Image: `my2pg:container-smoke-47865f9f8f42`, image ID `sha256:38f480390931c4a9a586a5edebcc5f8b474abde7c8ce3377ebff57accb91d659`; Linux/ARM64, default user `10001:10001`.
- Application binary: `my2pg 0.1.0`, SHA-256 `8de4331caf9eaa0a70149387e6568de55b6df48f02be6239e3783e494940261a`.
- Fixture: MySQL 8.4.11 → PostgreSQL 16.15, project `my2pg-mysql84-pg16-0ca17a778f5f`, Linux/ARM64; its `connections.json` records `state=stopped` after cleanup.
- The migration committed all three `users` rows. Independent target reads matched payload bytes `00015c09ff`, decimal `1234567890123456789012.12345678`, and UTF-8 value `emoji 😀`; the durable report says verification `complete`, one table checked, and no differences.
- Report SHA-256: `5715408203ea1417aebc53e20585214022b8693597d734adea0e7374ada89a68`.
- Result SHA-256: `c0f6d389c2e734f38b10f5a68be094420ae6ed6cb13963dfa083b81d0d79a041`; result artifact: `target/container-smoke/47865f9f8f42/result.json`.
- Post-cleanup Docker queries found no owned integration containers and no image tagged `my2pg:container-smoke-47865f9f8f42`.

## Limits and handoff

This is current dirty-source Docker Desktop Linux/ARM64 runtime evidence, not a native Linux-host test or a distributed artifact. It does not close reproducible Linux/macOS distribution, release checksum, hosted CI, or human license/fixture-attribution gates. The source archive and local build results remain separate from a published release.

- Worklog owner: coordinator; no source, fixture, harness, or image definition changed for this validation.
- Next: obtain native Linux and distributed release-artifact evidence; preserve the no-go status until all remaining T21–T23 gates and independent final review are complete.

## Independent review

`/root/t24_independent_review` matched the result/report hashes and metadata, confirmed that all independent target SQL assertions completed, and verified the fixture is stopped with no owned containers or temporary image remaining. It confirmed the checklist keeps native Linux-host execution and distributed artifacts open.
