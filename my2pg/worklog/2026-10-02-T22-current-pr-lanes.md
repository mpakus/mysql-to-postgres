# T22 — current local pull-request lane rerun

Status: both configured PR database lanes passed on the current ARM64 host; hosted GitHub Actions execution, MySQL 5.7 lanes, NLS, broader platform coverage, and remaining fault/soak gates are still open.

## Research and source evidence

- Project revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; the working tree was dirty. Cargo reported the same lockfile SHA-256 for both lanes: `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`.
- XERJ query: `project-my2pg "native integration required case release lane" -k 6 --full 80`; index revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Relevant hits were the current PR-lane setup and prior required-case refresh worklogs.
- Read `tests/support/harness.py` (including `required_cases`, `start`, and the serial ignored-test invocation), `tests/run-integration.sh`, `tests/compose/compose.yml`, `tests/compose/images.json`, `.github/workflows/ci.yml`, and the T22 checklist/task board. The harness rejects non-native image platforms, allocates uniquely named Compose projects, validates image versions, records test/build hashes, requires each configured case exactly once, and stops only its owned project.
- Adaptation: run both declared PR matrix cells sequentially with the harness's exact native-required command; record raw artifacts and leave the wider matrix and hosted-workflow gates open.

## Results

| Lane | Actual server versions | Platform and pinned image suffixes | Result |
| --- | --- | --- | --- |
| MySQL 8.4 → PostgreSQL 16 | MySQL 8.4.11; PostgreSQL 16.15 | Linux ARM64; mysql `6ea90827b110`, pg `721873c34ceb` | 156 cases passed; 26/26 required IDs passed exactly once; exit 0; no failed or unmet IDs. |
| MySQL 8.0 → PostgreSQL 18 | MySQL 8.0.46; PostgreSQL 18.6 | Linux ARM64; mysql `213bbfaf6996`, pg `89f747171c4b` | 156 cases passed; 25/25 required IDs passed exactly once; exit 0; no failed or unmet IDs. |

The harness ran `bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --tests --no-run --message-format=json` before each lane, selected the normal `my2pg` executable, then ran all ignored native cases serially. Both lanes used executable SHA-256 `c3e45d02ce6dabd0eb390fbb525991d3966723adf72ce403a0f7882d08a34b9a` and stopped their uniquely owned database projects after completion.

Raw evidence is retained under:

- `target/integration/my2pg-mysql84-pg16-56c2858fdaf2/` — test log SHA-256 `f91efa51582c6eef98d44839e653c611efa0f30887e53b5e9d635c2169fddee6`.
- `target/integration/my2pg-mysql80-pg18-aea66bcbb02e/` — test log SHA-256 `33ff0454ff0a716a812087cd55594a59a4e9715dd6bb4c260390f2265132c330`.

## Validation and limits

- `rtk run ./tests/run-integration.sh mysql84 pg16` — passed; the harness recorded all 26 required IDs.
- `rtk run ./tests/run-integration.sh mysql80 pg18` — passed; the harness recorded all 25 required IDs.
- Both runs were on a dirty worktree at the recorded Git revision. These prove current local ARM64 PR-lane behavior only; they do not prove hosted Actions ran, all six database matrix cells ran at this exact dirty snapshot, Linux x86_64/macOS distribution, NLS behavior, physical `fsync(EIO)`, benchmark performance, fuzz, or soak acceptance.
- No implementation code changed during these lane runs.
