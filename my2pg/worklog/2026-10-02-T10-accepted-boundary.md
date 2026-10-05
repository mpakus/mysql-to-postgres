# T10 — accepted MySQL value and default boundary

- Status: complete for the reviewed, lossless MySQL value subset
- Agent/role: coordinator
- Task and specification links: [T10](../docs/agent-tasks.md#task-board), [type and value contract](../docs/scope-and-compatibility.md#type-and-value-contract)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`

T10 was reopened after native investigation showed MySQL 8.4.11 can substitute supplementary Unicode in ENUM/SET declaration metadata before exposing it through ordinary metadata protocols. Ordered labels drive ENUM ordinals and SET masks; reconstructing them from rows would be incomplete. The accepted product behavior is therefore fail closed for ambiguous selected declarations before target DDL. This preserves correctness without promising a label-recovery interface that the release server does not provide. The investigation, server evidence, exact charset boundary, and negative native test are recorded in [the ENUM/SET fidelity report](2026-10-01-T10-enum-label-metadata.md); the guard is mandatory on all hosts.

## Verification

- Fresh full serial MySQL 8.0.46 → PostgreSQL 16.15, 17.11, and 18.6 suites each passed 149 cases, with 20 mandatory IDs exactly once and the lossy-label refusal passing in every lane. See [T10 required-case acceptance](2026-10-02-T10-lossy-label-required-registration.md#native-lane-acceptance).
- Fresh full serial MySQL 8.4.11 → PostgreSQL 16.15 passed 153 cases, with all 23 required IDs exactly once, no failed or unmet IDs, and all fixture containers stopped. See [the current lane record](2026-10-02-T22-mysql84-pg16-current.md).
- The runs execute the complete selected native suite, covering numeric, temporal, text/encoding, binary, ENUM/SET, source literal defaults, empty values, and conversion/reject semantics; the two T10 safety cases are additionally enforced as mandatory and recorded exactly once.
- Lossless ordered-label recovery remains outside the accepted boundary for source catalogs that collapse supplementary labels; the tested diagnostic prevents a corrupt plan. Native matrix support beyond the executed cells remains T22 scope.

The T10 task can close at this explicit supported boundary. This does not close the separate aggregate M2 checklist items, which include unfinished T11/T12/T13/T15 requirements.
