# 2026-10-02 — T24 — independent core and evidence review

## Review result

The independent read-only review found no concrete loss, duplication, or
source-scope defect in the bounded commit-ambiguity path it examined. The T12
lost-ack test withholds the acknowledgment after PostgreSQL `CommandComplete`
for COMMIT, records the batch as indeterminate, independently observes the
exact target rows, and confirms the pipeline does not replay them. The reviewed
source and architecture paths remain MySQL-only; no alternate source adapter
was found in the paths reviewed.

The review did find two acceptance/documentation issues, now corrected:

- SingleSnapshot preflight opened two MySQL sockets before the selected plan
  was known, but schema-only/no-table admission reported one. The scheduler
  now charges the two-socket preflight peak for every SingleSnapshot plan, and
  architecture/checklist/worklog text distinguishes preflight peak from the
  post-preflight sessions retained by a data run.
- The older T22 153-case/23-ID note and T13 open-gate descriptions could be
  mistaken for current evidence. They are labeled historical or updated to
  leave only broader shutdown/concurrency work open. The current reviewed
  MySQL 8.4.11/PostgreSQL 16.15 lane is linked separately.

The nullable processlist test failure and subsequent T16 event-trigger failure
were also traced: the observer panicked converting nullable `STATE`, which
skipped fixture cleanup. Optional decoding and ID-only disappearance polling
fixed the test; the refreshed full lane passed.

## Evidence and limits

- Independent agent review read the source, task checklist and retained lane
  artifacts. It did not edit files or run database tests.
- The reviewed-source MySQL 8.4/PostgreSQL 16 lane reports 156 passing cases,
  no failed IDs, and all 26 applicable required IDs exactly once. See [the T22
  lane report](2026-10-02-T22-mysql84-pg16-reviewed-source-refresh.md).
- This does not finish T12 physical `fsync(EIO)` coverage, broader T13
  shutdown/concurrency tests, T21 equivalent benchmark targets, other T22
  version/platform cells, NLS/fuzz/soak/CI gates, or T23 packaging/provenance
  and human notice review. T24 remains planned until those prerequisites and a
  release-candidate review are complete; no release recommendation is made.
