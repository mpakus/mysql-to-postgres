# 2026-10-01 — T15 artifact protocol and admission contract

Status: implementation design recorded before code; coordinator owns production integration. Read-only design lease is this file. Follow-up implementation lease is artifact_io.rs, minimal report/mod.rs integration, t15_artifact_io.rs and the worker worklog. No pipeline/recovery/scheduler/main/model/Cargo edits by this worker.

## Research and approved boundaries

RTK/Ponytail applied. Read reference workflow, current MigrationPlan/TablePlan/ColumnPlan/RawValue/RowLocator/TableReport/RunReport/Diagnostic models, scheduler MemoryLayout/Admission/Resources and its cancellation/retention tests, current raw_values size admission, report StoredValues/Hex serializer and synchronous writer tests, runner report/counter/producer/recovery paths, worker entry in main.rs, prior ENOSPC/descriptor-sync proofs.

Fresh XERJ project-my2pg query "MemoryLayout retained_per_table report", revision2b0573083761aa664c195514944459d1d595dbfc, located scheduler workspace formula and T13 native cases; originals read directly after lookup. ref-dmt-rs "WriteJob channel read_ahead", pin4e8015f7e841dbdf9df01e953aeb3948dfb3199a, located transfer.rs; reread original transfer/mod.rs:410–431 and adjacent signal script. Bounded channel slots do not count retained bytes or imply durable ACK. project-pgloader "process-bad-row write-sequence", pin231ab86778ca5ffd7de40878714760c8b4860cdf, located utils/reject.lisp and Clojure batch; original reject.lisp appends bytes then condition but has no fsync receipt. Reuse none of its weaker durability policy.

Read exact installed Tokio1.53.1 task/blocking.rs:106–143, process/mod.rs:198–232,1104–1156,1504–1545 and adjacent child-drop tests; process/unix/orphan.rs only provides best-effort orphan reaping. ChildStdin/ChildStdout::from_std provide async pipes while std::process::Child remains globally owned independent of a Tokio runtime. Read serde_json1.0.151 ser.rs:410–455: collect_str streams Display chunks through escaping and does not construct a full hexadecimal string. Our existing Hex serializer is appropriate for count-then-capped serialization.

Coordinator approves finite serialization caps with explicit overflow failure, immutable confirmed report generations, concrete owned reject tickets, global child/reaper containment and honest kernel limits. Uncapped authoritative model/catalog heap is outside the serialization payload bound; no total-RSS assertion follows from this reservation.

## Exact serialization inputs

J(x) means actual serde_json to_writer_pretty count plus newline for documents; K(x) means compact actual serde_json count plus newline for reject records. Counting writer stores only a checked usize counter, no JSON DOM or output allocation. The admitted encoder writes into a fixed boxed byte slice with a cursor; it never grows. Both counting and encoding fail on arithmetic/allocation/serialization error. Serialization larger than the cap fails before submission, without changing any artifact.

For the resolved run, let:

- R = max_row_bytes, N = selected table count, n_t = copied source column count for table t, v = size_of(mysql_async::Value).
- S_plan = J(actual plan).
- S_report = J(report sizing template): current identities/exclusions/plan diagnostics; longest status strings; every u64 counter/elapsed=u64::MAX; tables_checked=usize::MAX; all planned transform keys and counts=u64::MAX; all planned DDL object names and hook paths as failed steps; one fixed artifact-failure diagnostic. This bounds that finite template, not arbitrary future diagnostics/differences.
- S_boot = exact binary bootstrap bytes containing known base/run identity, u64 decoded table IDs and declared limits.
- F = max(S_plan,S_report,S_boot). Initial report identity uses the same parent-generated run ID/path as Create. No filesystem operation is needed to choose that identity.
- C = maximum compact serialized StoredValue length among maximal type-domain scalar samples: Null, i64::MIN, u64::MAX, f32/f64 bit maxima and all Date/Time field maxima. h = compact StoredValue::Bytes(empty) length. Values derived through the exact current serializer, not manually maintained guessed constants. Current independent JSON probe gives C142, h44; implementation tests must prove exact Rust serialization.
- b_t = R - n_t*v when nonnegative. If n_t*v>R, no row of that table can pass the current raw admission; a safe bound with b_t=0 remains valid but does not claim its rows are readable.
- V_t = max(n_t*C, (n_t-1)*C+h+2*b_t) + max(n_t-1,0), with V_t=0 for n_t=0. A single Bytes field maximizes expansion because C>=h, and every retained raw byte produces exactly two ASCII hex bytes. Commas are counted separately; array delimiters belong to the envelope.
- H_conversion = largest K(empty-values reject envelope), with ordinal=u64::MAX, key=None, object=generated table ID, existing fixed version/kind/code/stage/severity and empty diagnostic message. H_copy similarly uses offset/length maxima and its fixed encoding/kind.
- D = max(F, max_t(H_conversion+V_t+F), H_copy+F). The added F is a declared finite allowance for variable diagnostic message/optional locator key serialization, not a claim any arbitrary String fits. Entire actual metadata/document must fit D; larger runtime diagnostics/differences/keys fail publication explicitly, preserve confirmed evidence and do not silently truncate or turn into a successful rejection.

All additions/multiplications and integer narrowing are checked. Skip row/reject expansion in schema-only mode because no reject operation is legal there; still reserve fixed documents/buffers/ledger. A public derive helper returns the computed limits before any target mutation. Independent boundaries cover D-1/D/D+1 and usize/u32/sequence overflow; tests do not mirror a guessed multiplier.

## Artifact reservation and scheduler seam

One operation total is admitted, including active command, queue and pending pipe bytes. An operation permit is acquired before document encoding/owned payload construction. Parent never clones a complete report or converts it to serde_json::Value during publication: acquire publication gate, borrow current report under its short mutex, encode into the admitted fixed buffer, release mutex, then await child I/O. Preflight template/catalog sizing may own temporary model metadata; it is not falsely covered by the runtime payload budget.

A = D + 2*16384 + 80 + 48 + N*(size_of(ParentAckEntry)+size_of(WorkerTableState)) + L_paths + C_control.

The two project-controlled fixed buffers are child input16KiB and child buffered output16KiB. Parent writes the boxed payload/COPY slice directly to its async pipe and reads the48-byte response; no second payload/transfer Vec. Parent ACK entry contains fixed-width decoded table ID, reject count, last reject sequence and COPY offset; worker table state has decoded table ID/offset and optional owned data/metadata files. L_paths counts fixed-capacity owned identity/base and scratch path storage using actual known byte lengths and longest generated filenames. C_control is size_of the concrete actor/command/receipt/fixed one-slot mailbox state, excluding opaque Tokio/std/allocator/kernel bookkeeping that remains a measured runtime cost. Implementation must expose each component, no unexplained constant for variable strings or catalog snapshots. Global reaper stack/process/TLS/runtime/allocator/OS pipe buffering remain explicit process-tree RSS measurements.

Parent owns one boxed D-byte payload at a time; conversion keeps the original Vec<RawValue> (no clone) and an Arc<OwnedSemaphorePermit> covering its existing row/workspace, plus the counted metadata buffer. COPY keeps Arc<BatchStorage>, preserving its original encoded-byte permit; it transmits the exact immutable range without copying its allocation. Cancellation cannot release either before definitive completion or actual child reap. The operation permit travels with the accepted command too. One shared owned reject budget/ticket covers both conversion and COPY; actor holds and commits ticket only after sync-success ACK, then updates the bounded per-table ledger before waking caller.

Current scheduler P = (Q+2)*(batch_bytes+batch_rows*size_of(RowPosition)) + 3*R + encoder_workspace_bytes(R) +65536. Required artifact admission is separate and unavailable to tables:

- data migration: effective table workers = min(requested,N,floor((memory_bytes-A)/P)); require at least one when N>0.
- schema-only: require memory_bytes>=A; no row pipeline reservation needed.
- reserve A once for the run lifetime before creating a child/target mutation; initialize table byte semaphore from memory_bytes-A or take A permanently before any table lease. Never acquire report memory opportunistically after full batches occupy all bytes.
- schema-aware A cannot be known by offline check. Local check still validates P and the proved fixed artifact lower bound; actual resolved plan computes A before writes. Report actual required minimum A+P or A, not a stale P-only error.

Root/C must adapt Admission::new to accept ArtifactReservation, Resources to reserve it before table leases, shared queue boundary/unit tests, runtime/offline exact-minimum config cases, schema-only memory_bytes=1 cases, native concurrent-worker fixture sizing and resource-diagnostic output. Importer translations must still reject inadequate sizing rather than resizing config. No memory knob is added.

## Concrete facade and worker seam

artifact_io exports ArtifactIdentity::new(base), ArtifactLimits::derive(plan,initial_report,max_row_bytes), ArtifactReservation component accessors, DurableArtifacts::start(worker_command,identity,limits,table_ids,owned_artifact_permit), lease(), flush(), shutdown(deadline), confirmed() and concrete ledger snapshots. Start succeeds only after Create sync ACK; source start/create/plan/initial report all perform artifact I/O through the child, never through parent RunArtifacts calls.

ArtifactLease has typed write_plan(&MigrationPlan), write_report(generation,&RunReport), reject_conversion(table_index,&RowLocator,owned RawRetention,&Diagnostic,OwnedRejectTicket), reject_copy(table_index,&RowLocator,Arc<BatchStorage>,range,&Diagnostic,OwnedRejectTicket) methods. Bodies are serialized while holding admission; async methods return typed receipts. Receipt cancellation never drops accepted operation ownership. Lease rejection/serialization failure before submission releases its unused ticket; accepted operations are retained in the global owned child slot until ACK/failure/reap.

OwnedRejectBudget::new(limit), Arc-backed reserve()->Option<OwnedRejectTicket>, used(); ticket commit/drop retain current atomic reserve semantics. C later reexports/adapts this one budget, not a parallel counter. RawRetention moves existing raw values plus Arc row/workspace permit. API has no generic sink trait or arbitrary closure carrying ownership.

serve_worker(input:impl Read,output:impl Write)->io::Result<()> runs the concrete child protocol and synchronous filesystem operations. Existing RunArtifacts APIs remain for direct tests; expose shared record serialization helpers internally to avoid duplicate raw/hex/bit encoding. Child serve must use the same private-file/directory helpers and an exact parent-selected identity creation helper.

CLI intercepts exactly an internal worker argv marker before Clap/user signal/runtime setup. Prefer ordinary synchronous main dispatch: if internal marker only, call worker_stdio() and return sanitized operational code; otherwise construct existing Tokio main runtime/async run. No env credential passes: spawn current absolute executable with env_clear, stdin/stdout pipes, stderr null. Worker command is an explicit internal start argument so a library/test binary is never blindly relaunched as CLI. Test child entry calls exactly serve_worker/worker_stdio, with sibling fault helper confined to test code; no production environment toggle or fake errno.

Protocol80-byte requests: magic8, version2, op1, reserved1, seq8, report_generation8, table_id18, reserved2, COPY_offset8, data_len8, payload_len8, parent_confirmed_generation8. Big-endian integers, zero reserved bytes, strict op/length/table/sequence validation, explicit caps. Table IDs are the existing t_<16 lowerhex> grammar and bootstrap-proved membership. No arbitrary filename from row/database names. Bootstrap field lengths/table counts must match exact declared limits and fit the counted body; child never allocates an unbounded advertised length. COPY transmits data before metadata. Responses48 bytes: magic8, version2, result2, seq8, durable_generation8, COPY_offset8, errno4, reserved8. Error result is a finite stage class plus native errno; no raw path/secret/error text.

## Confirmed report retention and acknowledgement

Child keeps at most confirmed generation, next generation and one temporary report body. Write/sync the immutable generation, directory-sync it, replace report.json through a same-directory link/rename and directory-sync publication, THEN send ACK. Parent validates ACK seq/offset/generation before ledger/ticket commit; update ledger before oneshot response. The next request carries parent-confirmed generation and permits safe older generation cleanup. A lost ACK or failed rename/directory sync keeps the earlier confirmed immutable file. Never call an unACKed report.json the last durable report; final fixed diagnostic identifies its confirmed generation and unreaped PID/pending operation when necessary.

Reject metadata may contain a physical unacknowledged suffix after child death; do not rewrite/truncate it or use its presence as ACK. ACKed prefix/counts are authoritative; no automatic replay. Original row COMMIT ACK accounting is synchronous before artifact await and remains independent of artifact unknown status.

## Lifecycle containment

One process-global ChildSlot owns std::process::Child, life state, accepted operation retention, ACK ledger and artifact memory permit. One lazily started std reaper thread polls only that single child's try_wait; it performs no artifact syscalls and is not a Tokio blocking task. std ChildStdin/Stdout are moved through Tokio's from_std APIs; coordinator must enable process and io-util features.

Actor and its drop guard never relinquish the child to Tokio's best-effort orphan list. On caller/runtime drop, mark stop, request kill through the owned Child, leave pending bytes/batch/raw/ticket and global gate retained until actual reap. Reaper survives runtime drop and clears the slot only after try_wait proves exit. New runs refuse while the slot is occupied, with fixed sanitized visible PID/state; no substitute child/reaper/task is launched after timeout. A transient try_wait/kill failure retains ownership and is reported, not treated as successful reap.

Shutdown uses the shared deadline: stop admission, drain one outstanding response, reconcile ACK ledger, finish final report only while responsive, Close, request kill when time expires, and wait no longer than remaining deadline. Reaper remains globally bounded if actual reap cannot be observed. Killing cannot prove interruption of an uninterruptible syscall, hardware durability, process-tree quiescence or power-loss safety. OS process launch/kill/try_wait themselves are native kernel operations, not hard realtime guarantees. Existing synchronous console output is a separate T15 stall boundary; this increment does not quietly claim it is fixed.

## Required independent checks

Direct serializer maximum/overflow/cap tests; malformed/op/table/sequence/oversized/truncated request tests against actual child; confirmed generation before failed rename/directory-sync ACK; dropped receipt retains ACK ledger and ticket; runtime drop kills/reaps or preserves occupied global lease; repeated run cannot accumulate children/tasks.

Native syscall stall uses a disposable test worker with the real production serve loop. Its sibling test helper verifies its own artifact metadata FD by F_GETPATH/fstat/inode/device, substitutes a filled blocking Unix socket, and retains the peer undrained. Real write_all blocks on OS capacity. Parent heartbeat/real owned database cancellation and shared deadline remain responsive, previous durable prefix intact, pending reject not ACKed, and socket-blocked owned child killed/reaped. No fake fsync result or production test flag. Real ENOSPC and descriptor fsync EINVAL remain independent runtime cases; physical storage-sync failure/stall remains open unless observed on an actual scoped facility.

Design recorded before implementation; exact source APIs and component sizes must be sent to coordinator once published. No implementation acceptance or native proof claimed here.
