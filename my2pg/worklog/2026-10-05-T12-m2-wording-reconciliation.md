# 2026-10-05 — T12 / M2 acceptance wording reconciliation

- Status: T12 implementation and registered fault-case acceptance confirmed on the six available current-source base cells; hosted and broader release gates remain open
- Agent/role: coordinator / evidence audit
- Specification: [T12 acceptance closeout](2026-10-02-T12-acceptance-closeout.md), [task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release), and [T22 matrix contract](../docs/testing-and-performance.md#test-harness-and-ci)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Tested source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Tested executable SHA-256: `c3e45d02ce6dabd0eb390fbb525991d3966723adf72ce403a0f7882d08a34b9a`

## Question and method

An audit of M2 checklist language found a remaining statement that broader T12 fault coverage was open, while the task board and T12 closeout mark implementation acceptance complete. I independently inspected the six canonical matrix ledgers named in the [2026-10-04 current-source matrix refresh](2026-10-04-T22-current-source-matrix-refresh.md). I counted only case IDs prefixed `t12_` in each `runnable_case_results` object and required exactly one `ok` result per ID. This avoids counting older duplicate lane artifacts or the separate NLS case as part of the six-cell base matrix.

## Findings

Each canonical ledger reports 156 passed cases and no failed case IDs. All 30 registered `t12_*` cases appear and pass exactly once in every cell (180 T12 case executions total):

| Current-source cell | Isolated project | Registered T12 results |
| --- | --- | ---: |
| MySQL 8.0 → PostgreSQL 16 | `my2pg-mysql80-pg16-c7c9aad44873` | 30/30 passed |
| MySQL 8.0 → PostgreSQL 17 | `my2pg-mysql80-pg17-73c3d37adc70` | 30/30 passed |
| MySQL 8.0 → PostgreSQL 18 | `my2pg-mysql80-pg18-9dba744b6ef3` | 30/30 passed |
| MySQL 8.4 → PostgreSQL 16 | `my2pg-mysql84-pg16-4aa749211489` | 30/30 passed |
| MySQL 8.4 → PostgreSQL 17 | `my2pg-mysql84-pg17-84107cd497ac` | 30/30 passed |
| MySQL 8.4 → PostgreSQL 18 | `my2pg-mysql84-pg18-46b87cea2d83` | 30/30 passed |

This supports marking T12 implementation/fault acceptance complete for the available lanes. It does not establish hosted CI execution, MySQL 5.7 or native AMD64/Linux support, or final integrated release-candidate acceptance. T22 and T24 retain those broader release gates. No additional T12 test was justified by this audit.

The saved integration directory contains other historical or focused-run ledgers, so an unrestricted glob yields duplicate and noncanonical entries. The six project names above were taken from the current-source matrix report and checked directly; all six record the same executable hash. There is no configured Git remote in this checkout, so hosted Actions evidence is unavailable locally.

## Documentation changes and checks

- Updated the M2 history and release checklist to say T12 implementation acceptance is complete on the available lanes and to keep the overall gate open for hosted execution, full release-candidate matrix/platform evidence, and integrated release acceptance.
- Updated the coordinator gate worklog so its decision and reviewer follow-up no longer cite broader T12 fault coverage as unresolved.
- No application or test code changed.
- Independent reviewer: `/root/t12_release_audit`; audit conclusion matched the canonical-ledger recount and confirmed all 30 T12 cases passed in each of the six current matrix lanes.
- XERJ project index was refreshed after the documentation edits: `project-my2pg-source`, 443 files, 1,535 passages, 14 exclusions. Query `T12 M2 wording reconciliation 30 registered fault cases six current source lanes` retrieved this worklog, the updated coordinator gate report, and the checklist at indexed source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; a final refresh follows this evidence entry.
- Next: preserve T12 as complete on these lanes; continue T22 hosted/platform gates and leave overall release at no-go until all release evidence is available.
