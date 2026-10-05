# T16 — accepted MySQL `.load` importer subset

- Status: complete for the documented one-way MySQL importer subset
- Agent/role: coordinator
- Task and specification links: [T16](../docs/agent-tasks.md#task-board), [M3 checklist](../docs/checklists.md#m3-compatibility), [import contract](../docs/config-and-cli.md)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`

## Evidence

- The complete offline importer suite passes in the current all-target/all-feature run. It contains independent strict expectations for all 18 selected parser behaviors, classifies all 13 reviewed upstream load fixtures without executing source forms or exposing legacy secrets, tests atomic failure/publication, and drives emitted filters/renames/casts through the real planner and encoder.
- On MySQL 8.4.11 → PostgreSQL 16.15, both native importer cases pass in the serial full harness: `imported_cli_casts_filters_renames_and_defaults_have_native_fidelity` and `imported_existing_modes_preserve_native_oids_and_refusals_publish_nothing`. The second verifies the existing-policy refusal publishes no target schema/artifact and preserves source/outside-target sentinels.
- The refreshed full lane passes 153 tests; all 23 required IDs occur once, none fail or remain unmet, and fixture containers are stopped. Exact pins and artifact digest are recorded in [the current T22 lane result](2026-10-02-T22-mysql84-pg16-current.md).
- Earlier native work corrected the empty-source default path and the imported FK mismatch preflight; see [importer native acceptance](2026-10-01-T16-native-collation-acceptance.md), [FK preflight](2026-10-01-T16-fk-preflight-native.md), and [the parser implementation record](2026-10-01-T16-load-import.md).

Only the documented MySQL subset is accepted. Unknown, non-MySQL, ambiguous, executable, secret-bearing, or unsupported forms fail closed with no partial runnable publication. Other source dialects remain excluded by product scope. This closes T16; broader server/platform certification remains under T22.
