# T12 — recovery and durability acceptance closeout

Status: the T12 implementation acceptance is complete on the current integrated source. Hosted CI and wider M4 release-matrix gates remain separate.

## Acceptance audit

The T12 task asks for row-recovery behavior, SQLSTATE classification, stop/reject limits, partial commits, reject durability, indeterminate commit handling, and passing database fault cases. The current evidence covers each of those behaviors:

- The six current-source MySQL 8.0/8.4 → PostgreSQL 16/17/18 base lanes each passed all 156 registered integration cases and every applicable required case exactly once; see the [current-source matrix reruns](2026-10-02-T22-current-source-matrix-reruns.md) and [current PR lanes](2026-10-02-T22-current-pr-lanes.md). The native MySQL 8.4/PostgreSQL 16 lane also passes the French localized builtin error/SQLSTATE case in the [NLS refresh](2026-10-02-T22-nls-current-refresh.md).
- The [recovery suite](2026-10-02-T22-mysql84-pg16-reviewed-source-refresh.md) and [runner suite](2026-10-02-T22-mysql84-pg16-current.md) exercise bisection, retry classification, limits, stop/reject behavior, partial commits, and artifact failure through the actual pipeline/CLI.
- The [lost-COMMIT-ack test](2026-10-02-T12-lost-commit-ack.md) observes PostgreSQL completing COMMIT while the client receives no acknowledgement, then proves the run remains indeterminate and does not replay.
- The [uncertain-storage test](2026-10-02-T12-uncertain-storage.md) combines sent-COMMIT uncertainty with actual ENOSPC while publishing the next report, preserving the last acknowledged artifact and avoiding replay.
- The [native physical ENOSPC tests](2026-10-01-T12-durable-faults.md) prove report and reject persistence failures preserve the durable prefix and poison subsequent reject writes. The child-only descriptor test proves actual `fsync(EINVAL)` without calling it physical media failure.
- The [Linux FUSE EIO case](2026-10-02-T12-fuse-eio-prototype.md) now passes against the final helper version: the kernel `fsync` returns EIO at the production report writer, the prior durable report remains byte-for-byte unchanged, and the failed temporary report is removed. The current follow-up T13 case also exercises this helper with the production artifact-worker protocol.
- Required DDL failure is independently covered in the [required-DDL worklog](2026-10-02-T12-required-ddl-failure.md), and [reject collision/artifact failures](2026-10-01-T12-reject-collision-report.md) retain acknowledged rows while reporting failure.

The coordinator reran the T12 FUSE EIO control alongside the T13 production-worker stalled-filesystem case after the final C helper edits; both passed in the pinned privileged Linux/arm64 container. The native matrix and NLS runs used the same dirty source snapshot `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` before this test-only T13 helper change. No T12 production code changed during the follow-up.

This closes T12's implementation acceptance. It does not claim a hosted Actions run, MySQL 5.7 support, additional native architectures, or release certification; those remain T22–T24 gates. The task board now marks T12 complete.
