# 2026-10-01 — existing policies, sequences and owned shutdown

Source checkpoint: `d6d00e7df488b532621197e3ce5abbb9fa185e31`. The coordinator reviewed and integrated the frozen worker handoffs using RTK/Ponytail and reference searches recorded before code. This is a passing development increment; T10/T11/T12/T13/T15/T16 and M2–M4 remain open.

## Integrated changes

- T11: coherent read-only multi-namespace inspection, positive namespace/type/default/index/FK/constraint observations, compatible enum reuse and selected-object dependency closure for existing policies. Recreate removes positively owned objects; append/data-only preserves supported existing objects. [Inspection](2026-10-01-T11-multischema-inspection.md), [existing policies](2026-10-01-T11-existing-policies.md), [policy completeness](2026-10-01-T11-policy-completeness.md).
- T11: monotonic live BY DEFAULT identity adjustment rechecks ownership/options/privileges and current/loaded/source bounds under the selected-table lock. Reset=false records and verifies the exact preserved state, with no claim that future generation is ahead of imported rows. `setval` remains nontransactional and is never automatically replayed. [Sequence proof](2026-10-01-T11-monotonic-sequences.md).
- T11/T12: supported user constraint triggers and ordinary extra deferred UNIQUE constraints retain their observed identities and timing. Required source keys still require immediate semantics. Review added selected namespace/name proof for deferred keys; damaged metadata fails closed. Enabled user-trigger/custom-side-effect recovery refusal remains in place. [Trigger proof](2026-10-01-T11-constraint-trigger-preservation.md), [deferred keys](2026-10-01-T11-deferred-key-preservation.md), [integration review](2026-10-01-T11-deferred-key-integration.md).
- T13: first-fault publication precedes sibling stopping, and an unread source backend stays registered through the coordinator's actual cancellation attempt after a worker panic. Blocked control/verification/index cancellation retains acknowledged prefixes. The native test observes the real binary-protocol Sending-to-client backend and owned cleanup. [Panic ownership](2026-10-01-T13-panic-source-ownership.md), [shutdown phases](2026-10-01-T13-shutdown-phases.md).
- T15 preparation: a separately tested bounded child artifact writer owns filesystem calls, strict durable receipts, retained payload permits, immutable report generations and actual child reaping. It is **not wired into the migration runner**. Reservation/admission, CLI child dispatch, async pipeline/recovery ownership and full migration fault acceptance are still required. [Protocol](2026-10-01-T15-artifact-protocol-contract.md), [worker implementation](2026-10-01-T15-artifact-worker.md).
- T15: safe COPY error chains retain SQLSTATE/stage, including the original failure behind a replay-safety refusal, without emitting raw server exception messages. [Diagnostics](2026-10-01-T15-copy-diagnostics.md).
- T10 investigation: two native limitation assertions confirm SHOW CREATE/binary metadata/recursive-CTE approaches do not recover supplementary ordered ENUM/SET labels. No production workaround or source DDL was added; safe refusal and the open fidelity requirement remain. [Evidence](2026-10-01-T10-label-recovery.md).

## Validation at frozen inputs

| Check | Observed result |
| --- | --- |
| `bin/cargo test --locked --all-features --tests` | **126 ordinary passes**, 0 failures:86lib,14contracts,1driver,18importer,7integration. Database/NLS ignores remain explicit. Includes2pure artifact checks and2process cases. Log: `target/final-ordinary-increment.log`. |
| `tests/run-integration.sh mysql84 pg16` | **121 native passes**, 0 failures:3explicitly invoked ignored library,11driver,107integration. Oracle MySQL8.4.11/PostgreSQL16.15, native arm64, verified TLS. Fresh owned pair `a53e6379027d`; stopped in finally. |
| Initial full native run | Pair `be7826773d02`:118passes,2failures. Blanket deferred-key refusal prevented two unchanged recovery tests from reaching COMMIT. Retained original logs; pair stopped. Fixed the production preservation policy, added positive damaged-link/duplicate-COMMIT proof, and reran the full suite. |
| Focused deferred-key correction | Six native cases and23planner unit cases pass. Both original CLI tests reach real locked COMMIT with ACK2/reject1 retained and only the active tail indeterminate1. Earlier0-test filtered invocation is not counted. Worker strict all-target/all-feature Clippy passed after final code changes. |
| Formatting/diff | Coordinator `bin/cargo fmt --all -- --check` and `git diff --check` pass at the final frozen inputs. |
| Documentation paths |229 local Markdown file links under docs/worklog resolve; anchors and remote URLs were not checked by this path check. |
| Development RSS observation | Fixed708,112 application bytes; two tables each20,000→200,000rows; sampled process peak26,528→26,864KiB,429/4,143samples, normal source/target peaks3/3. Driver/TLS/allocator overhead is included in RSS. This is not a benchmark or hard RSS ceiling. |

Full-lane commands, fixture/harness hashes and case IDs are saved in `target/integration/my2pg-mysql84-pg16-a53e6379027d/{rust-tests.json,rust-tests.log}`. Full log SHA-256: `018324c2b03802e303b6c5ec089cd6987a3d5928adfd691c8d5a1812b1bad1b2`. Cargo.lock remains `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`.

Execution began at HEAD `2b0573083761aa664c195514944459d1d595dbfc` with dirty source. Before/after fingerprints match across178 executable/test/config inputs, excluding Markdown/worklogs: SHA-256 `44acfa213d2ad9da95f99fc91e2e345afac0705889a3f08ecfeabf59f8db1007`; per-file inventory in `target/increment-final-inputs.json`. The source checkpoint preserves those tested bytes. Prior separate NLS evidence belongs to `bd76d81`; NLS and the full version/platform matrix were not rerun here.

## Remaining work and handoff

All workers are frozen and their owned fixtures are stopped. The next integration seam is the artifact writer: reserve A before target mutation, derive table admission from `(M-A)/P`, move existing reject/report operations into the async receipt/ownership path, reconcile confirmed receipts after cancellation, and prove bounded shutdown with the actual migration runner under blocked filesystem operations. Synchronous process startup and console output need explicit review; isolated child tests do not prove every runner deadline.

T11 still needs guarded target preparation with locks/reinspection, complete cross-schema execution, serial/ALWAYS generation and separately explicit data-only sequence-reset intent. T13 SingleSnapshot external-MDL server cleanup remains unresolved under the strict one-connection contract. T10 lossless supplementary labels, T12 physical storage-sync proof, T15 complete persistence/verification, T16 importer-driven native compatibility and T17–T24/full matrix/content/ranges/performance/packaging/release review remain open. No grouped checklist closes at this checkpoint.

The coordinator saves documentation before refreshing the project XERJ corpus. Final coverage, exact passage comparison, pin cleanliness and representative-query evidence belong in `.reference-coding/reports/verification.json`, outside indexed content. Later edits require another coordinated refresh.

The staged documentation whitespace check found one trailing blank line in the artifact protocol worklog after the source checks passed. Removed it in a subsequent documentation-only correction and reran the staged check; executable/test inputs are unchanged. The project index is rebuilt again after that correction.
