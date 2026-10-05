# 2026-10-05 — T23 — current package after T21 adapter work

- Status: local source-package repeatability passes; later source edits require another refresh
- Agent/role: coordinator / integrator
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Environment: Darwin/ARM64; Cargo 1.99.0; `--locked --offline`

## Verification

Ran `CARGO_TARGET_DIR=target/t23-after-t21-a ./bin/cargo package --allow-dirty --locked --offline` and repeated with `target/t23-after-t21-b`. Both packaged and compiled successfully. The resulting 224-file archives are 554.6 KiB compressed and have matching SHA-256 `902351e482f1520046a12ed376b662c1f96130891c4454f22c760e49dd6ffbcd`.

The three package-boundary tests passed against Cargo's package file selection. Direct tar inspection confirmed that the package contains the current T21 runner, its tests, and the attributed pgloader phase/batch-control patch, and contains no `worklog/` or `target/` paths. The two isolated build products are retained under `target/t23-after-t21-a/` and `target/t23-after-t21-b/`.

This is local Darwin/ARM64 packaging evidence only. Human dependency/license and fixture-attribution review, hosted builds, native Linux, distributed artifact checksums, and publication remain open. Because the T21 batch-equivalence follow-up is under consideration, rebuild the package after any subsequent source or documentation changes before using this hash as current.
