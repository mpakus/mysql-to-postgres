# 2026-10-03 — T24 — preliminary independent core review

- Status: core-path review recorded; final release acceptance remains open pending T21–T23
- Reviewer: independent Codex agent; coordinator captured the handoff because that review environment was read-only
- Reviewed HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, with a dirty source tree
- Scope: MySQL-only boundary, COPY/commit accounting, lost-ack handling and release-checklist evidence

## Reference research

The reviewer read `docs/reference-coding.md`, the scope/architecture/testing contracts, the T24 task definition, and current implementation/worklogs. XERJ searches were `project-my2pg "T24 independent release review loss duplication commit ambiguity MySQL-only performance evidence"`, followed by `ref-rust-postgres`, `ref-mysql_async`, and `ref-dmt-rs` searches for commit completion, stream cleanup and backpressure. Indexed revisions: project `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, rust-postgres `1084ca8f5b5302e161892f2fa40abf71b4060c10`, mysql_async `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`, dmt-rs `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Original files inspected include `tokio-postgres/src/copy_in.rs` and its `copy_in_error` test, mysql_async result-stream/drop implementation/tests, and dmt-rs's permit placement. These references informed review questions; no upstream implementation was copied.

## Findings

No concrete data-loss, duplicate-replay, or alternate-source defect was found in the reviewed paths. The MySQL-only input/version guard is at `src/config/validation.rs:955-969`, with server identity/version checks in `src/mysql/mod.rs:96-130`. COPY explicitly finishes, checks row count, then waits for acknowledged transaction commit in `src/postgres/mod.rs:788-875`; the pipeline tracks DDL commit phase and conservatively reports possible commit timeout as indeterminate at `src/pipeline/mod.rs:1171-1202,2974-3047`. The registered `t12_lost_commit_ack` case at `tests/cases/t12_lost_commit_ack.rs:204-354` drops a completed backend COMMIT acknowledgement, expects indeterminate accounting, independently reads exactly two committed rows, and checks that no replay occurred.

This review is targeted evidence, not a whole-codebase or release-candidate certification. It did not execute database tests. The reviewer identified these still-open gates in `docs/checklists.md:97-111`: comparative timing/correctness, large-profile and medium-binary cases, adapter equivalence and repeated runs, four pending MySQL 5.7 dispositions, hosted/native platform matrix, soak/failure breadth, qualified license/fixture-attribution review, native Linux/AMD64 distribution and artifact checksums. T24 must remain incomplete until T23 prerequisites and all required checklist gates are resolved.

The reviewer also found stale package text: checklist prose said 219 files while the current-source T23 worklog reported 220. Coordinator independently re-ran current-source package verification on 2026-10-03; updated evidence is recorded in `2026-10-03-T23-current-source-package.md`.

## Verification and limits

- Independent source review at HEAD `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; dirty-tree implementation was inspected, but no database workload or full release-candidate test was executed by the reviewer.
- No P1/P2 code defect was reported in the reviewed core paths; the principal finding is that open release gates correctly prevent a final recommendation.
- Final T24 acceptance requires a new review after the T21/T22/T23 work is integrated and fully tested.
