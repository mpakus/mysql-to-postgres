# T12 — independent runner integration review

Review date: 2026-10-01. Scope is read-only `src/pipeline/mod.rs`, `tests/cases/t12_runner.rs`, and the adjacent recovery/transport/artifact contracts. Only this worklog was changed. The coordinator owns fixes, red/green proof and the fresh full database lane; this review does not claim a new runtime pass.

## Research and reviewed revision

Read `docs/reference-coding.md`, architecture COPY/recovery/cancellation/report requirements, the T12 implementation/evidence worklog, current runner, the complete T12 runner cases, `src/pipeline/recovery.rs`, PostgreSQL COPY transaction code, `src/report/mod.rs`, and report/event models. Used RTK and Ponytail guidance already read for this task family.

Project HEAD is `66c92db30cc1ba0a3ecdd9c0ac3ae4fe253f270f`; reviewed T12 integration is a working-tree change, not wholly represented by that commit. `rtk git -C my2pg diff -- src/pipeline/mod.rs tests/cases/t12_runner.rs` showed the tracked runner integration; the new case file was read directly, including the newly added actual reject-metadata collision test.

XERJ queries before findings:

- `project-my2pg`: `RecoveryProgress committed_bytes rejected_rows flush` found the observer, metrics/report model and neighboring contracts at the project revision above. Original runner and artifact files were then read.
- `project-pgloader`: symbol `retry-batch`, pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; read `src/pg-copy/copy-retry-batch.lisp:61–135`, compared left-first good-prefix commits and permanent row rejection. Its broad retry classification does not replace our confirmed-rollback/safety guard.
- `ref-rust-postgres`: symbol `copy_in_error`, pin `1084ca8f5b5302e161892f2fa40abf71b4060c10`; read `tokio-postgres/tests/test/main.rs:735–761`. Dropped COPY inserts nothing in its transaction; this establishes no uncertain-COMMIT replay guarantee by itself.

No reference code was copied, no shared index/pin was changed, and no production or test file was edited.

## Finding

**P2 — final console I/O can erase the run's indeterminate COMMIT status.** At the reviewed `src/pipeline/mod.rs:469–484`, an outcome/finish error unconditionally assigns `report.status = RunStatus::Failed` and persists the replacement report. If cancellation or connection loss already classified an attempted COMMIT as `Indeterminate`, the first final report contains that status, but a closed output pipe replaces it with `Failed`. The table's indeterminate count and prior diagnostic remain, yet the authoritative run status loses its most significant uncertainty. The neighboring reject-flush branch at `:453–456` correctly preserves `Indeterminate`.

Reported to the coordinator immediately. Small correction: preserve `Indeterminate` while appending `CONSOLE_IO`, still persist the final report and return nonzero I/O failure. Independent proof should combine the existing actual deferred-COMMIT cancellation fixture with a closed CLI output pipe, then inspect `report.json` directly; merely setting a mock target error cannot prove COMMIT timing. This branch predates T12 integration, but T12's active-slice uncertainty path reaches it. No automatic replay was found in this branch.

## Reviewed invariants

| Surface | Evidence and conclusion |
| --- | --- |
| ACK deltas before next await | Recovery calls the synchronous observer immediately after `copy_bytes` returns a commit ACK. The observer adds cumulative batch progress to captured table baselines, updates metrics/state and durably replaces the report before the next recovery network await. If report writing fails, the in-memory ACK counts remain for final failure reporting. No counter is credited at COPY finish alone. |
| Durable reject deltas | Conversion rejection reserves a global ticket, durably writes the raw-value record, commits the ticket, then updates the table count. COPY rejection writes exact bytes plus durable metadata, commits its ticket, and notifies the observer. A failed write does not count a durable reject, and its uncommitted ticket refunds admission. |
| Shared mixed cap | One `RejectBudget` is constructed for the entire `execute` call, outside the table loop. Conversion and server recovery use this same budget. Baselines for rejected counters are captured per batch, so earlier conversion rejects and previous batches are not counted again. The exact-cap/next-reject cases assert actual rows, total cap, one unaccounted next row and absence of a phantom reject file. |
| Active-slice uncertainty | Before each actual subrange attempt, observer stores that subrange's `active_rows`, selected table index and COPY phase. Confirmed rollback resets active rows; uncertain/failing rollback returns while preserving the active slice. Signal handling consults the independent target stage and bounds indeterminate rows by the current active count, leaving other already-read rows unresolved. Actual blocked-COPY and deferred-COMMIT cases assert preserved ACK=2/reject=1 and classify only the tail row. |
| No replay after uncertainty | Runner invokes the recovery module once per immutable batch. Recovery does not bisect/retry unless target stage is Idle, the error has no failed-rollback cause, its execution stage is eligible, and its type/code/safety proof permits replay. Runner has no reconnect or second migration invocation after a recovery error. |
| Byte metric range mapping | The observer's `first = previous.committed + previous.rejected` indexes encoded rows only, not source ordinals. This is correct because current recovery is sequential depth-first, left before right: successfully committed and rejected rows form a consumed encoded prefix. The captured table reject baseline excludes earlier conversion rejects from this index. Delta rows describe one newly acknowledged contiguous subrange, whose exact offset length is added once. The fixture's escaped UTF-8 value and conversion gap assert 23/7 bytes; blocked recovered-tail cases assert 16 bytes. |
| Prefix preservation on artifact error | ACK/reject counters and bytes are updated before report writes. The new metadata-collision case locks initial COPY, injects an exclusive-path collision, then observes two already ACKed rows, zero durable rejects, two unresolved rows and an unchanged collision sentinel. Final report is explicitly persisted despite poisoned `flush`. |
| Pre-write safety | In reject mode, every selected existing target (except intentionally recreated targets) is inspected before `execute`, artifacts, TRUNCATE or COPY. Runtime recovery rechecks actual behavior for each batch after preparation. The custom trigger/side-effect sequence fixture proves preflight refusal preserves the sentinel and never calls the sequence. Stop-mode transaction retry safety remains inside recovery before replay. |
| Poisoned flush finalization | `RunArtifacts::flush` reports a previously poisoned writer even if no reject file entered its map. Runner appends `REJECT_ARTIFACT_IO`, preserves Indeterminate, writes the failed final report and renders its outcome. This fixes the old early return that would have left only an incomplete running snapshot. No successful finalization is claimed for a failed reject writer. |

## Limits and follow-up

The byte-prefix mapping deliberately depends on current left-first serial recovery within each table. T13 may run different tables concurrently, but must preserve each table's cumulative progress order or carry explicit committed ranges; a cross-table sum cannot be used as an offset. Each table also needs its own stage/progress registry; the current single-table record is valid only under the enforced sequential gate.

The metadata collision proves an actual filesystem failure at reject initialization and poisoned finalization. It does not prove physical ENOSPC, metadata `sync_all` failure, or loss of the final report's filesystem. Existing module cases cover other COPY/no-replay faults; this read-only review did not repeat them while the coordinator's fresh harness was running.

The current tests independently inspect target rows, exact escaped data, durable reject records/offsets, table counters, status/exit, private file modes and stored-vs-emitted report identity. They do not merely snapshot generated SQL. Cross-table parallel cap races and full/empty queue shutdown remain T13's acceptance surface.

T13 preparation remains ready. Coordinator clarified that `index_workers > 1` belongs to the full T13 scope and must use separately bounded post-copy finalization; it must never be accepted silently. No T13 implementation lease has been granted yet.

- [x] Complete read-only integration and adjacent-contract review.
- [x] Report concrete finalization finding to coordinator.
- [ ] Coordinator fixes and proves indeterminate status preservation on console failure.
- [ ] Coordinator records red/green and fresh full-lane evidence.
