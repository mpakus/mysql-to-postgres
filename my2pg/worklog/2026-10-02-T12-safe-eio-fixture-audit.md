# T12 — safe physical `fsync(EIO)` fixture audit

Status: **historical capability audit**. At the time, the macOS host lacked an accessible FUSE device and Docker socket. A later approved Docker probe found FUSE in the Linux VM; see the [current FUSE EIO prototype worklog](2026-10-02-T12-fuse-eio-prototype.md). This audit adds no production or test code and does not close the physical-sync gate.

## Reference research

Followed `docs/reference-coding.md` before deciding whether a fixture or case was viable. The current my2pg checkout and project XERJ index both report revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Searches used:

- `project-my2pg`: `physical storage sync EIO fsync test fixture` — returned `worklog/2026-10-02-T12-physical-sync-eio.md`, `worklog/2026-10-01-T12-durable-faults.md`, and related acceptance notes.
- `ref-rust-postgres`: `flush sync file EIO storage error` — pinned revision `1084ca8f5b5302e161892f2fa40abf71b4060c10`; results were protocol encoding and SQLSTATE files, with no local file-sync implementation or fault fixture.
- `project-pgloader`: `fsync EIO durability failure` — pinned revision `231ab86778ca5ffd7de40878714760c8b4860cdf`; results concerned COPY retry, batch processing and parser code, not artifact persistence.
- `ref-dmt-rs`: `fsync EIO failpoint` — pinned revision `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`; no filesystem fault-injection implementation was found.

Read the current originals and adjacent tests:

- `tests/support/durable_faults.py:40-135` verifies the exact owned image/device/mountpoint, reaches native `ENOSPC` on that image, and calls real `os.fsync` before and after forced detach. The post-detach sync succeeds; it is not EIO evidence.
- `tests/cases/t12_durable_faults.rs:107-165` exercises production report and reject writers under image exhaustion and verifies preservation of prior durable bytes. The neighboring descriptor-only case at `:260-390` replaces a verified child descriptor with a Unix socket and receives native `EINVAL` from `fsync`; its own assertions label this as a descriptor fault.
- `src/report/mod.rs:144-165,217-268` calls `sync_all` for reject data, reject metadata, report documents and directory persistence. The test needs to fault this actual storage syscall to prove the open gate.
- Pinned `references/rust-postgres/tokio-postgres/src/copy_in.rs:128-142` and `tests/test/main.rs:721-764` address PostgreSQL COPY protocol completion/abort, not filesystem durability. Pinned `pgloader/src/pg-copy/copy-retry-batch.lisp:1-90` handles COPY batch failure/retry, not artifact sync. No source code or fixture is adapted.

RTK was used for all commands and source reads. Ponytail could not be applied: there is no `ponytail` executable or matching skill file in the configured Codex/Agents skill roots. No code was written, so this did not block a code change.

## Non-invasive host capability checks

Commands were run from the workspace with `rtk run` on 2026-10-02:

- `uname -a` reports Darwin 27.0.0 on ARM64.
- `ls -l /dev/fuse /dev/macfuse` reports both paths absent. `command -v macfuse mount_macfuse sshfs fuse-t fuse-overlayfs podman` returns no executable.
- The live `mount` output contains local APFS and HFS volumes only; no NFS, SMB, WebDAV, or FUSE mount can return a remotely injected storage error.
- Docker CLI is installed, but `docker info` and `docker ps` fail with `permission denied` on `/Users/renatibragimov/.docker/run/docker.sock`. A Linux-VM FUSE fixture therefore cannot be started without changing Docker access or daemon state.
- `hdiutil` can create/attach a disposable HFS+ disk image, but it exposes no fault-injection option. Existing owned-image evidence shows that exhaustion returns `ENOSPC` during writes and forced detach does not make a subsequent sync fail. Corrupting an image or inducing device failure would add risk without ensuring `EIO` at `fsync`.
- The system `diskutil` invocation cannot use the DiskManagement framework in this session. No disk utility or system-volume action was attempted.

The only realistic isolated kernel path identified is a FUSE filesystem whose `fsync` operation replies with `EIO`, producing an actual kernel `fsync(2)` failure on a file in that mount. This host has neither the FUSE device/driver nor permission to use its Docker VM, so reaching that path would require system or daemon changes outside this task's safe, owned facilities. A userspace wrapper, interposed syscall, fake errno, socket descriptor, post-detach descriptor, or relabeled `ENOSPC` would not prove the requirement and is rejected.

## Result and handoff

No new case or helper was created because there is no safe facility on which it could execute and pass. Keep T12's physical-sync checklist gate open. To close it, provide a CI/test host with an already-enabled, isolated faulting filesystem (for example, a disposable Linux FUSE mount) and register a case that verifies the exact production artifact FD and mount identity, observes errno `EIO` from the real `fsync`, confirms the nonzero surfaced run result and unchanged previous durable report, then unmounts only its owned mount. The fixture must fail on success, `ENOSPC`, `EINVAL`, or any errno other than `EIO`.

Validation: read-only searches and host capability probes only; no image was created or mounted, no database fixture was started, and no tests were rerun because there is no fixture to exercise.
