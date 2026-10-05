# T11 — deferred-key integration review

Coordinator owns integration registration and shared checks. The worker owns the narrow existing-policy predicate and native case. RTK/Ponytail and writing-for-agents guidance applied.

## Reference research before registration

At project HEAD `2b0573083761aa664c195514944459d1d595dbfc`, XERJ project query `t11_constraint_triggers` returned `tests/integration.rs:121` in the verified dirty snapshot. Read the entire original registration file and the worker's deferred-key worklog/case. Cases are explicitly registered as modules in the integration target. Chosen adaptation: use that existing two-line module registration for the new native regression; no new discovery mechanism.

The project query `deferrable recovery tail_unique` found the unchanged T12 CLI fixtures and existing-policy rejection at `tests/cases/t11_existing_policies.rs:661`. Read the originals. The tests require actual locked COMMIT after an acknowledged recovered prefix; exiting at preflight does not exercise that obligation. Query `ref-rust-postgres commit --symbol` at pinned revision `1084ca8f5b5302e161892f2fa40abf71b4060c10` returned `tokio-postgres/src/transaction.rs:1`; read commit/rollback at50–83. Its commit awaits the server response, and that does not authorize replay when the response is missing. Keep our existing ACK/uncertainty accounting.

Primary [pg_index](https://www.postgresql.org/docs/16/catalog-pg-index.html) and [pg_constraint](https://www.postgresql.org/docs/16/catalog-pg-constraint.html) facts support a positive ordered constraint/index link. Preserve a valid extra deferred UNIQUE while refusing to bind it as a required immediate source key. No replay-safety or transaction logic is changed by this correction. Full native rerun remains required; the earlier118-pass/two-failure run is retained.
