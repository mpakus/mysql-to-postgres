# T13 — bounded table workers and shutdown

Status: resource seam implemented and handed off; real runner/index/shutdown integration and RSS gates remain pending. Research was recorded before code. The coordinator owns `src/pipeline/mod.rs`, shared models/reports, root registration, and index refreshes.

## Reference research

Read `docs/reference-coding.md`, `scope-and-compatibility.md`, `architecture.md` (consistency, streaming, COPY recovery and cancellation), `agent-tasks.md`, `checklists.md`, `config-and-cli.md`, and `testing-and-performance.md`. Applied RTK guidance at `/Users/renatibragimov/.claude/RTK.md`, Ponytail at `/Users/renatibragimov/.claude/skills/ponytail/SKILL.md`, and codebase-design at `/Users/renatibragimov/.agents/skills/codebase-design/SKILL.md`. Use existing Tokio/futures facilities and concrete transports; no new queue library, connection-pool abstraction, or hypothetical source interface.

Research used `bin/reference-search.py` against the local lexical XERJ endpoint. The coordinator completed the latest refresh before renewed queries: project 168 files/545 passages; seven corpora 2,296 files/7,183 passages; 14 expected-source checks passed. Queries paused during that rebuild. Project revision is `66c92db30cc1ba0a3ecdd9c0ac3ae4fe253f270f`; the indexed/read working tree contains subsequent changes, so this hash alone is not a claim that all current files were committed.

| Prefix and query | Pin / original files read | Finding and adaptation |
| --- | --- | --- |
| `project-my2pg`: `queue_depth memory_bytes connection semaphore shutdown`; `memory_bytes admission queue connection`; renewed `queue_batches memory_bytes single_snapshot` | Project revision above; `src/pipeline/mod.rs:131`, `:496`, `:573`, `:810`; `src/config/validation.rs:181`; `src/model.rs:379`; adjacent `tests/cases/t09_cli.rs`, `t12_runner.rs`, and `tests/driver_spike.rs` | Current runner owns one reader/writer, global byte semaphore, immutable batch permits and synchronous recovery progress. It rejects all parallel settings and has one active-table cancellation record. Reuse COPY/recovery rather than introducing another executor. Replace its single-table state with per-table ownership in the coordinator's integration. |
| `ref-dmt-rs`: `bounded WriteJob channel cancellation select`; `cancel writer_handles reader_handle try_join_all`; renewed `bounded WriteJob` | `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; `crates/dmt-rs/src/transfer/mod.rs:364–680`, tests `:1469`; `scripts/test-signals.sh` | Bounded read/write channels, cancellation-aware blocked send, immediate sibling cancellation on writer failure, and explicit channel closure. Adopt these behaviors, with byte permits in addition to queue slots. Do not adopt multiple sources, OFFSET scans, binary COPY, staging/resume, or its range tracker. Adjacent tests prove range bookkeeping, not our full/empty-queue shutdown. Its `try_join_all` writer aggregation does not establish our requirement that every spawned task is joined after the first failure. |
| `ref-mysql_async`: `pool max disconnect cleanup cancellation` | `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`; `src/conn/pool/mod.rs:121–166`, `:330–444`, adjacent pool/reset tests | Pool disconnect waits for active borrowers; ordinary unpooled driver drop can schedule unread-result cleanup. Keep the already-tested concrete `SourceConnection` guard and explicit cancellation rather than adding a pool. Every admitted source connection receives the same validated session setup. |
| `ref-rust-postgres`: `cancel_query abort connection`; symbol `cancel_query` | `1084ca8f5b5302e161892f2fa40abf71b4060c10`; `tokio-postgres/src/cancel_token.rs`, `cancel_query.rs:1–65`; adjacent COPY/transaction tests already read for T07/T12 | CancelToken opens a separate TCP/TLS socket and has no server acknowledgment. Reserve cancellation capacity independently of normal connection admission; perform cancellation concurrently. Retain the concrete atomic COMMIT/cancel barrier and treat an attempted uncertain COMMIT as indeterminate. |

Queries located useful implementations but do not substitute for independent my2pg runtime cases. No reference code or fixture is copied. dmt-rs declares MIT in Cargo metadata but has no root license file at the pin; mysql_async and rust-postgres are read-only pinned references. Provenance/licensing remains recorded by the repository manifest and T23.

## Proposed smallest seam

The proposed new `src/pipeline/scheduler.rs` supplies checked resource admission and cancellation-aware batch ownership. The existing runner keeps source-first preflight, schema lock, DDL, verification, artifacts and console ownership. A per-table transfer uses one existing source guard, one target guard, one bounded `tokio::sync::mpsc` queue, and the existing recovery function. Do not create a parallel migration engine or raw-row queue.

Proposed concrete resource interface, subject to coordinator review:

- `Admission::new(config, selected_tables)` computes checked batch/workspace reservations, effective active table count and normal connection bounds before writes.
- `Resources::admit(cancel)` acquires a table/connection-pair permit before either network connection opens. Its lease holds the row workspace reservation for the producer lifetime.
- `Resources::batch(cancel)` returns the existing owned global-byte permit. `BatchStorage` retains it across queued, active COPY and recovery ownership. Queue helpers may remain ordinary Tokio `select!` at the root integration rather than wrappers if that is shorter.
- Root records effective table admission in a structured diagnostic/event and manages a bounded collection of per-table futures. No new TOML key, dependency, shared model or serializable credential field is required initially.

This proposal deliberately does not assign exact Rust signatures before root agrees how transfer contexts and progress are borrowed. `ResolvedCredentials` is not Clone; avoid rereading credentials per worker or adding public serialization merely to satisfy `tokio::spawn`. A bounded collection of scoped `FuturesUnordered` futures can borrow resolved contexts and cannot detach. If multithreaded Tokio tasks are selected, define a private owned, redacted connection context and explicitly join/abort every task. Concurrent asynchronous I/O is not evidence of CPU-parallel conversion or a speed improvement; T21 measures that separately.

### Memory admission

Let `B = batch_bytes + batch_rows × size_of::<RowPosition>()`. Current row locators have no heap key; any future populated key must be budgeted. Reuse the current audited workspace `W = 3 × max_row_bytes + encoder_workspace_bytes(max_row_bytes) + 65,536`, covering raw/converted enum storage, encoded lookahead and pinned encoder scratch.

For queue depth `Q`, reserve progress capacity `P = W + (Q + 2) × B` per admitted table: at most Q queued batches, one active COPY/recovery batch and one filling or blocked-send producer batch. Compute `A = min(table_workers, selected_tables, floor(memory_bytes / P))`; reject a nonempty plan if A is zero, or arithmetic/individual semaphore permit conversion overflows. Empty plans need no table lease.

At most A producers acquire W and batch reservations from the global semaphore. Because `A × P <= memory_bytes`, every admitted producer can make progress even when every queue is full and every writer retains a retry batch. This avoids admitting many producers that each retain raw data while waiting on bytes held by one another. Never acquire a lease for the next table while retaining the previous table's buffers. Do not silently reduce Q to pass admission. Reducing effective A under the configured global byte limit must be visible.

The offline validator currently uses a smaller fixed minimum; root must align its preflight diagnostics with the runtime formula or explain the stricter runtime rejection before network/mutation. Retained batch/workspace bytes are an application allocation bound, not a hard RSS claim. Catalog metadata, allocator capacity/rounding, driver packet/caches, TLS, task stacks and filesystem buffers remain measured overhead. T03 oversized raw/packet behavior is already proved; T13 must measure a many-batch plateau, not merely inspect semaphore availability.

### Consistency and connections

`single_snapshot` remains exactly one source reader and reuses the original snapshot connection for all reads and source verification. Source tables can be streamed sequentially while the distinct target consumes a bounded queue. Never reconnect or allocate another source reader into that transaction.

`frozen` allows A concurrent table transfers, each with one reader and one writer; it remains an operator write-freeze assertion. Control/preflight/advisory-lock connections are separate. A simple upper bound is A+1 normal connections per database; leases cover connection initialization and teardown as well as copying. An explicit pair permit avoids acquiring one half while waiting indefinitely for the other.

PostgreSQL cancellation can temporarily add at most A+1 separate sockets if control operations also cancel. Normal admission must not consume cancellation capacity. Source guards close unread results without draining; target guards own and abort protocol tasks. All session/TLS/passfile behavior stays in the existing connection implementations.

`readers_per_table > 1` remains T20 range scheduling. The coordinator must decide whether separately bounded `index_workers > 1` is part of this T13 implementation or an explicitly unfinished finalization gate. Do not silently accept a setting that has no effect.

### Progress, recovery and shutdown

One run-global `RejectBudget` is shared across conversions and server-side recovery. Reject persistence retains its concrete durable-write interface. Recovery retains a single immutable batch/permit while replaying slices; there is no extra queue copy and no replay after failed rollback or uncertain COMMIT.

The root's current single `ExecutionState` cannot identify several active COPY stages. Each active table needs its own stage/shutdown handle, cumulative read/ACK/durable-reject counters, active range and elapsed timing. A synchronous recovery callback cannot await a full progress queue: update authoritative per-table counters synchronously and notify the sole report/console manager with a coalesced wakeup. Never hold a progress mutex across network or disk I/O. Final snapshots must retain all ACK/reject deltas even if cancellation outruns a notification. Reports are evidence, not exactly-once resume checkpoints.

First original failure stops new admission and requests global stop immediately. Close/drop channels to wake both blocked sides; derivative channel-closed errors must not replace the source/target/artifact failure. A panic is a real worker failure, sanitized without credentials. Retain each active target's atomic stage barrier independently so only a table whose COMMIT was attempted becomes indeterminate; queued/unsubmitted work remains unresolved.

Mark all cancellation barriers before awaiting network, then cancel active targets concurrently. `ShutdownHandle::cancel` currently marks at its first poll and uses a three-second request timeout before aborting the protocol task. A collection of concurrent cancellation futures avoids A sequential timeout periods. Root may need a barrier-only method if concurrent first polling cannot establish the required ordering. Do not add it without a verified need and transport lease.

Use one documented global shutdown deadline, await every owned worker/driver cleanup, and force-abort remaining async tasks at expiry. Do not claim a hard wall-clock deadline for an OS filesystem call that blocks synchronously; preserve existing second-signal uncertainty semantics. No new admission is allowed while canceled buffers/connection guards are being released. Do not reconnect a failed source or replay an uncertain target batch.

## Independent acceptance design

| Case | Observable oracle |
| --- | --- |
| Checked minimum / overflow | Reject P−1 before writes; accept exact P for one pipeline; overflow and u32-permit overflow produce configuration failure, no connection. |
| Memory-limited table admission | Request several tables with memory for only one/two; observe the reported effective count, never more active leases, exact target rows and all permits returned. |
| Full queue / blocked producer | Lock target COPY so Q queues fill plus active/filling batches; verify source progress stops at the bounded prefix and retained permits never exceed budget; release lock and get exact complete data. |
| Empty queue / blocked consumer | Hold or terminate the source before its first row; cancellation/error wakes writer waiting on recv, leaves target empty, closes both backends. |
| Writer fault while queue full | Terminate active target or cause an actual permission/COPY failure; producer wakes, other table admission stops, original fault is reported, no hang or extra retry. |
| Reader fault / worker panic | Terminate actual source connection; independent Tokio worker panic case catches JoinError/unwind, closes unread socket and joins siblings. Panic text/URLs must not leak. |
| Global rejects under concurrency | Bad conversion and server rows in distinct tables race for a low shared reject cap; durable reject totals equal consumed tickets, never exceed cap, acknowledged good rows/counters are preserved. Do not require a cross-table deterministic winner. |
| Single snapshot | Update InnoDB rows externally between table reads; target/source expectations remain on the same transaction snapshot. Multiple table workers/readers still fail configuration. |
| Frozen parallelism | Block two distinct target tables and inspect two active transfers, then release and compare exact rows; one-reader-per-table and bounded connection counts hold. |
| Signal with blocked send/recv | Actual CLI SIGINT and SIGTERM at full/empty queues terminate within the documented deadline; no owned task/backend survives, artifact status is nonzero and ACK/reject prefix remains correct. |
| Concurrent COPY and attempted COMMIT | One table is blocked in COPY and another at deferred COMMIT; signal classifies only attempted COMMIT indeterminate, earlier ACK/reject prefix persists, and no batch is replayed. Reuse T12's real deferred constraint/lock technique. |
| Connection exhaustion / init cancellation | Occupy DB connection slots or use a listening stalled endpoint; initialization error/deadline stops siblings and queued admission. Cancellation has reserved socket headroom and backend disappearance is observed. |
| Many-batch memory trend | Stream data much larger than the budget with slow target; sample actual process RSS and application reservations over time, record peak/plateau plus driver/TLS overhead. No throughput claim from this fault test. |

Tests use owned/disjoint namespaces and artifact paths in the pinned Oracle MySQL8.4/PostgreSQL16 harness. Runtime prerequisites fail the mandatory lane; they are never successful skips. Keep unrelated Strangler containers intact. The root owns cleanup of its currently shared fixture; no active DB lease is claimed for this research-only task.

## Handoff checklist

- [x] Search project and relevant pinned references; read originals and adjacent tests.
- [x] Record research and proposed adaptation before implementation.
- [x] Identify root integration/model boundaries and independent fault oracles.
- [x] Coordinator approves exact interface, index concurrency scope and write lease.
- [x] Implement agreed scheduler resource seam and focused unit checks.
- [ ] Complete root runner/index/shutdown integration.
- [ ] Mandatory queue/resource/failure/signal runtime evidence, formatting and strict Clippy.
- [ ] Record resource/RSS limitations and handoff; release fixture lease.

## Renewed scheduler implementation lease — research before code

Coordinator approved only new `src/pipeline/scheduler.rs`, its adjacent unit tests and this worklog. Root retains runner integration, offline minimum validation, root registration, transports/models/reports. Separately bounded post-copy index finalization is explicitly included in full T13; index_workers is not silently ignored. Reviewed HEAD85b238bbe5b0b69154b923f73ab241fde8cdc696 and the root-reported tested source45cfca4; current working implementation is read directly. Latest verified project coverage173 files/564 passages and14 expected-source checks.

Repeated XERJ `project-my2pg / queue_batches workspace_reservation single_snapshot` found config/minimum/snapshot validation at85b238b. `ref-dmt-rs / bounded WriteJob cancellation` found the same pinned transfer path; reread original bounded channels at414–418, send/cancel select at589–615 and adjacent tests. Read existing MigrationOptions, execution_options, BatchStorage/RowPosition and Cargo dependencies. No new dependency or index/reference change is needed. This module supplies checked reservation/admission and resource RAII only; native bounded channels, executor, progress and cancellation registry remain root-owned.

Concrete adaptation: share MemoryLayout::compute between root's offline/runtime validators. Admission caps COPY workers by selected count and floor(M/P), index workers by actual index-job count, and shares a target-worker semaphore sized to max(table,index workers). Acquire full table/target/workspace permits before connection initialization; retain the TablePermit until producer, queue, recovery and both connection guards are completely torn down. IndexPermit holds separately bounded index capacity and the same target-worker capacity. Global byte permits are still retained by immutable BatchStorage; no pool, detached task, alternate executor, generic source abstraction, or copied reference implementation. Cancellation-aware permit acquisition follows the root's existing watch semantics: true cancels; an externally dropped false sender is not implicitly a signal.

Normal socket upper bounds include one control connection per DB, except single_snapshot has only the one original source reader. Concurrent PostgreSQL CancelToken sockets require independent headroom equal to at most the normal target bound; record normal plus emergency peak. Index jobs may contend for the shared target capacity safely, but root must enforce their post-copy dependency order. Application reservations do not include metadata/TLS/driver/RSS overhead, and unit semaphore checks do not close the outstanding real runner/RSS gates.

Coordinator accepted the concrete signatures. Direct reread of root `cancelled` and adjacent T08 tests confirmed that a closed false watch parks rather than cancels; coordinator corrected an initial contrary reading and explicitly preserved that existing contract. The module therefore cancels on true (including true then closed), does not cancel on closed false, biases cancellation before simultaneously ready acquisition, and releases each already acquired permit if later acquisition fails. Read pinned Tokio1.53.1 original semaphore docs: OwnedSemaphorePermit keeps permits until drop, acquire_many_owned is fair, and canceled acquisition loses its queue position. Use ordinary owned permits instead of custom byte counters.

## Resource seam handoff and focused evidence

Written only `src/pipeline/scheduler.rs` and this worklog; adjacent unit tests are contained in the new module. Root registered `pub mod scheduler` and reported successful all-target check plus **7/7 scheduler units**. Independently ran `rtk run 'bin/cargo clippy --locked --all-targets -- -D warnings'`: passed in0.73s. Owned module formatted with pinned1.99 rustfmt. No database fixture was started for this helper task; no real runner/RSS pass is claimed.

Exact API:

```rust
MemoryLayout::compute(&MigrationOptions) -> Result<MemoryLayout, ResourceError>
Admission::new(&MigrationOptions, Consistency, usize, usize) -> Result<Admission, ResourceError>
Resources::new(Admission) -> Resources
Resources::admission(&self) -> &Admission
Resources::available_bytes(&self) -> usize
Resources::close(&self)
Resources::table(&self, &mut watch::Receiver<bool>) -> Result<TablePermit, ResourceError> // async
Resources::index(&self, &mut watch::Receiver<bool>) -> Result<IndexPermit, ResourceError> // async
Resources::batch(&self, &mut watch::Receiver<bool>) -> Result<OwnedSemaphorePermit, ResourceError> // async
```

`MemoryLayout` fields are batch_reservation/workspace_reservation/pipeline_reservation. `Admission` fields are layout/table_workers/index_workers/queue_batches/memory_bytes/connections. ConnectionBounds fields normal_source/normal_target/emergency_target/peak_target include control and independent cancellation socket headroom. The target worker pool uses max(A,I); table and index admission retain separate respective capacities plus a shared target slot. TablePermit additionally retains W. Resources clones share every semaphore; close prevents new acquisitions while existing retained batches keep their owned permits.

Independent helper oracles:

- Exact P−1 refuses a nonempty COPY plan; P admits one pipeline,2P admits two, checked queue/row arithmetic and individual u32 reservation overflow refuse. Small fixture checks W=65,992 and B includes actual RowPosition storage rather than the old hardcoded locator size.
- Snapshot rejects requested parallel readers/tables and reports one original source connection. Schema-only admits zero COPY workers with a tiny byte budget but keeps real index capacity. Unsupported range readers fail explicitly.
- True-then-closed cancellation wins even when table/index/bytes are immediately available; closed false remains usable. Closing resources yields Closed, rather than consuming a new permit.
- An index monopolizes the shared target slot while a table partially owns table capacity; explicit cancellation releases that partial capacity. Failure acquiring W also releases table/target slots.
- With exact P, Q-full native channel plus active/filling batches consumes all admitted bytes. Canceling the blocked send releases its batch; queued bytes and a retry Arc alias retain permits until dropped. Table/target/W ownership persists through this cleanup and all bytes return at teardown.
- Empty recv and exhausted byte acquisition remain blocked until explicit cancel and wake promptly. No permit appears merely because the waiter is canceled.
- Three active pipelines each retain Q+2 batches at exact3P without deadlocking. Separately limited two index leases block a third even when one target slot remains free; close wakes that index waiter and retained leases preserve their slots until drop.

Caller obligations remain explicit: acquire the table/index lease before connection initialization; keep it until every associated protocol task/guard, stream, queue and batch alias is torn down; bound each table to Q+2 batch reservations. The module does not create connections, enforce DDL dependencies, or send cancellation requests. Socket upper bounds assume root issues at most one concurrent CancelToken request per owned target; root's stage registry must enforce that ownership. Root must report A reductions relative to min(requested,selected), share one RejectBudget, and preserve per-table recovery order/COMMIT uncertainty. Effective asynchronous concurrency is not a measured speed or CPU-parallelism claim.

Coordinator is adopting MemoryLayout::compute for offline/runtime validation. Independent boundary note sent: schema-only skips only M>=P, not positivity, checked arithmetic, B/W u32 or global semaphore limit. Non-schema-only runtime should retain config validation before network even when the currently selected COPY count is zero, because Admission itself intentionally permits zero-work resource sets with M<P. Existing sequential refusal remains until complete table and bounded post-copy index execution is wired.

Module lease is now frozen after handoff. Real full/empty queue faults, worker panic, connection exhaustion, concurrent reject cap, signal/COMMIT classification, backend disappearance and measured RSS remain required root integration gates; helper unit checks alone do not satisfy the T13 checklist.
