# T15 — bounded child artifact writer implementation

Status: implementation in progress; pipeline/recovery integration and physical-sync acceptance remain open. Lease: new src/report/artifact_io.rs, minimal report/mod.rs integration, new tests/cases/t15_artifact_io.rs and this log. Coordinator owns CLI/Cargo/registration/resources; C owns pipeline/recovery migration.

## Reference research before code

RTK/Ponytail guidance applied. Queries, exact pinned revisions, original implementation/adjacent test findings and chosen adaptation are recorded in [the protocol/admission contract](2026-10-01-T15-artifact-protocol-contract.md), written before source changes. Current XERJ project revision2b0573083761aa664c195514944459d1d595dbfc; dmt4e8015f7e841dbdf9df01e953aeb3948dfb3199a and pgloader231ab86778ca5ffd7de40878714760c8b4860cdf. Original Tokio1.53.1 process/from_std/drop source and serde_json1.0.151 collect_str source read. No source copying or stronger physical-sync claim.

## Implementation evidence

Facade increment ready for coordinator review; production pipeline integration and T15 acceptance remain open.

### Source/index distinction

Coordinator released the freeze after verifying dirty source SHA531b23369fae40278f051bf4df0ed035498c2c008849b723d898091f1fea62d7 at2026-10-02T01:34:13Z (2367files/7530passages,32/32checks). This code was edited afterward: that index is an earlier timestamped snapshot. Reference pins remained read-only; no worker rebuild.

### Concrete integration API

- ArtifactIdentity::new(base:&Path) creates identity/path strings without filesystem I/O.
- ArtifactLimits::derive(identity,&MigrationPlan,&RunReport,max_row_bytes) counts documents, derives payload/reservation and verifies matching table coverage. Start independently validates public structural limit fields. Reserve reservation_bytes before start and before target mutation.
- DurableArtifacts::start(std::process::Command,identity,limits,table_ids:&[String],OwnedSemaphorePermit) asynchronously creates/syncs the private directory. Root must dispatch WORKER_ARGUMENT (--internal-artifact-worker-v1) before ordinary Clap parsing to worker_stdio()->io::Result<()>. Generic serve_worker(input:impl Read,output:impl Write)->io::Result<()> supports the child helper.
- lease() acquires the sole operation permit BEFORE encoding/allocation. Consuming lease methods: write_plan, write_report(generation,report), reject_conversion(table_index,locator,RawRetention,reason,OwnedRejectTicket), reject_copy(table_index,locator,Arc<BatchStorage>,Range<usize>,reason,OwnedRejectTicket). flush() is fallible; shutdown(deadline) returns authoritative Confirmed including pending sequence and unreaped PID.
- OwnedRejectBudget::new(limit) returns the shared atomic budget; reserve() supplies a ticket, used() includes durable rejects plus outstanding reservations. Actor commits only after validated durable ACK. Never retain a second independent recovery reject budget.
- RawRetention moves Vec<RawValue> with Arc<OwnedSemaphorePermit>; actual vector/byte capacities must fit that permit, and logical source footprint fits R. COPY retains original BatchStorage/range; bytes must fit its original permit. No row/payload clone.
- Dropping a receipt waiter does not cancel accepted work. Actor publishes Confirmed table counts/offsets before waking the waiter. Integration must reconcile confirmed() even after cancellation. A pending sequence is an unacknowledged suffix, never replay authorization. Allocation-free live_worker()->Result<Option<WorkerState>> exposes PID/generation/pending sequence even when cancellation dropped start before returning its facade.

### Controlled-byte reservation

F=max(exact serialized plan, finite maximal-counter/status report template, bootstrap), including newline. Actual Rust serializer tests prove scalar maximum C142 and empty binary envelope h44. For n copied columns and R admission, b=max(0,R-n*sizeof(MySQLValue)); V=max(n*C,(n-1)*C+h+2*b)+(n-1) for n>0. D=max(F,Hcopy+F,Hconversion+V+F) across tables. Schema-only uses D=F/COPYlimit0. F supplies an explicit finite allowance for diagnostic/key content; arbitrary strings can exceed it and then fail Bounds without truncation or automatic sizing changes.

A=D+N*(sizeof(AckEntry)+sizeof(WorkerTable))+[2*(base_bytes+run_id_bytes)+6*(directory_bytes+64)]+fixed_reservation(). The fixed term is 2*16384+2*(80+48)+sizeof(Command/Life/Global/Flight/ChildSlot/Worker/DurableArtifacts)+6*sizeof(usize). Six words account for Arc strong/weak allocation prefixes of three owned control allocations. Paths include both identities and bounded child scratch paths. Checked arithmetic and u32 reservation ceiling apply. Retained rows/batches keep existing P permits; D never clones COPY bytes. Root admission must use floor((M-A)/P), reject M<A even for schema-only, and reject impossible resolved configurations before target mutation.

This accounts for controlled requested bytes/state/protocol buffers. Uncapped authoritative plan/report heap, preflight template clone, allocator rounding/reallocation, opaque Tokio/channel/task/std-process bookkeeping, kernel pipes and driver buffers remain separate process/RSS costs. It is not a total RSS guarantee. Caller-owned Confirmed snapshot clones require caller accounting; actor operation paths do not clone table ledgers.

### Lifecycle/publication

All directory/plan/report/reject filesystem work executes in the child. Strict80-byte headers and48-byte receipts validate magic/version/sequence/reserved fields/table IDs/generation/offsets/lengths. ACK follows required file/directory sync. Immutable report publication: synced temp -> exclusive hard link to generation -> unlink temp -> directory sync -> head hard link -> rename report.json -> directory sync. Parent-confirmed immutable generation survives failed later publication. Cleanup removes only temp/head names created by that operation, preserving independent exclusive-create blockers. Subsequent parent-confirming request removes the older confirmed generation; successful operation retains confirmed+next+bounded temp/head names. An unacknowledged report.json replacement is not durable ACK evidence.

One std ChildSlot/reaper thread survives runtime teardown. Actor guard is captured before spawn, protecting never-polled task drop. Accepted flight/ticket/raw/batch permits stay in global slot on failed/unknown I/O until actual child reap. Kill targets only its owned child object. A still-owned child makes later start Busy, preventing accumulation. Deadline bounds cooperative pipe waiting; kernel-uninterruptible child may survive and remains visibly leased. SIGKILL is not syscall cancellation or physical durability proof.

### Independent checks

Final focused command: rtk run 'my2pg/bin/cargo test --offline --features artifact-worker-tests artifact_io -- --nocapture'. All4cases passed:2pure library +2process; process lane0.78s. Default-off feature artifact-worker-tests registers my2pg-artifact-test-worker at tests/support/artifact_worker.rs; no production fault switches/environment modes. Actual evidence:

- Private directory and plan/report; lossless NaN/infinity IEEE bits/raw00ff; original COPY subrange and exact sidecar offset.
- A-minus1 refusal before creation, concurrent live-worker Busy, dynamic cap refusal preserving generation1.
- Generation1 retained through generation2 publication until next parent-confirming request; old generation then removed.
- Actual exclusive-create failure retains confirmed generation2/head and independent blocking file.
- Test child verifies exact metadata descriptor device/inode before replacing it with a full blocking Unix socket.50ms dropped receipt retains raw permit/ticket;100ms shutdown returns within500ms; actual child reap releases failed suffix; durable count stays1.
- Writable socket gives actual fsync failure. Independent socket probe proves native macOS EBADF9 (not assumed EINVAL22); production error errno agrees and original file has one record.
- Held durable response on test pipe, then dropped caller waiter: releasing ACK advances actor ledger to2, budget remains2, pending clears and permit releases without caller notification.
- Active blocked child across current-thread runtime teardown remains globally owned/reaped; failed suffix reservation releases afterward; durable prefix stays1.
- Initial create ACK withheld: live_worker proves owned PID/pending1/generation0 while start is pending; dropping the entire start future triggers owned kill/reap, and global slot clears. Created directory remains without a falsely acknowledged initial report.
- Malformed/short frames, reserved bytes, unknown opcode, uppercase ID, wrong sequence and undersized bootstrap never produce successful ACK.
- Serializer tests prove C142/h44 and two hexadecimal digits per binary byte.

Initial test failures exposed incorrect assertions: IEEE fields are integer bits, not hex strings; native fsync(socket) returns9, not22. Assertions now use parsed bit values and independently probed errno. An unrelated root Option<String> test edit temporarily blocked compilation and was repaired by root; no peer edits here. Failed fixtures are retained on panic, successful roots removed. No DB fixtures/physical volumes started.

Strict all-target/all-feature Clippy final pass0.82s. Owned files formatted with pinned Rust1.99 rustfmt using workspace CARGO_HOME/RUSTUP_HOME, without recursive peer formatting. A documentation command initially used unsafe nested quoting of Markdown backticks and failed before writing; switched to apply_patch. No sensitive data involved. Final process case exposed a test-ready file creation/write race: observer read an empty file before complete descriptor evidence. The preserved root is /var/folders/x_/gmqbjcj50cjdyz8kcj30g4jr0000gn/T/my2pg-artifact-test-83579-1790906764287298000 (ready evidence83582:16777233:6257428:9). Readiness now waits for four fully parsed fields, not mere path existence; focused green above includes that fix.

### Open gates

Root main dispatch, C pipeline/recovery async ownership migration, artifact resource/config admission, full migration cancellation/fault proof and physical storage-sync failure proof remain open. Blocking console stdout/stderr is a separate T15 shutdown seam. This is not T12/T15 completion or release acceptance.
