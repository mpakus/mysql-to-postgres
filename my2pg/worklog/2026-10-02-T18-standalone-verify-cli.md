# 2026-10-02 — T18 — standalone saved-run verification

- Status: in_progress
- Agent/role: coordinator / I
- Task and specification links: [T18](../docs/agent-tasks.md#task-board), [CLI workflow](../docs/config-and-cli.md#commands), [verification contract](../docs/architecture.md#verification-contract)
- Workspace/worktree: `/Users/renatibragimov/www/pg/my2pg`
- Base revision and tested revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` / pending
- Dependencies and their tested revisions: T11, T13, T15 and integrated T18; current T18 native evidence covers MySQL 8.4 with PostgreSQL 16 and 18
- Claimed files: `src/cli.rs`, `src/main.rs`, `src/model.rs`, `src/pipeline/mod.rs`, `tests/cases/t18_pipeline.rs`, `tests/contracts/report.json`, `docs/config-and-cli.md`, `docs/architecture.md`, this worklog

## Intended result

Make the documented `my2pg verify CONFIG --run-dir PATH [--mode counts-and-schema|content]` command execute a read-only comparison against an explicitly selected completed run's saved plan and report. Reject missing, malformed, mismatched, incomplete, and unsupported artifact/mode combinations before scanning. Return a nonzero code for differences or unsupported verification. The operator must keep the source dataset unchanged since the run; the command cannot restore the original MySQL snapshot. An append run's row-count baseline is not durable, so standalone counts/schema verification must surface the existing unsupported result; content verification for a nonempty append target remains unsupported by the T18 contract. The run's integrated verifier remains authoritative for its original `single_snapshot` and append baselines.

## Reference research (before coding)

| Problem/query and prefix | Repository commit and file/line | Source/test behavior observed | Adaptation or rejection; our acceptance test |
| --- | --- | --- | --- |
| `verify command previous run report artifact plan integrated verification`; `project-my2pg` (XERJ index revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`) | Current project: `src/main.rs:101-145`, `src/cli.rs:29-65`, `src/pipeline/mod.rs:1748-1802`, `src/report/mod.rs:70-90`, `src/model.rs:522-539, 813-863`, `src/verify/mod.rs:35-150`; `tests/cases/t18_content.rs`, `tests/cases/t18_pipeline.rs`, `tests/cases/t15_verify.rs`, `tests/contracts.rs` | CLI parses `verify` but dispatch returns `FEATURE_PENDING`. Run artifacts persist `plan.json` and `report.json`; report does not persist append count/content baselines. Live verifiers require plan, per-table reports, and baseline; the content verifier refuses a later `single_snapshot` scan. Existing tests exercise verifier APIs and pipeline, not CLI dispatch. | Add an explicit run directory to bind verification to saved accounting; validate pair, completed status, plan/report consistency and schema/source identity; keep all scans read-only. Do not infer a latest run or claim historical snapshot proof. Test parser/dispatch inputs and native exact-content/count behavior, plus malformed/mismatched/incomplete artifact refusals and append limitation. |
| `read only repeatable read transaction snapshot count verification`; `ref-rust-postgres` pin `1084ca8f5b5302e161892f2fa40abf71b4060c10` | `tokio-postgres/src/transaction_builder.rs:45-104`; adjacent test `tokio-postgres/tests/test/main.rs:630-660` | Builder composes isolation and read-only access mode into `START TRANSACTION`; test exercises transaction builder behavior. | Reuse the existing content verifier's read-only repeatable-read target scan; no new transaction abstraction. Verify target is unchanged in the CLI native case. |
| `consistent snapshot read only transaction verify rows`; `ref-mysql_async` pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5` | `src/queryable/transaction.rs:124-182`; repository tests/source search for transaction setup and cleanup | Driver establishes read-only/consistent snapshots explicitly and rolls back an abandoned transaction on drop. No saved snapshot can be reattached by a later CLI process. | Reuse `verify_content`'s current source snapshot behavior; explicitly document that a later command requires unchanged source data and cannot certify the original snapshot. |
| Follow-up after independent review: `durable report plan digest run id database endpoint identity artifact binding`; `project-my2pg` at XERJ revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` | `src/plan/mod.rs:24-30`, `src/pipeline/hooks.rs:21-26`, `src/pipeline/mod.rs:664-680`, `src/model.rs:840-863`, `tests/contracts.rs:1-15`; reviewer evidence in the T18 review message | The project already uses SHA-256 for stable identifiers/hooks. A report currently lacks a plan digest and endpoint identity. The review found that a same-version database at another host could pass and that plans from different runs could pair if table identities happen to match. | Persist SHA-256 of compact serialized plan plus password-free canonical MySQL/PostgreSQL URLs in the run report, then require exact matches before DB scans. Add contract tests for new fields and refusal of legacy/unbound reports. This keeps credentials out of artifacts and binds both files/endpoints without adding a dependency. |
| `Config get_hosts get_dbname get_user password connection identity`; `ref-rust-postgres` pin `1084ca8f5b5302e161892f2fa40abf71b4060c10` | `tokio-postgres/src/config.rs:181-205, 285-314, 388-429`; adjacent parser tests `tokio-postgres/tests/test/main.rs:630-660`, `tokio-postgres/tests/test/parse.rs` | The client exposes separate parsed host, database, user, and password fields; its URL syntax permits multi-host and query options. | Reject driver-specific endpoint reconstruction: my2pg already rejects those PostgreSQL URL forms and validates a single network endpoint. Hash the validated URL after removing its password. |
| `Opts get_ip_or_hostname get_tcp_port get_db_name connection endpoint`; `ref-mysql_async` pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5` | `src/opts/mod.rs:829-894`; adjacent test `src/opts/mod.rs:2278-2300` | The MySQL driver exposes host, port, user, and database separately and tests equivalence between URL and builder options. | Keep endpoint hashing in the shared pipeline from already validated credential URLs, avoiding dependency-specific identity structs and never serializing the password. Test identical URL stability and host/database/password sensitivity. |

Search limitations: lexical project-index results describe the currently indexed checkpoint and do not represent unindexed working-tree changes. No equivalent prior-run verifier or artifact binding was found in the pinned references. `rtk` is available at `/opt/homebrew/bin/rtk`; the required Ponytail guidance was read at `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`. No RTK `SKILL.md` exists in the documented active skill roots; the project command wrapper is used for all shell operations.

## Changes

- `src/cli.rs` adds required `--run-dir` and retains optional verification-mode override; `src/main.rs` loads only the selected run's plan/report and dispatches verification with structured JSON/status codes.
- `src/pipeline/mod.rs` validates complete table accounting, schema/consistency, exact plan digest, and password-free fingerprints of the source and target URLs before connecting. It then calls the existing read-only count/schema or content verifier. The configured password is deliberately excluded from endpoint identity so credentials can rotate.
- `src/model.rs` and `tests/contracts/report.json` add the plan and endpoint SHA-256 fields to durable reports. `tests/cases/t18_pipeline.rs` exercises a successful actual CLI scan, target row-count stability, wrong endpoint refusal, tampered plan refusal, and the append unsupported outcome. `src/cli.rs` and `src/main.rs` have parser/artifact-pair unit coverage.
- `docs/config-and-cli.md` and `docs/architecture.md` now describe explicit run selection, endpoint binding, unchanged-source expectations, and append/snapshot limits.
- The coordinator refreshed `project-my2pg` through XERJ after these changes: 384 files, 1345 passages, 11 exclusions. A follow-up XERJ query returned the digest/fingerprint implementation at the refreshed project index revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.

## Verification

| Check | Exact command or artifact | Environment/revision | Result |
| --- | --- | --- | --- |
| Relevant unit tests | `bin/cargo test --offline --locked` | current workspace; 2026-10-02 | pass: 126 library tests, 1 binary test, 15 contract tests, 1 driver test, 18 importer tests, and 13 ordinary integration tests; 137 DB-only integration tests ignored by default |
| Focused native database/CLI check | `set -a; . target/integration/my2pg-mysql84-pg16-7cf9619fb5a9/env.sh; set +a; bin/cargo test --offline --locked --test integration content_verification_is_reported_by_the_real_pipeline_and_append_includes_baseline -- --ignored --nocapture` | MySQL 8.4.11 → PostgreSQL 16.15, linux/arm64; MySQL image `sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242`, PostgreSQL image `sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea`; fixture SQL hashes `efbe952f0397182d11caf187cc806b741abd642799a6d7ad8e554c3bf49f7c56`, `af1413b85316897eb0a81f249713131fddc929f515d21840b44cb835b1c5e504` | pass: successful `verify` JSON; target row count unchanged; wrong endpoint and plan tampering rejected before database access; append counts return `unsupported` with exit 4 |
| Formatting/lint | `bin/cargo fmt --all -- --check`; `git diff --check`; `bin/cargo clippy --offline --locked --all-targets -- -D warnings` | current workspace; 2026-10-02 | pass |
| Expected-output review | `bin/cargo run --offline --locked --bin my2pg -- verify --help`; actual binary exercised by T18 native case | current workspace; 2026-10-02 | pass: `--run-dir` required, supported mode names printed, verification report carries operation/run ID/status |

## Decisions and deviations

An independent review first found missing endpoint binding and missing plan/report binding. The follow-up added both hashes and native wrong-endpoint and edited-plan refusal assertions. The reviewer rechecked the final implementation and found no remaining actionable issue.

## Limitations or blocker

The standalone verifier cannot reattach to the source snapshot that existed in the original process. It cannot perform complete verification of a nonempty append run because neither the row-count baseline nor exact content baseline is durable. The integrated run verifier retains both baselines while they are valid.

## Handoff

- Completed acceptance cases: explicit successful-run verify; saved-plan tampering refusal; endpoint drift refusal; no target row changes; append count scope refusal; password rotation preserved by unit test of endpoint identity.
- Artifacts and digests: native lane evidence under `target/integration/my2pg-mysql84-pg16-7cf9619fb5a9/`; fixture state is stopped and ownership metadata was retained.
- Files/revision ready to integrate: current working tree; no commit created.
- Remaining work: other M2/M4 acceptance gates remain open on the task board.
- Next task/owner: coordinator integrates and updates T18 task/checklist evidence

## Review

- Reviewer: `t13_review` (independent design review)
- Findings and resolutions: both initial findings were resolved with stored plan and endpoint digests; final review had no actionable finding
- Task-board update/reference: pending
