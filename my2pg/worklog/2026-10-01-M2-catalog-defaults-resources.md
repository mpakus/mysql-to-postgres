# M2 catalog, default and resource increment

Coordinator accepted these bounded increments after the saved core45cfca4. T11/T13/T15 remain in progress; no grouped M2 or release checkbox is closed by helper/query tests alone.

- T11: three read-only SQL queries retain typed column facts, ordered index parts and complete constraint headers. Native2-case proof covers duplicate index names in different schemas, quoted/comma names and reverse composite order, defaults/generation/identities/comments/array dimensions, INCLUDE/expression/partial/opclass/collation/null/deferral metadata, NOT VALID constraints and actual invalid concurrent-index state. Rust decoder integration, full sequence/dependency/trigger inventories and existing-policy acceptance remain open. See [query contract and evidence](2026-10-01-T11-observed-inspection.md).
- T13: the scheduler resource module has checked B/W/P calculations, table/index/target/byte permits and explicit cancellation. Its seven focused cases pass; offline and runtime now share the queue-aware minimum. Data modes reject insufficient memory before network/DDL; schema-only needs no COPY allocation. Parallel runner/index execution, actual full/empty queue fault tests, backend teardown and measured RSS are still required. See [scheduler](2026-10-01-T13-worker-scheduling.md) and [shared validation, including observed regression](2026-10-01-T13-shared-validation.md).
- T15: finite typed default comparison removes the unknown-expression equality bypass. A production18-default migration passes, all18 independent default mutations differ, native default inserts match, exact-float-output settings are checked and an unknown identical side-effect function stays unsupported without consuming its sequence. JSON/inet/unqualified enum and alternate rendering styles remain explicit completion gaps. See [default evidence](2026-10-01-T15-default-verifier-preparation.md).

## Integrated validation

Required RTK/Ponytail and before-code reference searches are recorded in the linked worklogs; pinned references remain read-only. Registered both actual native case modules and the scheduler resource module. No dependency or driver pin changed.

- `bin/cargo test --offline --all-targets`:96 ordinary cases passed; database cases stay ignored in this lane and were separately required below. Local deadline/socket tests ran through the approved local-network lane.
- `tests/run-integration.sh mysql84 pg16`: all76 explicitly invoked fresh cases passed (11 driver/65 integration) against owned `my2pg-mysql84-pg16-424e53627418`; fixture stopped automatically. The first pair a44778dcf791 retained75 passes/one failure from an old schema-only memory expectation; the preserved failure and strengthened test are recorded in shared validation. Do not count that first run as acceptance.
- Full rustfmt, strict all-target/all-feature Clippy and authored whitespace checks passed after source freeze. These are the native development pair, not the full server/platform matrix or speed/RSS acceptance.

Private ignored artifacts: `target/integration/my2pg-mysql84-pg16-424e53627418/{rust-tests.json,rust-tests.log,seed-checks.json,connections.json}`. Evidence records parent85b238b with dirty worktree; the following source checkpoint preserves the exact tested changes. Cargo.lock SHA-256 remains `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`.

Source checkpoint is `7aeae7a1c474b04fdf67c298d730327bea489c8f`, preserving the exact tested source. Native versions were Oracle MySQL8.4.11/PostgreSQL16.15;14 seed assertions passed. Final native log SHA-256 is `1d665ac4548893012bec42889a150265ab6b87cca4cc6fa9dc6ba51e90ba1399`.

Coordinator refreshed XERJ after source freeze:183 project files; all seven corpus counts/pins and every eligible Rust source were checked, with17 expected-source queries passing and a green cluster. Final dynamic counts and query paths live in `.reference-coding/reports/verification.json` after the documentation refresh. No reference pin advanced. T11/T12/T13/T15 and the full matrix/performance/release gates remain open.
