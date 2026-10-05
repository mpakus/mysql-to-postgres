# T12 — physical sync EIO gate review

Status: **open**. No test or production code was changed. The currently owned native faults prove physical `ENOSPC` on write and descriptor-induced `EINVAL` on `fsync`; neither is an EIO returned by a filesystem while syncing production artifact data. No fixture was started.

## Reference research and original implementation

Followed `docs/reference-coding.md`; used RTK for searches and original-file reads. Ponytail guidance was unavailable in this environment: `command -v ponytail` and `rtk find /Users/renatibragimov/.codex/skills /Users/renatibragimov/.agents/skills -iname '*ponytail*'` returned no result. Project checkout and XERJ project-index revision observed: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.

Queries and pins:

- `project-my2pg`: `physical fsync EIO sync_all`; `forced detach fsync actual syscall errno`; returned T12 durability worklogs and `tests/support/durable_faults.py`.
- `ref-rust-postgres`: `filesystem fsync sync_all durable error`; pinned commit `1084ca8f5b5302e161892f2fa40abf71b4060c10`; lexical results were unrelated codec/TLS/notification code, with no filesystem sync implementation or fault fixture.
- `project-pgloader`: `sync_all filesystem error`; pinned commit `231ab86778ca5ffd7de40878714760c8b4860cdf`; results were unrelated fixture/logging/batch-retry source, with no filesystem durability fault fixture.

Read original sources and adjacent tests:

- `tests/support/durable_faults.py:40-50,53-90,93-135`: every mounted operation checks the exact owned image, device and mountpoint; the 64 MiB HFS+ image reaches native errno 28 on a one-byte write. The probe calls actual `os.fsync`, force-detaches that exact image, and records the post-detach syscall result.
- `tests/cases/t12_durable_faults.rs:104-145,330-390`: production report/reject writer ENOSPC assertions preserve prior report bytes. The child-only descriptor test verifies the original metadata inode/device, replaces only the child FD with a Unix socket, observes actual `fsync` errno 22 (`EINVAL`), and labels the evidence descriptor-only.
- `src/report/mod.rs:144-165,217-263`: reject-copy flushes and calls `sync_all` on data before writing metadata; report/document and metadata persistence also call `sync_all`. The reject writer is poisoned after any persistence error.
- `../references/rust-postgres/tokio-postgres/src/copy_in.rs:128-142` and adjacent `tests/test/main.rs:739-764`: COPY finish/drop behavior concerns PostgreSQL protocol/transaction completion, not local filesystem durability. No code is adapted.
- `../pgloader/src/pg-copy/copy-retry-batch.lisp:1-90`: original pgloader batching/recovery deals with COPY errors and retry subdivision, not artifact fsync. No code is adapted.

Existing T12 worklogs (`2026-10-01-T12-durable-faults.md`, `2026-10-02-T12-uncertain-storage.md`) retain the negative and positive native evidence. The forced-detach syscall returned success/errno 0, real image exhaustion returned `ENOSPC` during write, and the socket replacement returned `EINVAL` on fsync. These are distinct failure classes and must remain distinct in evidence and reporting.

## Decision and proposed acceptance

I did not add a test because the current owned-image mechanism cannot deterministically produce a genuine filesystem/block-I/O `EIO` at the production `sync_all` boundary. No FUSE mount helper or `/dev/fuse` is present in this environment. Truncating or corrupting an attached image, simulating an errno in a wrapper, replacing a descriptor, or re-labeling ENOSPC/EINVAL would not independently establish the requested acceptance claim and could damage the active owned image without guaranteeing an EIO from sync.

A sound next test needs a dedicated, disposable faulting filesystem/block-device fixture that returns a genuine EIO to the kernel `fsync` syscall for the exact production artifact FD. It must retain independent proof of the faulted device/volume identity, syscall errno and operation stage; assert nonzero/indeterminate surfaced outcome and that the prior durable report bytes remain unchanged; and cleanly detach only the owned fixture. The fixture support must fail if it gets success, ENOSPC, EINVAL, or any error other than EIO. This likely requires a reviewed support-harness change or a dedicated CI host facility, so it is outside this new-test/worklog-only lease; request a precise lease for `tests/support/durable_faults.py` (or a new dedicated helper) before implementation.

Do not close the T12 physical-sync checklist gate based on current cases. `docs/testing-and-performance.md` already distinguishes descriptor-induced EINVAL from physical sync failure, and `docs/checklists.md` correctly leaves physical sync open.

## Validation

Read-only investigation only. XERJ searches succeeded after the coordinator confirmed the shared corpus was stable. No database pair or mounted image was started. There is no code/test change to compile or lint.
