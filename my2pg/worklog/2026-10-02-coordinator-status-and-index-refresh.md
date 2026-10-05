# 2026-10-02 — coordinator — status reconciliation and XERJ refresh

## Scope

Reconcile dated M2 checkpoint notes with the current T10/T11 closeouts and T22 lane evidence. No application code changed. T11 was already accepted for its reviewed MySQL structure and target-policy subset; this update corrects stale summary prose without changing the task boundary or release gates.

## Evidence reviewed

- Read `docs/checklists.md`, `docs/agent-tasks.md`, `docs/reference-coding.md`, and the T11 accepted-structures/policies closeout.
- Current project revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- XERJ query after refresh: `project-my2pg "T11 target policy schema-only recreate sequence reset accepted boundary"`.
- Search returned the T11 closeout as the top match, followed by the older T11 audit and identity worklogs. The older audit's “T11 remains open” statement is a dated audit finding, not current status; the task board and acceptance closeout govern current status.
- Latest native artifact `target/integration/my2pg-mysql84-pg16-92362583765a/rust-tests.log` lists all 27 `t11_*` tests as passed. The enclosing full lane records 153 passed cases and all 23 required IDs exactly once in `worklog/2026-10-02-T22-mysql84-pg16-current.md`.
- The T12 DDL-failure case is included in the same lane. T12 physical-storage sync failures and T13 external metadata-lock/shutdown behavior remain open; T21, T22, T23, and T24 remain release work.

## Documentation changes

- Reworded earlier M2 checkpoints as historical evidence and pointed current status to later task closeouts.
- Updated T11 policy and identity notes to reference the integrated closeout.
- Corrected the current MySQL 8.x/PostgreSQL 16 required-case count to 23 and the full run count to 153.
- Replaced stale current remaining-scope wording with T12/T13 and T21–T24 work.

## XERJ verification

- Refreshed only the coordinator-owned `project-my2pg` source index after the documentation edits; pinned references were not changed.
- Result: `project-my2pg-source`, 352 files, 1,264 passages, 11 recorded exclusions. Coverage metadata records source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- The representative query returned the current T11 closeout first and included the expected current evidence. XERJ remains configured for lexical search; the query is a retrieval aid and original files remain authoritative.

## Validation

- `rtk run 'python3 my2pg/bin/reference-index.py project-my2pg'` — passed; 352 files and 1,264 passages indexed.
- `rtk run 'python3 my2pg/bin/reference-search.py project-my2pg "T11 target policy schema-only recreate sequence reset accepted boundary"'` — passed; current T11 closeout ranked first.
- `rtk run 'git diff --check'` — passed before adding this log.
- No Rust code or test behavior changed in this documentation-only reconciliation.

## T12/T21 closeout and current verification

- The T12 Linux FUSE acceptance case now exercises the production `RunArtifacts::write_report` on a real mounted filesystem. Its kernel fsync probe returns errno 5, report publication propagates `EIO`, the previous durable bytes remain unchanged, and the failed temporary report is removed. The exact test and local privileged ARM64 environment are recorded in [the FUSE worklog](2026-10-02-T12-fuse-eio-prototype.md). Hosted execution and broader fault acceptance remain open.
- The T21 pinned pgloader v4 adapter verifies the exact image ID, extracts the JAR from a stopped container, checks `pgloader v4.0.0`, builds loopback verified-TLS endpoints, stores credentials in private temporary files, and redacts child logs. Its unsupported resource/settings/phase comparisons force `timing_eligible=false`. No migration or performance workload was started. Details and adapter research are in [the T21 worklog](2026-10-02-T21-benchmark-runner.md).
- `cargo fmt --all -- --check` passed. `cargo test --offline --locked --all-targets -- --test-threads=1` passed with 13 passed, 0 failed, 137 ignored after rerunning with local loopback access; the sandbox-only first attempt could not bind three auth-test listeners. `cargo clippy --offline --locked --all-targets -- -D warnings` passed.
- Support suite: 24/24 passed. T21 performance runner suite: 11/11 passed. Python compilation and comparison-manifest JSON parsing passed. `bin/check-third-party-notices.sh --check` passed with Cargo's existing package-metadata warning. `git diff --check` passed.
- Remaining evidence gates include hosted Actions, hosted privileged FUSE, MySQL 5.7/other native platform coverage, NLS, fuzz/soak, residual T12/T13 failure modes, legal/provenance review, a correctness-passing resource-equivalent T21 benchmark, and T24 independent review. The task board and checklist remain open where evidence is incomplete.
- Final XERJ refresh after these evidence and checklist updates returned `project-my2pg-source`: 396 files, 1,431 passages, and 13 exclusions. Queries `T12 FUSE fsync EIO report durable bytes` and `T21 pgloader v4 adapter correctness resource timing eligible` each retrieved the relevant current worklog first. Pinned reference corpora were not changed.
