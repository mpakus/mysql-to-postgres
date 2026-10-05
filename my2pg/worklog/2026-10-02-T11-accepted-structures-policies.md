# T11 — accepted structure, identity and target-policy behavior

- Status: complete for the reviewed MySQL structure and policy subset
- Agent/role: coordinator
- Task and specification links: [T11](../docs/agent-tasks.md#task-board), [M2 checklist](../docs/checklists.md#m2-reliable-core), [structure and naming contract](../docs/scope-and-compatibility.md#structure-and-naming-contract)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`

## Closure of documented gaps

The earlier [policy audit](2026-10-02-T11-policy-mode-audit.md) identified three concrete gaps. Later changes and tests now cover each one:

1. Data-only migrations require an explicit `append` or `truncate` policy; the ambiguous default `error` is rejected offline. See [policy explicitness](2026-10-02-T11-data-only-policy-explicitness.md).
2. Existing-target policy tests cover schema-only refusal and actual schema-only recreate, with target structures, zero copied rows, and outside sentinels asserted. See [schema-only recreate](2026-10-02-T11-schema-only-recreate.md) and native test `t11_schema_only_recreate` in the current lane.
3. `data_only + truncate + reset_sequences=true` now has a real MySQL/PostgreSQL run proving selected rows are replaced, constraints/identity survive, the sequence advances to the required source bound, and source/outside-target sentinels stay unchanged. See [truncate sequence reset](2026-10-02-T11-truncate-sequence-reset.md).

## Integrated acceptance

The current full serial MySQL 8.4.11 → PostgreSQL 16.15 run passes all registered T11 tests, along with the full lane's 153 passed cases and 23 required IDs. T11's native cases independently exercise ordered/typed catalogs, dependency closure and malformed metadata, renamed composite structures, PK/UNIQUE/index/FK and constraint-trigger behavior, comments, schema mapping, append/truncate/recreate, schema-only/data-only/full modes, identity width/value boundaries, sequence ownership/state/exhaustion and privilege refusals. The exact artifact, image pins, stopped fixture and log digest are in [the current T22 lane record](2026-10-02-T22-mysql84-pg16-current.md).

Additional source-version evidence includes the MySQL 8.0.46/PostgreSQL 16.15 truncate/reset run and signed/unsigned identity mappings; T22's earlier MySQL 8.0.46 lanes against PostgreSQL 17.11 and 18.6 also exercised the T11 native suite. Those historical runs are cross-version evidence, not a replacement for the open T22 matrix.

Unsigned BIGINT AUTO_INCREMENT remains explicitly unsupported and fails before destination DDL; its numeric data representation remains lossless. The accepted T11 scope is complete at this refusal boundary. Full platform/version certification remains T22 work, and the separate M2 shutdown/storage limits remain in T12/T13.
