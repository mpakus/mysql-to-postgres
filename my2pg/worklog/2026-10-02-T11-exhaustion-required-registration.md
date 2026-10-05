# 2026-10-02 — T11 identity exhaustion required-case registration

Status: complete for mandatory registration and refreshed MySQL 8.0/PostgreSQL 16–18 native gates; broader T11 structure and policy work remains open. Coordinator lease: `tests/support/harness.py`, `tests/support/test_harness.py`, this worklog, and affected current acceptance summaries. No source planner, native case, fixture, or pinned reference edits.

## Reference research before edit

- Read `docs/reference-coding.md`, the T11 acceptance row in `docs/agent-tasks.md`, T11 identity requirements in `docs/scope-and-compatibility.md`, and the mandatory native-case rules in `docs/checklists.md` and `worklog/2026-10-02-T22-required-case-followup.md`.
- XERJ project query: `project-my2pg "T11 identity exhaustion AUTO_INCREMENT native required case harness mandatory IDs"`, current index label `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It found the passing boundary case, its prior worklog, the coordinator registration worklog, and the harness's cross-platform required-case contract.
- XERJ pinned pgloader query: `project-pgloader "MySQL AUTO_INCREMENT exhaustion sequence PostgreSQL boundary"`, pinned revision `231ab86778ca5ffd7de40878714760c8b4860cdf`. The lexical results were not relevant to native harness policy, so no pgloader code is being adapted. The task's earlier boundary worklog already inspected pgloader sequence code/tests and Paganel's sequence renderer; neither supplies this fail-before-target-DDL guarantee.
- Read originals: `tests/support/harness.py:21-47`, `tests/support/test_harness.py:13-39`, `tests/integration.rs:32-33`, and `tests/cases/t11_identity_exhaustion.rs:1-147`. The test creates a signed SMALLINT source identity at 32767, observes next value 32768, requires `IDENTITY_RANGE`, and proves no target schema was created. Existing full serial PG16/17/18 artifacts each show this test passed, but its exact ID is absent from `required_case_results`.
- RTK commands wrapped repository reads and test commands. Ponytail guidance was read from `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`; the smallest complete harness change is one required-ID string and one cross-platform membership assertion. No new abstraction or database test is needed.

## Chosen adaptation and proof

Promote the already passing native boundary regression into `REQUIRED_CASES` and the generic discovery assertion. Do not place it in the macOS-only list: it uses the owned MySQL/PostgreSQL services and no host-specific storage behavior. Acceptance is the seven support-harness tests plus fresh full serial MySQL 8.0.46 → PostgreSQL 16.15, 17.11, and 18.6 runs. Every lane must report this exact case `ok` exactly once in the mandatory result map and have no unmet IDs.

## Native lane acceptance

- `python3 tests/support/test_harness.py`: all seven harness tests passed, including generic Linux/macOS membership and fail-closed missing/ignored/failed/duplicate result checks.
- Fresh serial MySQL 8.0.46 → PostgreSQL 16.15, 17.11, and 18.6 runs each passed 149 cases, had zero failed cases, and recorded all 19 Darwin-required IDs exactly once. For every `rust-tests.json`, `t11_identity_exhaustion::native_runner_refuses_source_auto_increment_beyond_smallint_sequence_max` is `["ok"]` and `unmet_required_case_ids` is empty. All three `connections.json` files record `state=stopped` after harness cleanup.
- PG16 artifact `target/integration/my2pg-mysql80-pg16-2021a0cf8925/`, test log SHA-256 `75ed1387f739d7b2d38728645dc2cc413a0edb760ca568e733961ca097f2296d`; PG17 artifact `target/integration/my2pg-mysql80-pg17-ba6425f997a1/`, log SHA-256 `a4041923e309242a6df4b7d78da403d317d3a64c998a660e3f559777268f5ffb`; PG18 artifact `target/integration/my2pg-mysql80-pg18-c476d96ca843/`, log SHA-256 `7783f9f1e2a77f5649ae75b5f01ed6f13a8b94a4ec97e2af5c49bbc5b9734ee9`. Each run used Cargo.lock SHA-256 `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23` and test binary SHA-256 `0b527485d354a391ddee85501d5a6ff02111044e9cd5bd211c14e5623959cc74`.
- `python3 tests/support/matrix.py --check --all` still fails closed during pin validation before database execution; this lane-registration change does not resolve the separately documented MySQL 5.7/runner pin gap.

This closes only required-case registration and these three source/target runtime cells. T11's wider structure, existing-target policy, and identity coverage and T22's full matrix remain open.
