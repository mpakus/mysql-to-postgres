# 2026-10-02 — T22 — MySQL 8.4/PostgreSQL 16 reviewed-source refresh

## Result

The final refreshed MySQL 8.4.11/PostgreSQL 16.15 Darwin/ARM64 required lane
passed after T13 cancellation review and nullable processlist-oracle hardening.
The serial native suite reported 156 passing cases, zero failed IDs, and all 26
applicable required IDs exactly once. The fixture harness exited successfully
and stopped its owned database project.

## Prior lane failure and correction

The first latest-source rerun reported 154 passing cases and two failures.
`t13_external_mdl` attempted to decode nullable `STATE` as a non-null Rust
`String` after its killed backend disappeared. The panic interrupted fixture
cleanup and left its event trigger installed; the later T16 existing-target
case correctly rejected that trigger. The oracle now reads `STATE` and `INFO`
as optional values and disappearance polling selects only the connection ID,
so a disappearing row cannot panic during conversion. The full rerun passed
both cases and the entire lane.

## Reproduction evidence

- Command: `rtk run 'sh tests/run-integration.sh mysql84 pg16'`
- Git HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the working tree was
  dirty with the implementation and test changes described in the T13
  worklogs.
- Owned project: `my2pg-mysql84-pg16-b14e07c9444a`; MySQL 8.4.11 and PostgreSQL
  16.15. The harness connection metadata records cleanup after the run.
- Result JSON:
  `target/integration/my2pg-mysql84-pg16-b14e07c9444a/rust-tests.json`
  SHA-256 `ecffd0496700ea7c2b6a550cd799bcf45ef5f0fdfde39a731bfaa449b9c6683a`.
- Test log SHA-256:
  `efc9b95ae4d2c171a4dde1a5827a12ee4ea03c716c437bf11812c2b9132badf9`.
- Production worker binary SHA-256:
  `42a33d84192cf21c0a3702694308094ad23021423424f710030c5ce967ea3b81`; it
  matches the current `target/debug/my2pg` binary after the lane.
- Cargo lock SHA-256:
  `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`.
- Harness SHA-256:
  `84d999b665608325e29e1579bbbb00a877b2d6f5f8a97685e413430448e2a68c`.

This refresh replaces the earlier 156/26 T13 lane claim at
`my2pg-mysql84-pg16-dfcf1d05dbbf`, which predated reviewer follow-ups. It
accepts only this MySQL/PostgreSQL version cell; the remaining matrix, T12
physical-sync faults, T21 benchmark equivalence, NLS/fuzz/soak/CI, and release
gates remain open.
