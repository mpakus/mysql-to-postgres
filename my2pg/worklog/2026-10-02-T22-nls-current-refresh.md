# T22 — current-source PostgreSQL NLS refresh

Status: localized-error acceptance passed on the current dirty source snapshot; broader T22 gates remain open.

## Evidence

- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` with uncommitted implementation changes present.
- No code or fixture changed; this was a runtime-only refresh.
- Native pinned fixture: MySQL 8.4.11 and PostgreSQL 16.15 (Debian, NLS-enabled), both `linux/arm64` on Darwin/ARM64.
- Project: `my2pg-mysql84-pg16-nls-7ea276e1b514`; PostgreSQL image pin: `sha256:1c2f3efc9c5ab63fe557565c9443dbcfc0cecd2b28a3989244dfc80eb6cb96f9`.
- Generated `fr_FR.UTF-8` inside the owned PostgreSQL container, restarted only that service, and ran `nls::t12_localized::actual_french_builtin_check_error_keeps_sqlstate_recovery_and_durable_reject` against its new loopback port.
- Command: `rtk proxy sh -c '. target/integration/my2pg-mysql84-pg16-nls-7ea276e1b514/env.sh; export MY2PG_POSTGRES_URL=postgresql://my2pg:integration-only@127.0.0.1:50841/target; export MY2PG_NLS_LOCALE=fr_FR.UTF-8; bin/cargo test --offline --locked --features nls-integration --test nls -- --ignored --nocapture'`.
- Result: exit 0; 1 passed, 0 failed, 0 ignored in 1.21 seconds. The real built-in CHECK error returned SQLSTATE `23514`, severity `ERREUR`, and French text `la nouvelle ligne de la relation « rows » viole la contrainte de vérification « rows_n_check »`; production recovery and durable reject assertions passed.
- Retained artifact: `target/integration/my2pg-mysql84-pg16-nls-7ea276e1b514/t12-nls/run-18dae5cfecf00f60-1510a-0/localized-error.json`, SHA-256 `62dfca386b9e0cd14cb4e69704ecd32377f5f4de0cd3f8fdf9fac7e1def8a38f`.
- Cleanup: `rtk run ./tests/run-integration.sh --stop target/integration/my2pg-mysql84-pg16-nls-7ea276e1b514/connections.json`; metadata records `stopped`. Docker inspection showed only pre-existing Strangler services.

This refresh closes only the current-source NLS case on this native arm64 fixture. It does not establish MySQL 5.7 support, other operating systems/architectures, hosted CI, broader fault/soak coverage, comparative performance, packaging, or release acceptance.
