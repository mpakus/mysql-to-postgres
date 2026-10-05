# T12 — Linux FUSE physical-sync fault prototype

Status: Linux FUSE acceptance case passes locally; hosted CI execution remains open.

## Research and ownership

- Project HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the worktree is dirty.
- XERJ query: `project-my2pg "physical fsync EIO artifact writer" -k 8 --full 100`; indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It returned the prior [T12 physical-sync review](2026-10-02-T12-physical-sync-eio.md), [safe-fixture audit](2026-10-02-T12-safe-eio-fixture-audit.md), durability tests and report writer worklogs. Query `ref-rust-postgres "fsync EIO filesystem" -k 5 --full 100` returned no relevant passages; the pinned Rust PostgreSQL project handles COPY protocol completion rather than filesystem persistence.
- Pinned reference revisions remain `rust-postgres` `1084ca8f5b5302e161892f2fa40abf71b4060c10`, `pgloader` `231ab86778ca5ffd7de40878714760c8b4860cdf`, and `dmt-rs` `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Prior worklogs and original implementation review found no applicable FUSE fixture in those migration tools.
- Read `tests/support/durable_faults.py:40-135`, `tests/cases/t12_durable_faults.rs:107-165,260-390`, and `src/report/mod.rs:32-87,144-165,217-268`. The open production boundary is `File::sync_all()` while writing report/reject artifacts; existing evidence distinguishes real `ENOSPC` from descriptor-induced `EINVAL`.
- Official contract consulted: Linux kernel FUSE documentation describes request/reply from the userspace filesystem, and libfuse documents its `fsync` callback. A userspace FUSE callback returning `EIO` can therefore test the kernel syscall result instead of substituting an errno in the application process. Sources: [Linux FUSE overview](https://www.kernel.org/doc/html/latest/filesystems/fuse/fuse.html), [libfuse fsync callback](https://libfuse.github.io/doxygen/structfuse__operations.html), [libfuse reference implementation](https://github.com/libfuse/libfuse).
- Implementer pre-edit query: `project-my2pg "RunArtifacts write_report fsync EIO FUSE"` at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` returned prior T12 reviews and adjacent tests. `ref-rust-postgres "fsync EIO filesystem failure"` at `1084ca8f5b5302e161892f2fa40abf71b4060c10` had no relevant FUSE result.
- Implementer reread `src/report/mod.rs:82-97,245-254`: `write_report` creates a private temp file, serializes/flushes and calls `File::sync_all`, renames, then syncs the directory. It also reread `tests/cases/t12_durable_faults.rs:1-170` and `tests/support/durable_faults.py:1-180`; those cases cover macOS disk-image ENOSPC and do not provide Linux FUSE behavior.
- Ownership lease: `/root/t12_fuse_eio` owns `tests/cases/t12_fuse_eio.rs`, one isolated helper/container file, and the single target registration line in `tests/integration.rs`; the coordinator owns this worklog and integrates results. The lease excludes production report code, the existing macOS helper, shared Compose matrix, and another agent's CI changes.

## Updated capability evidence

- Native host remains macOS ARM64; its `/dev/fuse` is absent. The previous `worklog/2026-10-02-T12-safe-eio-fixture-audit.md` accurately records that host-level limitation.
- After Docker access was granted for the authorized disposable test lanes, `rtk run docker info` succeeded: Docker Desktop 29.8.0, LinuxKit 7.0.12, `aarch64`, 18 CPUs.
- A no-mount probe using the already-local BusyBox image (`dc2d74b28e4c`) ran as an ephemeral privileged container with no host/project mounts. The exact probe `docker run --rm --privileged busybox:latest sh -c 'test -c /dev/fuse && echo FUSE_DEVICE || echo NO_FUSE_DEVICE; mount -t proc proc /proc 2>/dev/null || true; grep fuse /proc/filesystems || true'` reported `FUSE_DEVICE`, `fuseblk`, `nodev fuse`, and `nodev fusectl`. The container exited successfully.
- A second ephemeral privileged Ubuntu 24.04 ARM64 container installed `fuse3`, `libfuse3-dev`, and `build-essential`, mounted a minimal libfuse filesystem at `/mnt/probe`, opened `/mnt/probe/probe`, and called Python's real `os.fsync()`. It returned `OSError.errno == 5` (`EIO`). This proves the kernel syscall boundary in the Docker VM, but does not yet prove `RunArtifacts` behavior inside the mount. The probe container exited without mounting the workspace or changing host files.
- First mount attempt without a connected userspace daemon returned `EINVAL`; this was expected protocol behavior, not a mount-permission rejection. The successful follow-up ran a real libfuse daemon.

## Acceptance contract before coding

Prototype a private, disposable FUSE filesystem that returns `EIO` only from its `fsync` callback for the named report artifact. First require an actual mount and a raw `fsync(2)` probe to fail with errno 5; fail closed on success or any other errno. Then exercise `RunArtifacts::write_report` on that exact mounted path, prove the syscall-stage error is `EIO`, assert its result is non-success and the prior durable `report.json` bytes remain unchanged, capture mount/container ownership, and unmount/remove only that owned fixture. If any boundary cannot be proven, leave the gate open and do not label the result physical-sync acceptance.

## Rust acceptance result

- The Linux-only ignored integration case is registered in `tests/integration.rs` and uses `tests/support/t12_fuse_eio.c` as a small libfuse passthrough fixture. The fixture first returns `EIO` for a raw kernel `fsync` probe, then arms one `EIO` for the next `report.json.tmp` sync callback.
- Ran inside Docker Desktop LinuxKit ARM64 with Rust 1.99.0 and Debian bookworm `fuse3`, `libfuse3-dev`, and `build-essential`; repository mounted read-only and no database services started:
  `cargo test --locked --test integration t12_fuse_eio::linux_fuse_fsync_eio_preserves_the_last_durable_report -- --ignored --exact --nocapture --test-threads=1`
- Result: **1 passed, 146 filtered out**. It confirmed mount identity, raw `fsync` errno 5, durable baseline creation, propagation of errno 5 from `RunArtifacts::write_report`, unchanged prior report bytes, removal of the failed temporary report, and cleanup of the owned mount/daemon/root.
- The fixture owns a unique temporary root and verifies the mount path, FUSE filesystem type, device boundary, and daemon source before unmounting. Setup and teardown retain cleanup ownership on failure; `Drop` reports cleanup errors without masking the test panic.
- This closes the local physical `fsync(EIO)` acceptance case at this working-tree revision. It does not establish execution in hosted CI; the test remains ignored and requires a privileged Linux runner with `/dev/fuse` and libfuse3.
