# 2026-10-01 — T02 — classified fixtures and isolated database harness

- Status: review
- Agent/role: G (fixtures_harness)
- Task: [T02](../docs/agent-tasks.md), [test contract](../docs/testing-and-performance.md)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Dependencies: T01 shared Rust contracts being established by coordinator; harness preparation is independent.
- Claimed files: `tests/fixtures/`, `tests/compose/`, `tests/support/`, `tests/run-integration.sh`, this worklog.
- Guidance read: repository AGENTS.md, reference coding, scope, source audit, tests/performance and task docs; Ponytail SKILL.md and RTK.md. No separate RTK SKILL.md exists in the supplied catalog; commands use installed RTK guidance/wrappers.

## Reference research (before coding)

| Query/prefix | Revision and original files | Observed behavior | Adaptation and independent check |
| --- | --- | --- | --- |
| `fixture harness readiness MySQL`, project-my2pg | working documentation, testing-and-performance.md:69–100 and fixture catalogue | Required explicit integration execution, unique resources, scoped teardown, loopback ports and modern 8.4 authentication | Standard-library Python wraps native Compose; requested lane errors are fatal; seed smoke asserts actual values and authentication. |
| `mysql_native_password healthcheck`, project-pgloader | `231ab86778ca5ffd7de40878714760c8b4860cdf`, clojure/tests/mysql/docker-compose.yml:1–50, data/load-datasets.sh:1–40 and adjacent Makefile | Upstream changes authentication for Lisp v3 and global SQL mode; health waits for all data; target contains PostGIS | Preserve Oracle MySQL defaults; permissive SQL mode only in deliberate seed session, plain PG target, modern account asserted, health/readiness then deterministic seed. |
| `docker compose mysql postgres`, ref-dmt-rs | `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`, docs/benchmarks.md:180–265 | Fixed names/ports and performance-specific durability changes; no relevant general integration helper returned | Reject fixed shared names and tuned durability; use unique Compose ownership labels, ephemeral loopback ports, defaults and digest pins. |
| Fixture definitions inspected | pgloader revision above, all selected MySQL `.load` files, parser_test.clj/ast_test.clj, unified SQL/assertion files, test/README.md and LICENSE | Citus, source-view creation, option files and Mustache differ from our product; consolidated regression overlaps older cases | Classify each selected file and parser case; map original behavior to independent case IDs, preserve unsupported tests and attribution instead of importing unrelated drivers or expected output wholesale. |

Initial sandbox XERJ/Docker access failed with permission errors; normal tool escalation allowed local loopback/socket reads. Existing Strangler containers are outside our ownership and have not been modified. Docker Desktop server is linux/arm64, engine 29.8.0.

## Intended result

Pinned Oracle MySQL 8.4 and PostgreSQL 16 containers seed synthetic relational and value-edge cases, publish connection metadata, assert real server values and account authentication, and teardown only the project they created. Every selected upstream MySQL load fixture has revision/hash/license, classification rationale and independent case IDs. Baseline attempts record actual build/run outcomes and limitations without claiming performance.

## Verification

| Check | Command/evidence | Result |
| --- | --- | --- |
| Fixture pin/inventory/provenance | `rtk run 'python3 tests/support/fixture_manifest.py --upstream'` | 13 load files, 18 MySQL parser cases, 19 assertion cases, 65 case IDs; 40 immutable snapshots and notices; all SHA-256 checked. Local-only command also passes without sibling access. |
| Harness security checks | `rtk run 'python3 tests/support/test_harness.py'` | 4 passed: refuse foreign project, missing ownership label, unpinned requested lane; seed mismatch fails. |
| Syntax | `PYTHONPYCACHEPREFIX=target/python-cache python3 -m py_compile tests/support/*.py`; `sh -n tests/run-integration.sh` | Passed; default macOS Python cache location initially denied by sandbox, task-local cache succeeds. |
| Initial startup/seed | `tests/run-integration.sh mysql84 pg16 --start` | Real MySQL8.4.11 / PG16.15; native arm64; 12 independent SQL seed/auth/TLS assertions passed. |
| Rust TLS/values | `bin/cargo test --locked --test integration -- --ignored --nocapture` with harness env | 1 passed, 0 failed; both Rust driver connections verified the CA and negotiated TLS; exact decimal/u64/all256bytes values and public sentinel pass. |
| Rust missing environment | same ignored test with all MY2PG_* variables removed | Fails explicitly with missing integration variable; no successful skip. |
| Rust lint | `bin/cargo clippy --locked --test integration -- -D warnings` | Passed. |
| Startup→test→teardown | initial `tests/run-integration.sh mysql84 pg16` | 12 seed checks and 1 Rust test passed; owned project `my2pg-mysql84-pg16-f31b822f7f2e` stopped and metadata records stopped. |
| Automatically discovered DB suites | expanded `bin/cargo test --locked --tests -- --ignored --nocapture` | Initial run discovered 6 C/T03/T05 tests; 4 passed, 2 failed due actual MySQL SET-label and CHECK/FK fixture restrictions. Harness returned failure and cleaned its project. C corrected its fixtures; fresh rerun outcome below. |
| Fresh automatic discovery after C fixture fixes | owned run `my2pg-mysql84-pg16-be7dcea9b1ec`, `rust-tests.log` and `rust-tests.json` | 5 passed, 1 failed: `mysql_owned_stream_backpressure_and_cancellation` timed out draining unread MySQL results during disconnect. T05 catalog/raw-charset/snapshot/mTLS cases pass. C informed and owns this T03 behavior fix; harness correctly returns nonzero and stops resources. |
| Final fresh automatic discovery after actual socket-close correction | owned run `my2pg-mysql84-pg16-d090c32b6831`, `rust-tests.log` and `rust-tests.json` | **9 executed passed, 0 failed**: 8 T03/T05 tests plus independent TLS fixture smoke. MySQL cancellation no longer drains results; all prior seed assertions pass. Startup/test/scoped-cleanup complete; metadata records stopped. |
| Upstream preservation | `git -C ../pgloader status --porcelain` | Empty: no reference source edits. Existing Strangler containers unchanged. |

## Changes and artifacts

- `tests/compose/`: two-service Compose, reviewed digest registry and runnable usage/baseline commands.
- `tests/support/harness.py` and entrypoint: fresh UUID project, ephemeral loopback ports, health waits, deterministic seeds and hashes, generated CA/server/client/untrusted/wrong-host certificates, required-X509 MySQL account, connection JSON/env, Rust automatic integration discovery/counts/logs and ownership-checked cleanup on success/failure/interruption. No global teardown command exists.
- `tests/integration.rs`: independently asserts exact source values and PostgreSQL sentinel over verified TLS; ignored only in ordinary unit runs, mandatory in requested database lane.
- `tests/fixtures/`: independently authored synthetic core; classified immutable upstream input snapshots and original notices; baseline evidence preserved with artifact hashes.
- `tests/support/baseline.py`: exact audited upstream pin, isolated owned DB run, loader image identity, logs/load hashes and independent SQL result checks.

Image pins: MySQL `sha256:6ea90827b1100f8f2ae306a539f86d2c264a26ed435a2a9f75551dd5c3aeb242`; PG16 `sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea`. Source seed SHA-256 `efbe952f0397182d11caf187cc806b741abd642799a6d7ad8e554c3bf49f7c56`; target sentinel seed `af1413b85316897eb0a81f249713131fddc929f515d21840b44cb835b1c5e504`.

## Actual v3/v4 outcomes

Both images were built from audited pgloader `231ab86778ca5ffd7de40878714760c8b4860cdf`. Commands/build output and successful/failed attempts are saved in [baseline artifacts](../tests/fixtures/baselines/2026-10-01/).

- v3 builder: `docker build --target pgloader-v3-builder --build-arg DYNSIZE=4096 -f ../pgloader/clojure/tests/Dockerfile -t my2pg-reference-v3:231ab867 ../pgloader`; executable reports `3.6.10~devel`, SBCL2.2.9.debian. Image ID `sha256:70dd9fef34e3263377be2ae8f527339deb0fab02c3d87b1523654ac91e7584c2`.
- v4: correct context is `../pgloader/clojure`, not the repository root; initial wrong-context attempt is preserved. Image ID `sha256:c0d9e27a869deb46df9f94c2487dfb01e60a6fbf41785ef844d13655cd4375e9`.
- v3 first rejected PG `sslmode=verify-ca`/sslrootcert parser syntax, then failed certificate trust using accepted `sslmode=require`. A further retry supplied the test CA through `SSL_CERT_FILE`; migration exited0, six independent assertions passed: count3, exact high-precision decimal, exact BLOB, NULL vs empty, and public sentinel. **Earlier interim suspicion of a v3/MySQL8.4 authentication failure was disproven.** Default caching_sha2_password stayed enabled; the accepted load requests MySQL TLS through `useSSL=true`.
- v4 migrated3rows, exited0 and passed5of6checks, but changed BLOB `00015c09ff` into `5830303031356330396666` (ASCII `X00015c09ff`). It is not a correct baseline for this workload. The Rust source fixture smoke preserves the expected bytes; transport/COPY acceptance belongs to T07/T10.

## Decisions, limitations and handoff

- Native arm64 MySQL8.4/PG16 is pinned and exercised. Other version/architecture lanes remain explicit T22 work and fail when requested without reviewed pins. No full matrix/performance/RSS certification is claimed.
- Upstream compatibility case dispositions are complete; their `implementation_status` remains pending until T10/T16/T17 workers link actual Rust acceptance evidence. This task does not mark all65featurecasespassed merely because classification exists.
- The initial TIME edge `-838:59:59.999999` failed strict MySQL range validation; corrected to valid `-838:59:58.999999` and actual value asserted. Full maximum seconds/fraction boundaries belong to T10.
- Baseline build inputs record resolved image/build output but apt/Quicklisp/Maven dependency resolution is not a reproducible release benchmark toolchain; T21 must pin/reproduce comparable measurement inputs.
- Shared live worker fixture: `target/integration/my2pg-mysql84-pg16-dad0a38e415d/`; C received env/JSON. It stays live deliberately for the driver work and must be stopped with `tests/run-integration.sh --stop <connections.json>` when its lease ends. Separate completed/failed test runs are already stopped.
- Tested base revision: `6e9d7a4d99626055fa36a47ec4f004c65e10f25b` plus explicitly uncommitted implementation files; coordinator owns the integration commit and task-board updates. No shared model/root modules/dependency files were authored by G.
- Coordinator review requested for fixture dispositions, stored baseline results and harness lifecycle. T01 contracts now available; T02 is ready for handoff. The discovered cancellation defect was fixed by C and the final fresh all-target lane passes9cases; broader milestone/release acceptance still belongs to the coordinator.

## T02 acceptance handoff

- [x] All selected upstream fixtures classified with original pin/hash/license and new case IDs; immutable snapshots available locally.
- [x] Deterministic pinned MySQL8.4/PG16 harness runs actual seed/Rust TLS checks, publishes metadata and cleans only owned resources.
- [x] Explicit database request without required env or discovered test cases fails; resource-security regressions checked.
- [x] v3/v4 real build/run outcomes recorded with independent content checks and actual correctness gaps.
- [x] Full database discovery surfaced dependent suite errors instead of silently skipping or counting them passed.
- [ ] Coordinator review/integration revision and final M0 decision (owned by I).
