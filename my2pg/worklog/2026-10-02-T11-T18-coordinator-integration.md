# 2026-10-02 — T11/T18 coordinator integration

Status: focused coverage integrated; T11, T18, M2, and M3 remain open. The coordinator registered `t11_identity_types` and `t18_content` in `tests/integration.rs` after confirming the workers' file leases and exact module paths.

The T11 identity worker ran `t11_identity_types::` with `--ignored` on an owned MySQL 8.4/PostgreSQL 16 TLS pair: **2/2 passed**. This covers nine signed/unsigned integer mappings and refuses unsigned BIGINT AUTO_INCREMENT before target schema creation. The pair was stopped. Detailed query, source values, and independent assertions are in [the T11 worklog](2026-10-02-T11-identity-type-coverage.md). The separate stale-sequence data-only append regression also passed **1/1** in the [reset worklog](2026-10-02-T11-data-only-sequence-reset.md).

The content primitive is covered by two registered integration tests, both passing in the coordinator run. Its external sorter is independently covered by six library tests. It remains disconnected from MySQL/PostgreSQL canonicalization, migration snapshot reuse, append-baseline behavior, and final report wiring; content verification remains unsupported.

The default-parallel integration command first produced three loopback permission failures in auth tests and two timing/resource failures in artifact-worker tests. It is not accepted as evidence. Rerunning with the repository's serial native protocol and approved local-network access, `bin/cargo test --offline --locked --all-features --test integration -- --test-threads=1`, passed **12 tests, 0 failed, 116 fixture-dependent tests ignored**. The separate all-feature library suite passed **99, 0 failed, 3 ignored**. Strict all-target/all-feature Clippy, format check, and `git diff --check` passed after integration.

These runs confirm the registered and owned cases only. They do not close the broader T11 structure/existing-policy matrix, T18 end-to-end comparison, the other grouped M2/M3 checklist items, or the server/platform release matrix.
