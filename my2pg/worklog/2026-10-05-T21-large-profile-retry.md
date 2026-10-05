# 2026-10-05 — T21 — large-profile correctness retry

- Status: passed — both loaders matched the independent 10-million-row correctness oracle; timing remains ineligible
- Agent/role: coordinator / T21 evidence
- Task and contract: [T21 task board](../docs/agent-tasks.md#task-board), [large-profile corpus and oracle](../docs/testing-and-performance.md#proving-faster), [performance runner](../tests/performance/runner.py)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Project/XERJ snapshot: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Scope: one 10,000,000-row `one_large_narrow_integer` correctness-only attempt; no timing will be collected or claimed

## Preflight and method

Project XERJ query `target_oracle target_canonical_query ORDER BY id stream_digest database memory` returned the MySQL/PostgreSQL canonical queries and oracle code at `tests/performance/runner.py:280-342,344-444`, plus adjacent tests. The prior artifact was re-read: pgloader completed COPY, then PostgreSQL's target-oracle backend received signal 9. The fixture caps PostgreSQL at 512 MiB. `tests/performance/corpus.json` fixes this workload at 10 million rows for `large` and warns that memory profiles require capacity planning.

The local Docker engine reported 7,746 MiB available to containers and 18 CPUs. Immediately before launch, six existing unrelated containers used about 495 MiB total and were nearly idle. The owned fixture retained its 1 CPU/1 GiB MySQL service and 512 MiB/1 CPU loader-container limits; only its PostgreSQL service was raised from 512 MiB to 2 GiB for this correctness-only run so the target content oracle had headroom. Docker swap was capped at the same 2 GiB memory limit. The runner's saved resource inspection records these actual limits. This intentionally unequal database limit makes the attempt timing-ineligible; it did not alter either loader's 512 MiB process limit.

Started owned fixture `my2pg-mysql84-pg16-50e714bf164b` (MySQL 8.4.11/PostgreSQL 16.15), verified the PostgreSQL container's exact Compose project/service/integration labels, raised only that container from 512 MiB to 2 GiB with swap capped at the same 2 GiB, ran `tests/performance/runner.py --correctness-only --workload one_large_narrow_integer --profile large` with both pinned adapters, and stopped the fixture through the owner-checked harness.

## Acceptance and limits

- Pass: both adapters completed and matched all independent correctness checks over 10 million rows; the result remains `timing_eligible=false`.
- Fail/incomplete: a loader or oracle exits, a correctness mismatch occurs, or any required report/artifact is missing. Preserve the failed run and the exact failing stage.
- This does not close comparable-phase timing, matched database-control, repeated 1+5 timing, memory-profile, or dedicated-host gates.

## Result

- Result artifact: `target/integration/my2pg-mysql84-pg16-50e714bf164b/t21/one_large_narrow_integer-large-10371f64178e/results.json`; SHA-256 `a3b6ee9fc20fcb9aa6b46553283bb82a7bfc9e0fcf23f833b9e95f8afe40c42b`.
- The independent source and target each produced 10,000,000 rows, the same SHA-256 `a55a04c47f49b974c992a2b6a1e70ee518878383efd40e4196a012b4a2da707a`, aggregate `10000000,50000005000000,5114877760,10729903196310280,10734264702998009,1,10000000`, exact four-column shape, and `id` primary key. Both loader records say `correctness=passed`, exit code 0.
- pgloader image: `my2pg-reference-v4:231ab867`, image ID `sha256:c0d9e27a869deb46df9f94c2487dfb01e60a6fbf41785ef844d13655cd4375e9`, extracted JAR SHA-256 `2edad404ff29a7cedda1e7cacb48b806287a79a3e87112856d137244467835ce`.
- My2pg source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`, container image ID `sha256:0264614c8559c41361e89a76cbfc91a9dafcac90d69c8ba125ae813cbf6389b4`, binary SHA-256 `8de4331caf9eaa0a70149387e6568de55b6df48f02be6239e3783e494940261a`.
- Result records PostgreSQL at 2 GiB, MySQL at 1 GiB, and both loader containers at 512 MiB/1 CPU. Because fixture resource limits were intentionally unequal and phase metrics remain unsupported/incomplete, `timing_eligible=false`; the approximately 16.2-second run durations are not performance evidence.
- Independent target verification completed in about 5.4 seconds for each loader. PostgreSQL's observed peak during the pgloader run was 374,446,489 bytes; My2pg's was 54,316,236 bytes. These correctness-run samples are diagnostic only, not comparative RSS evidence.
- pgloader native summary SHA-256 `c7771dd3df2d405a8e6ed3937f73b8ff4b6f802369b8f90d1d1e124592f3d348`; My2pg report SHA-256 `0b470f87488b849fa522326081acdb8e3005fb5a89a238c6672f2f2bc80292c3`. Container log SHA-256 `29a2c007d85f2e4e579a7ba06afef77e653c9be6af7f8de079bc26fdcc72c405`.
- After cleanup, `connections.json` reports `state=stopped` (SHA-256 `017d7534584e12578c70f4e7e66d055f57d732acc1841de60b733433ba4989ce`) and an exact project-label inventory returned no remaining containers.
- Post-run validation: T21 performance runner tests passed 44/44; package documentation-link tests passed 3/3; `rtk git diff --check` passed. No source code changed in this step.
- Coordinator refreshed `project-my2pg-source` after the result and checklist updates: 445 files, 1,537 passages, 14 exclusions. Query `10 million row correctness passes 2 GiB PostgreSQL large profile remains timing ineligible` returned this retry, the failure reconciliation, and the checklist at indexed revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; a final refresh follows this evidence entry.
