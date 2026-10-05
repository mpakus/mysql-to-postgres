# T12/T15 — actual durability and localization faults

Status: three native durability cases and one separately registered NLS case passed; production unchanged. Exclusive source lease is new `tests/cases/t12_durable_faults.rs`, new `tests/support/durable_faults.py`, this worklog, and subsequently coordinator-approved new `tests/cases/t12_localized.rs`. The coordinator owns registration/image pins and any verified repair.

## Reference research before code

Read reference-coding workflow, architecture COPY/recovery/artifact requirements, current production `RunArtifacts::{reject_copy,reject_conversion,write_report,flush}`, `write_record`, `write_document` and directory sync, current runner finalization, adjacent production-writer tests and actual T12 quota/collision/COMMIT signal cases. Applied RTK and Ponytail guidance already read. Source/index revision observed in renewed searches:776901a1377fbea654bdf19f7a201014af757fbc; originals read directly because working changes can follow that revision.

XERJ `project-my2pg / ENOSPC sync_all report.json metadata` located actual report/metadata file sync and prior integration review. `project-pgloader / reject disk full locale sqlstate` returned logging/locale formatting and state declarations at pinned231ab86778ca5ffd7de40878714760c8b4860cdf, but no physical disk-full or fsync fault fixture. Existing pinned rust-postgres COPY/transaction tests are the transaction oracle reference; no reference code is copied.

Primary sources: Apple's [fsync manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html) distinguishes successful sync from EBADF/EINVAL/EIO syscall failure and documents F_FULLFSYNC's stronger hardware-ordering guarantee. We test the application's actual current sync_all/fsync behavior, not crash/power-loss durability. PostgreSQL16 [SQLSTATE appendix](https://www.postgresql.org/docs/16/errcodes-appendix.html) and [locale documentation](https://www.postgresql.org/docs/16/locale.html) say error codes are locale-independent while parsing translated text is unreliable. Need actual server-localized builtin errors, not trigger-raised pretend SQLSTATE/messages.

Local read-only discovery: host is Darwin; `/dev/full` is absent; `/usr/bin/hdiutil` and CommandLineTools clang exist. Read actual `hdiutil create/attach/detach -help` before use. This OS warns that hdiutil verbs are deprecated in favor of diskutil image verbs, but their documented fixed-size image/mountpoint/plist interface remains available. No system/global volume operation is proposed.

## Published owned filesystem lifecycle — before mounting

Runtime artifacts use a fresh exclusive `my2pg/target/t12-durable-<uuid>/` directory, mode0700. Exact image `volume.dmg` is a fixed64MiB HFS+ UDRW image; mountpoint `volume/` is an empty newly created directory within that owned root. Record returned plist/device identity and logs outside the mounted image. Planned native commands:

```sh
hdiutil create -size 64m -fs HFS+ -type UDIF -volname MY2PG_T12_FAULT <owned-root>/volume.dmg
hdiutil attach <owned-root>/volume.dmg -mountpoint <owned-root>/volume -nobrowse -noautoopen -plist
# Fill only <owned-root>/volume/filler.bin with a bounded amount of allocated data.
# Probe actual fsync/open-file behavior after planned removal of this owned image.
hdiutil detach <owned-root>/volume -force
# If needed, reattach this same image at this same owned mountpoint to inspect evidence.
hdiutil detach <owned-root>/volume
```

Publish lifecycle to coordinator before any mounted-image operation. Helper must validate owned paths/device identity, never select an arbitrary disk, never fill the host filesystem, and cap writes by the image size. Normal teardown stops all owned child processes first and detaches only the recorded mountpoint/device. A forced detach is used only for the explicit native fault or an owned busy-cleanup fallback. Retain logs/image evidence; never delete/mutate unrelated volumes or Strangler containers.

First probe uses an open file descriptor, written data and real os.fsync before/after owned forced detach. A failure counts only if the real syscall returns an observed errno. If it succeeds, record that result and do not label it metadata-sync evidence. For production stage positioning, a child-only native pause barrier may intercept fsync/write solely to pause before calling the real syscall; it must never return a fabricated errno. An actual physical/native syscall failure and an injected failure remain separate claims. Any needed alternative production seam is proposed to root before changes.

Initial create returned native argument error: this installed hdiutil accepts -format only with srcfolder/srcdevice; a blank image uses its documented -type UDIF read/write default. No image was mounted by that failed attempt. Corrected/published command above before retry; ownership/size/cleanup bounds are unchanged.

## Acceptance oracles

- Physical ENOSPC must be distinguished from EFBIG and path collisions using the observed OS errno on the owned filesystem.
- Already acknowledged target rows/counters remain counted; reject counts advance only after the concrete writer returns durable success. Poisoned writer flush is nonzero and surfaced.
- A real report-file/sync failure leaves the previous atomic durable report bytes intact. Restored filesystem may permit final failed report persistence; last snapshot is evidence rather than a checkpoint.
- COMMIT uncertainty remains indeterminate and never causes replay, including during later artifact/console failure. Where the filesystem cannot store an updated report, explicitly identify the last durable snapshot and inability to persist new uncertainty.
- A real non-English PostgreSQL builtin failure still carries the same SQLSTATE and reaches the same rollback/recovery outcome; mandatory locale absence is a recorded gap/failure, not a successful skip.
- Actual CLI path and fresh owned database fixture are required. Root owns final registration; cleanup targets only resources started for this task.

## Native probe and approved descriptor fault refinement

Approved owned probe used `target/t12-durable-52746e753ca84a69af11a71d3a90bfbc/volume.dmg`, mounted as/dev/disk5s1 at its exact private volume/ path. Real allocated writes reached65,011,712 bytes and failed with errno28 ENOSPC. Image was detached, reattached to inspect unchanged probe bytes, then normally detached; ownership/probe JSON is retained outside it. **Negative evidence:** actual os.fsync on the open descriptor after forced detach returned success/errno0. This method does not prove sync failure; no syscall hook/interposition is justified by that result. Fill now reduces its write size until even one byte returns native ENOSPC, preventing a misleading large-write failure while small report writes still fit.

Coordinator separately approved a child-only descriptor fault as actual syscall/descriptor evidence, explicitly not physical storage-sync failure. Concrete mechanism before execution: a disposable test child identifies its own production reject metadata FD using Darwin F_GETPATH and inode/device identity; replace that FD only using native dup2 from a connected Unix socket pair. The next production reject_copy first appends/flushes/syncs its unchanged data File, then metadata writes succeed into the socket, but its actual File.sync_all/fsync returns native EINVAL because the FD is now a socket. Keep and drain the peer; no fabricated errno, preload/global env, host descriptor manipulation, or production hook. An already durable first rejection is the baseline; second rejection must not advance counters/tickets, prior on-disk metadata/report bytes stay unchanged, appended orphan data remains evidence and poisoned flush fails. File ownership remains valid (dup2 replaces the descriptor with another open descriptor), and this deliberately isolated fault executes only in the disposable child.

First three-case run: both physical ENOSPC writer/atomic report and actual CLI tests passed. Descriptor child failed before injection: Darwin stat of `/dev/fd/N` returned the fdesc filesystem device rather than the original file device, though inode matched. Correct the test using a borrowed owned-child descriptor cloned with Rust's OwnedFd and File::metadata (fstat); close that clone before injection. This preserves verified inode/device ownership without manipulating the production FD before dup2. Renewed XERJ queries `project-my2pg / metadata sync_all durable reject` and `ref-rust-postgres / DbError code message` returned production write_record/sync_all and driver structured-error parser at the unchanged project/reference revisions above. Originals/adjacent tests reread before this correction and localization addition.

Coordinator approved a separate official native PG16 NLS image lane, manifest sha256:1c2f3efc9c5ab63fe557565c9443dbcfc0cecd2b28a3989244dfc80eb6cb96f9. Read coordinator localized-image research and original pinned DbError accessors: SQLSTATE field C is independent of translated message M. New test will require NLS prerequisites (never successful skip), set server session lc_messages only on the disposable target, observe an actual built-in negative CHECK failure's French message and SQLSTATE23514, then run production rollback/bisection and verify exact ACK/reject bytes and SQLSTATE diagnostics. No host locale changes or reference code copied.

## Actual verification and boundaries

Owned main fixture: `target/integration/my2pg-mysql84-pg16-709803da445b/connections.json`. After correcting the child identity check, the final run with actual ACK1/reject1 baseline report passed all three cases in2.41s:

```sh
source target/integration/my2pg-mysql84-pg16-709803da445b/env.sh
bin/cargo test --locked --test integration t12_durable_faults -- --ignored --nocapture
```

- Physical64MiB image: fill reaches actual errno28 even for a one-byte write. Production report replacement and reject data write both return ENOSPC28, previous report/metadata bytes remain unchanged, temporary report is removed, poisoned flush fails. Restoring space allows report writing while retaining its already-indeterminate fixture status. This status-retention check uses a report fixture; it does **not** simulate a new uncertain COMMIT.
- Production runner/CLI: independently block its second COPY on a real unique-key lock after the first batch commits and report ACK2 is durable; fill its image, commit the competing row, then real SQLSTATE23505 triggers reject writing and filesystem failure. CLI exits1, stderr exposes artifact failure, last durable report remains byte-identical with ACK2/reject0, and target contains exactly source rows1/2 plus independent locker row3. Source row4 never inserts/replays. Report snapshot remains the last durable evidence; no claim that storage successfully persisted a newer failed report is made.
- Disposable child metadata fault: verified backing inode/device through F_GETPATH plus fstat. Production data append/flush/sync completes, metadata JSON reaches the socket, then actual fsync returnsEINVAL22. Recovery reports RejectIo, second durable-reject delta remains0, global budget remains1, target ACK1 remains present, prior disk metadata/report bytes stay unchanged with actual ACK1/reject1, orphan reject data is `-1\n-2\n`, poisoned flush fails. Latest retained native proof: `target/integration/my2pg-mysql84-pg16-709803da445b/t12-durable-descriptor/run-18da8a0ad9aca5b0-86c4-0/descriptor-proof.json` (FD11, inode6044892, device16777233). This is a real descriptor/syscall failure, **not** physical storage-sync failure.

Owned NLS fixture: `target/integration/my2pg-mysql84-pg16-nls-c26ea35d8f34/connections.json`; native aarch64 Debian PG16.15 build explicitly contains `--enable-nls`, `/usr/share/locale/fr/LC_MESSAGES/postgres-16.mo`, and localedef. Generated fr_FR.UTF-8 only inside that exact container. Initial server session rejected the newly generated locale with22023 because the running postmaster had initialized beforehand. Restarted only the owned PostgreSQL service; Docker changed its ephemeral host port, so inspected its exact port and updated only owned connection metadata before rerunning. No shared harness or host locale was changed. Native prerequisites are retained in `nls-prerequisites.txt`.

Coordinator registered separate `tests/nls.rs` behind required feature `nls-integration`; generic Alpine integration does not discover it. Missing locale is a failed prerequisite, never a successful runtime skip. Corrected an initial nested test-model `{}` literal to serialized SchemaExpectations::default; actual production recovery then passed1/1 in0.31s:

```sh
source target/integration/my2pg-mysql84-pg16-nls-c26ea35d8f34/env.sh
export MY2PG_NLS_LOCALE=fr_FR.UTF-8
bin/cargo test --offline --features nls-integration --test nls -- --ignored --nocapture
```

Actual built-in message: `la nouvelle ligne de la relation « rows » viole la contrainte de vérification « rows_n_check »`, translated severity `ERREUR`, SQLSTATE23514. Production recovery commits exact rows1/2, durably rejects exact bytes `-1\n` at ordinal2, returns ACK2/reject1/budget1, and writes its sanitized SQLSTATE23514 diagnostic. Retained proof: `target/integration/my2pg-mysql84-pg16-nls-c26ea35d8f34/t12-nls/run-18da8a129e5a7818-8732-0/localized-error.json`. Neither forged trigger SQLSTATE nor translated string matching determines production classification; test string matching confirms only the independent French oracle.

Strict `bin/cargo clippy --offline --features nls-integration --all-targets -- -D warnings` passed3.23s after the coordinator repaired an unrelated T10 test lint. Python helper AST validation passed. All eight image ownership states are attached=false (seven native attached probes/tests and one unmounted initial create failure); exact devices recorded, images retained. No production defect was found. Remaining full-gate limit: physical storage-sync failure was not achieved—forced detach/fsync succeeded—and the descriptor regression must not close that physical requirement. These new cases also do not replace the existing actual uncertain-COMMIT/no-replay cases or claim the combined disk-full/new-unknown-COMMIT condition has been exercised.

Approval review once rejected a readiness command that would have printed the connection URL; the safe replacement checked only endpoint equality and readiness without exposing credentials. No blocked action remains from that rejection.

Final cleanup: exact harness --stop commands for709803da445b andc26ea35d8f34 completed; both retained connection files report state=stopped. All task images remain detached. Final fmt--check has no owned-file difference after formatting the ACK tuple; only concurrently edited coordinator-owned src/mysql/mod.rs differs. Reported to coordinator instead of changing another lease. Source/tests/worklog frozen for handoff; physical-sync and combined uncertainty/storage fault limits above remain open.
