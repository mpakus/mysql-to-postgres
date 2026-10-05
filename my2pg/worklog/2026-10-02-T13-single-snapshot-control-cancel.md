# T13 — cancel the SingleSnapshot server query

Status: the bounded SingleSnapshot cancellation fix is accepted on the focused
cases and refreshed MySQL 8.4/PostgreSQL 16 required lane. Broader T13 shutdown
and concurrency acceptance remains open.

## Reference research (before code)

- Project query: `SingleSnapshot source ID cancel external metadata lock MDL control KILL CONNECTION`, prefix `project-my2pg`, lexical XERJ at pinned project revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Relevant current source: `src/pipeline/mod.rs` lines 923–954 (`ExecutionRegistry::cancel_sources` sends exact `KILL CONNECTION` to registered Frozen readers), lines 1257–1284 (SingleSnapshot runs one `table_pipeline` on the shared snapshot source without registration), and lines 545–575 (fault shutdown drops the source only after cancellation). `src/mysql/mod.rs` lines 15–60 wraps `mysql_async::Conn`, exposes `id()`, and closes the client socket on drop. `src/pipeline/scheduler.rs` lines 184–204 charges one normal source connection for SingleSnapshot.
- Native oracle: `tests/cases/t13_snapshot_mdl.rs` lines 244–390 reaches a real SELECT waiting on fixture-owned MySQL MDL, signals cancellation, confirms the same server thread is still waiting when the run ends, then releases the lock and waits for that thread to disappear. The change will reverse the expected behavior: the exact reader ID must disappear while both the lock-holder and its MDL remain live, before run completion is accepted.
- Reference query: `connection id kill connection cancel statement`, prefix `ref-mysql_async`, pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`. Search found connection ID access in `src/conn/mod.rs` and no cross-session server-query cancellation helper. The local MySQL wrapper uses the driver's connection ID directly; no driver patch is warranted.
- Reference query: `cancel source connection exact ID shutdown reader`, prefix `ref-dmt-rs`, pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. Search found bounded transfer orchestration and generic shutdown paths but no MySQL exact-ID kill pattern. Reuse the project's existing bounded shutdown deadline and keep cancellation in the MySQL pipeline boundary.
- Official MySQL 8.4 KILL documentation: `https://dev.mysql.com/doc/refman/8.4/en/kill.html`. `KILL CONNECTION` terminates the selected connection; its acknowledgement can precede server-side thread disappearance. Therefore the independent native processlist observer must verify the captured ID is gone before releasing the test lock. This behavior is covered by the existing test fixture and must not be inferred from the KILL result alone.

## Chosen adaptation and acceptance

Use a pre-opened, otherwise-idle MySQL control connection for SingleSnapshot
preflight. Preserve the original snapshot connection and its read view; store
that exact connection ID for terminal cancellation and issue `KILL CONNECTION
<id>` through the separate control session before dropping the snapshot
socket. Do not run reads through the control session. Frozen reader cancellation
remains unchanged. Charge the two-source-session preflight peak for all
SingleSnapshot plans; no-data/schema-only plans release the control session
after preflight. Keep the bounds diagnostic and architecture documentation
aligned.

The native T13 test observes the same source backend disappear while the
external lock holder remains present and still owns the lock, then verifies
the runner reports cancellation with zero committed rows and exit code 130.
Run the focused T13 case, the full MySQL 8.4/PostgreSQL 16 required lane,
resource-admission unit tests, strict Clippy, formatting, and offline support
tests. Record any KILL failure in the existing source-cancel diagnostic path;
never interpret a successful request as proof of thread exit.

## Implementation and evidence

- The pre-fix required MySQL 8.4/PostgreSQL 16 lane failed exactly at the new
  oracle: after the runner returned, the same ID still showed `Waiting for
  table metadata lock`. The owned fixture was cleaned up by the harness.
- `run_with_artifact_executable` opens the control connection during
  preflight, before target mutation, for every SingleSnapshot run so catalog
  reads are cancellable before the plan is known. On any execution fault it issues KILL for the captured original
  source ID, polls that exact ID through `information_schema.processlist`
  under the shared shutdown deadline, then closes the snapshot reader. The
  control connection never executes migration reads. Failure to kill or
  observe shutdown feeds the existing `SOURCE_CANCEL` warning.
- Admission reports the two-socket SingleSnapshot preflight peak for every
  SingleSnapshot plan. Schema-only/no-table runs release the extra control
  session after preflight; data-bearing runs retain it for execution.
- `bin/cargo fmt --check` and the focused scheduler admission unit test pass.
  The first focused native regression passed on MySQL 8.4/PostgreSQL 16: it
  held the external MDL until the exact reader exited, then confirmed exit 130
  and zero rows (`0.433s` report elapsed; Cargo test 1 passed, 157 filtered).

## Independent review and follow-up (before follow-up code)

- Project XERJ queries at `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`:
  `preflight cancellation before registry catalog snapshot cancellation source
  lock` and `single_snapshot source control source ID lifecycle cancel`. The
  first ranked the existing SingleSnapshot characterization and shutdown
  worklogs; the second returned test/config passages, with no preflight
  cancellation implementation to reuse.
- The independent review inspected the current uncommitted change and found
  that the runner creates `ExecutionRegistry` only after preflight at
  `src/pipeline/mod.rs:407-543`; cancellation during source catalog recheck
  could therefore bypass the new KILL path. It also found the ordinary fault
  branch awaited `cancel_all()` outside the shared deadline and that the
  native test checked only the persisted report's exit code.
- Follow-up adaptation: keep the SingleSnapshot control session and captured
  source ID in a shared preflight-lifetime cancellation handle, invoke the
  same bounded KILL-and-observe operation if preflight is interrupted, and
  retain that handle for execution faults. Apply the existing shutdown
  deadline to the final target cancellation join. Compare persisted table
  accounting and failure fields with the in-memory report. Preserve the
  fixture's external-MDL test as the independent behavioral oracle.

## Follow-up implementation and evidence

- The shared handle is installed immediately after source connection setup,
  before catalog queries. Cancellation or failure during preflight now kills
  and observes the exact source ID before returning. Data-bearing execution
  keeps the same handle; artifact/admission/setup errors and cancellation
  before worker registration also terminate the idle snapshot connection.
  The ordinary target-cancel join is bounded by the same 10-second deadline.
- Added a second required test that adds a literal-default column only to its
  own source fixture, holds a write MDL before `run`, then cancels the
  preflight catalog's real SELECT. It verifies the source ID disappears while
  its lock-holder remains live and confirms preflight returns exit 130. The
  existing execution-time fixture stays free of that default column so the
  later snapshot catalog read cannot hold an idle transaction MDL and mask
  the intended data SELECT.
- Final focused native run: both `t13_snapshot_mdl` cases passed on a fresh
  MySQL 8.4/PostgreSQL 16 pair (2 passed, 157 filtered, 1.77 seconds). The
  execution-time case persisted exit 130, zero read/committed/indeterminate/
  unresolved/rejected rows, and exact agreement between returned and saved
  reports. RESOURCE_ADMISSION reported two normal source sockets.
- Final local checks after the preflight extension: formatting, strict
  all-target/all-feature Clippy, 122 library tests passed (3 intentionally
  ignored), 7 Python harness tests, and `git diff --check` passed. The full
  suite is recorded below.
- At the original T13 implementation revision, the MySQL 8.4.11/PostgreSQL
  16.15 required lane passed: 156 aggregate cases, exit code 0, all 26
  applicable required IDs exactly once. This predates reviewer follow-ups and
  is historical evidence. Evidence:
  `target/integration/my2pg-mysql84-pg16-dfcf1d05dbbf/rust-tests.json`
  SHA-256 `08519ade55de1454e451dfa3ad072834cbd90792780c3be9d30aa25634ed0e44`;
  test log SHA-256
  `f9326199ca7fb71d0c44b4fb3d8d3123680059779d0a888b4db0a997c29e4175`.
- Coordinator refreshed `project-my2pg` through XERJ after implementation:
  377 files, 1,320 passages, 11 exclusions. Representative final searches for
  `SingleSnapshot preflight cancellation control KILL CONNECTION processlist`
  and `cancellation_terminates_single_snapshot_reader_blocked_during_preflight`

## Reviewer follow-up: report unconfirmed preflight termination (before code)

- Project XERJ query: `SingleSnapshot cancellation preflight control connection`,
  prefix `project-my2pg`, pinned revision
  `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Results point to this worklog,
  `src/pipeline/mod.rs` around source-ID registration, and the two blocked-query
  tests; no separate implementation reports KILL failure to the caller.
- Reference XERJ query: `connection setup driver connect`, prefix
  `ref-mysql_async`, pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`. The
  original `src/opts/mod.rs` documents that `after_connect` runs before `init`
  queries and that an error fails connection initialization. This is not an
  exact cancellation pattern; the useful boundary is that driver initialization
  remains a bounded operation before our wrapper can expose `Conn::id()`.
- Current implementation and adjacent native tests read: `src/pipeline/mod.rs`
  `PipelineError`, `SnapshotCancellation`, and preflight cancellation/error
  branches; `src/mysql/mod.rs::connect` and its 10-second initialization
  deadline; `tests/cases/t13_snapshot_mdl.rs` preflight and execution MDL tests.
- Chosen adaptation: register the source ID/control session immediately after
  MySQL initialization and before PostgreSQL connection setup. On every
  preflight exit, preserve the existing safe error/exit code but append a
  generic warning when exact server-side termination cannot be confirmed.
  Do not report credentials or server messages. The driver's own MySQL
  initialization stays bounded at ten seconds when it runs to completion;
  before it returns, an ID is not available to our cancellation handle.
- Independent review worklog: `worklog/2026-10-02-T13-single-snapshot-cancel-design.md`.
  Its preflight, timeout and persisted-report findings are addressed here.

## Reviewer follow-up: cancellation during source initialization (before code)

- Project XERJ query: `cancelled during mysql connection initialization source
  id`, prefix `project-my2pg`, pinned revision
  `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Results led to the current
  cancellation worklog, source transport, and worker-fault tests; no
  initialization-stage cancellation marker exists.
- Reference XERJ query: `connect initialization after_connect init timeout`,
  prefix `ref-mysql_async`, pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`.
  The original options documentation confirms `after_connect` precedes driver
  `init`; `src/conn/mod.rs` shows connection setup is driver-owned. There is no
  pre-ID cancellation hook in this project's current wrapper.
- Independent review found that dropping `mysql::connect` cancels its wrapper
  timeout too. Since no server ID exists yet, cleanup cannot know whether a
  backend was opened. Chosen adaptation: mark source
  initialization as in progress before awaiting the driver and leave that state
  set if the preflight future is dropped. Cleanup must then report that source
  termination could not be confirmed, while retaining the bounded driver
  initialization behavior. Clear the marker on a returned success or error and
  register the ID synchronously before the next await. The native post-connect
  MDL cases remain the live server-query proof; the new unit oracle covers the
  unknown-ID initialization state. Rerun them and the affected database lane.

## Follow-up implementation and verification

- `SnapshotCancellation` now marks MySQL initialization before awaiting the
  driver. If cancellation drops that future, the marker remains set and the
  returned exit-130 error says server-side termination could not be confirmed.
  A completed connection registers its ID synchronously before another await;
  any connection error retains the uncertainty marker because session setup
  can fail after the server has assigned an ID that the wrapper does not expose.
  Closing the control handle for a schema-only/no-table plan also clears its
  now-inapplicable ID.
- All preflight cleanup exits preserve the original exit status and append the
  generic confirmation warning if KILL or processlist observation fails.
  Database errors and credentials are not included in that message.
- Updated-source checks: formatting; 124 library tests passed with three
  intentionally ignored; strict all-target/all-feature Clippy; seven Python
  harness tests; and `git diff --check`.
- Latest-source MySQL 8.4.11/PostgreSQL 16.15 focused lane: both required
  `t13_snapshot_mdl` cases passed (2 passed, 157 filtered, 1.91 seconds).
  The harness stopped its owned fixture project after the run. The earlier full
  MySQL 8.0/PostgreSQL 16 run passed 156 cases and all 26 required IDs, but its
  test binary may predate the final initialization marker; it is not used as
  evidence for this latest-source change. The full MySQL 8.4/PostgreSQL 16 lane
  recorded above validates the preceding T13 implementation.

- A refreshed full MySQL 8.4.11/PostgreSQL 16.15 lane then passed after the
  initialization uncertainty handling, source-socket admission correction and
  nullable Frozen processlist-oracle fix. It passed 156 cases and all 26
  required IDs exactly once with no failures; executable hash matched the
  current binary. See [reviewed-source lane evidence](2026-10-02-T22-mysql84-pg16-reviewed-source-refresh.md).
- Coordinator refreshed the project XERJ index after the final code/docs/tests:
  378 files, 1,326 passages, 11 exclusions at HEAD
  `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Representative searches for
  `source_connecting preflight shutdown unconfirmed` returned this worklog and
  `src/pipeline/mod.rs`; `156 26 reviewed source lane` returned the reviewed
  T22 lane evidence and updated checklist.

## Review follow-up: conservative session-initialization failures (before code)

- Project XERJ query: `mysql connect session initialization fails connection ID
  register`, prefix `project-my2pg`, index revision
  `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. It pointed to T14 session setup
  tests and the source transport; neither verifies exact backend termination
  after a post-handshake SET fails.
- Reference XERJ query: `Conn new session init query error connection id`,
  prefix `ref-mysql_async`, pin `d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5`.
  The original `Conn::id` at `src/conn/mod.rs:245` becomes available after the
  handshake, while `src/mysql/mod.rs:261-288` runs configured session SET and
  validation queries after `Conn::new`. The wrapper currently does not return
  that ID when one of those setup queries fails.
- Review also noticed the unit test still exercised a registered ID without a
  control session, not the in-progress marker. Chosen adaptation: test the
  in-progress state directly and keep it set on every MySQL connect error,
  because the wrapper cannot distinguish pre-handshake errors from a live
  backend whose session setup failed. Cleanup then reports termination
  unconfirmed instead of claiming success. Exporting the ID through every
  post-handshake error path is a possible stronger follow-up; this change makes
  the current boundary explicit and fail-safe.

## Review follow-up: charge preflight control socket (before code)

- Project XERJ query: `SingleSnapshot preflight temporary control socket
  admission schema only`, prefix `project-my2pg`, index revision
  `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Results locate
  `src/pipeline/scheduler.rs` and this T13 worklog's earlier one-socket claim.
- Reference query: `connection admission fixed concurrency source sockets`,
  prefix `ref-dmt-rs`, pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`. It found
  operational concurrency reporting but no equivalent preflight-control
  reservation. The relevant evidence is local: scheduler lines 191-194 count
  one source socket for SingleSnapshot with zero selected tables, while
  `run_with_artifact_executable` opens a separate control socket before source
  catalog inspection and cannot know the plan yet.
- Review found that schema-only/no-table admission and architecture text
  understated the preflight peak: two source sockets are concurrently open
  until the plan is known. Adaptation: charge two source sockets for every
  SingleSnapshot preflight, including schema-only/no-table plans; describe the
  planned pipeline retention separately from preflight peak. Update admission
  expectations and rerun the scheduler test plus native lane.
