# M2 core integration checkpoint

Coordinator acceptance; RTK and Ponytail guidance applied. This checkpoint integrates the handed-off T10/T12/T14 modules, a first T11 structure increment and T15 terminal/metrics work. It is not M2 or release completion.

Saved source checkpoint: **`45cfca4ad71c8afec5644fa2c0452a4ea70fae85`** (`feat: integrate durable recovery and core migration fidelity`). The 57 committed files contain the exact integrated source/tests described below. Subsequent documentation acceptance updates do not alter the tested implementation.

## Accepted behavior and evidence

T10 core casts/values/defaults and semantic transformation counters are accepted. The [T10 worklog](2026-10-01-T10-values-defaults.md) records the exact numeric widths/DECIMAL65/30, finite float guards, temporal domains/precision, source charset/Unicode boundaries, ENUM ordinal and SET mask semantics, typed literal defaults and all thirteen named transforms. Independent native value assertions and actual inserted default results pass. Root's enum projection and SET-check cases, actual right-trim/remove-NUL/empty-to-NULL migrations, effect counts and exact target values cover the integration. Existing target enum/sequence policies remain T11; advanced expressions remain T17; default semantic verification remains T15.

T14 production connection paths are accepted on the development pair with independent C review. The [implementation](2026-10-01-T14-auth-connections.md) and [peer review](2026-10-01-T14-peer-review.md) record required password/current MySQL authentication, valid/wrong-host/expired/untrusted/missing client/server credentials, private passfiles including IPv6 normalization and percent escapes, secret redaction, exclusive configured CA trust, exact session readback and whole-initialization deadlines. Invalid known source settings reject before a reachable stalled endpoint accepts a socket. Nine server pairs/platform portability remain T22's separate gate.

T12 recovery is integrated through the executable: a run-global durable reject cap shared across conversion and COPY, immutable left-first subranges, confirmed-rollback SQLSTATE retry, no uncertain-commit replay, synchronous ACK/reject accounting and private durable evidence. Actual metadata initialization collision and broken final output preserve earlier acknowledgements and the correct final failed/indeterminate report. [Runner acceptance](2026-10-01-T12-runner-integration.md#integrated-checkpoint-acceptance), [module evidence](2026-10-01-T12-copy-recovery.md), and [independent review](2026-10-01-T12-integration-review.md). Physical ENOSPC/sync/report-filesystem fault acceptance and concurrency-dependent shutdown remain open; T12 stays in progress.

T11 now has a shared finite type signature parser, actual renamed composite PK/UNIQUE/descending-index/FK/comment/identity behavior in full/schema-only modes, and positive observed standalone target topology. All policies reject unproved/inherited/partition targets before artifacts or mutation; direct recovery also rechecks actual hierarchy membership. Twelve real policy/topology combinations preserve selected and unselected sentinels. Complete observed structures/sequence ownership/state, enum reuse and append/truncate/recreate compatibility remain the [next contract proposal](2026-10-01-T11-existing-catalog-proposal.md); T11 stays in progress.

T15 real stderr PTY width, redirected plain/quiet/verbose/JSON/color controls, diagnostics, unresolved/indeterminate totals and ACK-only COPY-text byte/rate metrics pass. Table events report their own counts/timing. Remaining finite supported-default comparison, full structure verification and required persistence faults keep T15 in progress. [Console evidence](2026-10-01-T15-artifacts-console.md), [shared metrics](2026-10-01-T15-shared-metrics.md), and [default preparation](2026-10-01-T15-default-verifier-preparation.md).

## Integrated checks

| Check | Result |
| --- | --- |
| Ordinary `bin/cargo test --locked --all-targets --all-features` | 85 pass: 67 library, 12 contracts, 1 driver, 5 integration; localhost peer tests require approved local networking |
| Fresh `tests/run-integration.sh` | 72 pass: 11 driver, 61 integration; all 14 seed assertions pass |
| Strict all-target/all-feature Clippy | Pass, warnings denied |
| Full rustfmt and authored whitespace checks | Pass |
| Immutable upstream fixture provenance/inventory | 13 load files, 18 parser cases, 19 SQL assertions, 65 case IDs; hashes match pinned upstream |
| Harness isolation checks | Four pass |
| Scoped runtime cleanup | Fresh `bbe7eaf5152d`, focused `0c90d81b58d2`, and earlier shared `1d68940b1228` metadata stopped; worker-owned fixtures separately stopped |

Fresh runtime is native macOS aarch64 with Oracle MySQL8.4.11 and PostgreSQL16.15. Canonical evidence paths and hashes are recorded in the runner acceptance entry. The tested parent revision was `66c92db30cc1ba0a3ecdd9c0ac3ae4fe253f270f` with the explicitly recorded working-tree changes; the coordinator's subsequent Git checkpoint contains that source. Logs retain initial failing obsolete expectations, red/green regressions and fixture corrections. Ignored database tests in the ordinary lane are invoked by the mandatory database lane, never counted as ordinary passes.

## Next owned increments

- D supplies complete observed target metadata/policy contract; coordinator publishes shared types and inspector changes once, then D implements proven existing policies.
- F implements the reviewed finite default comparator under an exact verifier/test lease; unknown expressions do not pass through exact-string equality or SQL evaluation.
- C implements the bounded scheduler resource seam; coordinator integrates per-table progress/shutdown, queues and separately bounded index finalization. [T13 proposal](2026-10-01-T13-worker-scheduling.md).
- Required M3/M4 compatibility, content verification, ranges, faults/fuzz/soak, fixed performance corpus, version/platform matrix, packaging/provenance and independent release acceptance remain intact in the task board and checklists.
