# Implementation plan

## Milestones

M0 and the M1 development-pair alpha are accepted at the tested implementation snapshot `7555e32a661f459db95fd1ef8ca7d03c5e601faf`, followed by the independently checked source-PK identity correction. [T01](../worklog/2026-10-01-T01-foundation.md#coordinator-integrated-acceptance) and [T09](../worklog/2026-10-01-T09-cli-alpha.md#acceptance-still-required) record 59 offline tests and 34 actual native MySQL8.4/PostgreSQL16 tests plus fixture cleanup. M2–M4 remain open, including the full server/platform matrix and measured performance/release gates. Task ownership and exact dependencies are in [agent-tasks.md](agent-tasks.md); completion evidence belongs in [worklog/](../worklog/README.md).

| Milestone | Tasks | Demonstrable result | Exit gate |
| --- | --- | --- | --- |
| M0 — Baseline and contracts | T01–T03 | Reproducible reference, isolated fixture harness, tested Rust driver path, shared data contracts | Toolchain/dependencies pinned; streaming/auth/type spike passes; baseline limitations recorded |
| M1 — Working migration | T04–T09 | TOML → plan → create → stream COPY → finalize a small real MySQL fixture | End-to-end values, PK/identity basics, fresh-schema behavior, plain CLI, and failure exit tested |
| M2 — Reliable core | T10–T15 | Core type/schema fidelity, modes, row recovery, table concurrency, secure connections, useful console/report | Core edge cases and fault tests pass; no required failure is reported as success |
| M3 — MySQL compatibility | T16–T19 | Strict `.load` importer, advanced MySQL structures, full verification, view snapshots and hooks | Every selected MySQL fixture classified; supported cases pass and excluded semantics fail explicitly |
| M4 — Performance and release | T20–T24 | Bounded range readers, measured speed/RSS, complete test matrix, clean distributable application | [Release checklist](checklists.md#release) has linked evidence and independent review |

Do not close M1 with stubs that always return success. Do not start throughput tuning before correctness tests can detect dropped, duplicated, or altered rows. A phase finishes only when the tested executable demonstrates the behavior claimed by that phase.

## Sequence

1. Establish a standalone my2pg repository/build root when implementation begins; preserve this planning history and the upstream revision/provenance. The workspace parent is currently not a Git repository. Keep the sibling pgloader checkout unchanged as a reference.
2. Run M0 transport experiments before locking driver-dependent types. Freeze the small shared interfaces and synthetic fixtures, then merge independently owned modules in short increments.
3. Deliver a thin vertical migration in M1. The initial fixture contains NULLs, text, a numeric PK, an identity, and a FK relationship. It provides a real smoke test while deeper mapping and parallelism are added.
4. Add the difficult semantics in M2: exact values/defaults, identity state, schema isolation, row recovery and status, shutdown, memory limits, and verified connections.
5. Expand to M3's tested MySQL subset. Use the fixture manifest to drive implementation rather than copying the old module tree. Keep explicit rejection paths for semantic features without an equivalent.
6. Measure M4 performance with the fixed workload corpus. Add range reads only under a valid source consistency policy, and keep a single-reader fallback.
7. Finish packaging, operator documentation, license/provenance checks, release matrix, and an independent acceptance review. Present remaining gaps as gaps; do not turn them into completed checklist items.

## Cleanup plan

The new application starts clean, so cleanup primarily means **not importing unrelated machinery**. No deletion from `../pgloader/` is needed to achieve a MySQL-only binary.

| Area | Keep/reimplement | Leave out of my2pg |
| --- | --- | --- |
| Source transport | MySQL catalog and row streaming | All other source drivers and the generic source registry |
| Target | PostgreSQL DDL, COPY, verification | Citus/Redshift distribution and S3 COPY paths |
| Config | Versioned TOML and a strict MySQL importer | Full multi-source DSL, INI migration, mustache contexts, arbitrary code loading |
| Runtime/build | Cargo package, tested Rust toolchain, release binaries | SBCL/Quicklisp, ASDF, JVM/JDBC, GraalVM, old bundling scripts |
| Tests | Classified MySQL fixtures and database assertions | Non-MySQL suites, duplicate baselines, a JAR required to run Rust tests |
| Benchmarks | Actual MySQL workloads and exact correctness checks | SQLite/CSV/MariaDB results presented as MySQL performance |
| Documentation | Supported options, tested recipes, known limits | Stale paths, unused flags, executable-looking commands without an implementation |

At T23, inspect the compiled feature/dependency graph, help output, examples, parser tests, container, and release contents. Every forbidden source must be absent as functionality and rejected as input. A negative-test string mentioning SQLite is not a violation; an SQLite source driver in the binary is.

Retain project/fixture notices for adapted code or data. Track external datasets separately by source, license, version, and digest. Review the reference license and dependency licenses before distribution; do not assume a Rust rewrite erases provenance requirements.

## Risks and resolution tasks

| Risk | Evidence or reason | Resolution |
| --- | --- | --- |
| Driver normalizes zero dates, decimal, or binary before our policy sees them | New transport differs from JDBC/qmynd | T03 proves raw fidelity before the driver is frozen |
| Rust is not faster because PostgreSQL/indexing dominates | Language choice does not change destination bottlenecks | T02 baseline, T21 phased profiling and comparable benchmarks |
| Parallel reads observe different source states | Separate MySQL transactions are not a shared snapshot | T05/T13 enforce consistency policy; T20 requires frozen source |
| Required objects disappear behind warning-only behavior | Audited post-DDL handling and parser/implementation drift | T11/T15 explicit object outcomes and nonzero failure status |
| Lost commit acknowledgement causes duplicate replay | Batch commit is a distributed outcome | T12 indeterminate state, no automatic replay; T22 fault injection |
| Source semantics lack a direct target equivalent | Collations, prefixes, generated SQL, routines/triggers | T17 explicit subset/overrides/omissions, no guessed SQL translation |
| Identifier transforms break source reads or FKs | Multiple naming stages in the old catalog flow | T06/T11 stable source IDs and one mapping used everywhere |
| Memory bound excludes driver packets or multiplies with workers | Queue bounds alone do not bound total process memory | T03/T07/T13 packet, row, and global buffer limits; T21 RSS soak |
| Baseline tests are incomplete or unavailable | Missing mysql57 suite and selected namespace lists | T02 manifest; T22 real discovery and independent version matrix |
| Agents diverge on shared structures/config | Several modules consume the same plan/event contracts | T01 interface fixtures; integrator owns shared files and contract changes |

## Decisions to confirm through implementation evidence

Proceed using the choices in this plan. These are focused experiments, not reasons to pause all work:

- T03 selects exact crate/toolchain/TLS versions after raw-value and auth tests.
- T02/T21 establish whether the proposed speed/RSS thresholds are achievable on the chosen hardware.
- T17 enumerates the supported SQL expression grammar and behavior of ON UPDATE emulation. Any remaining case gets a tested explicit override/rejection path.
- T20 sets range sizing after sparse/skewed-key benchmarks; avoid copying the misleading legacy `rows per range` semantics.
- T23 confirms release platforms against actual build/test evidence; untested platforms are not advertised.

Changing a supported behavior requires a small decision entry in the worklog, the matching specification/test update, and integrator review. Do not expand to another source database as a convenience while solving a MySQL case.

## Migration operation boundary

The application copies data and structures and reports verification. It does not switch application connections, coordinate business writers, retire MySQL, or perform a production cutover. An operational rehearsal still needs the correct environment, source consistency, successful reconciliation, application compatibility tests, and recovery evidence. These are separate from saying the Rust executable passed its test suite.
