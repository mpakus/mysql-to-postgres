# T21 — run benchmark-runner regressions in offline CI

- Status: CI registration and local validation complete; hosted CI remains unobserved
- Owner: coordinator
- Task: [T21](../docs/agent-tasks.md#task-board), [CI workflow](../.github/workflows/ci.yml)
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree is dirty

## Reference research before implementation

| Query and prefix | Pinned revision and result | Adaptation |
| --- | --- | --- |
| `.github/workflows/ci.yml offline job test_matrix.py test_harness.py Python performance unit suite`; `project-my2pg` | Project index `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` returned T22 CI/task evidence; direct current workflow inspection is authoritative for the present gap. | Current offline CI runs fixture, harness, and matrix Python tests but omits `tests/performance/test_runner.py`. Add its standard-library unittest command to the existing offline job so the local-binary path regression runs on every push and pull request. |
| `GitHub Actions continuous integration tests workflow Python Lisp`; `project-pgloader` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; XERJ returned package/CI publication documentation, not a reusable Python test-discovery pattern. | Do not adopt pgloader's legacy runtime or its release workflow. |
| `GitHub Actions cargo test workspace CI workflow`; `ref-paganel` | Paganel pin `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`; XERJ returned its database Compose fixture, not a relevant test-discovery pattern. | No reference code or dependency is needed for a standard-library test command. |

Read `.github/workflows/ci.yml`, `tests/performance/test_runner.py`, its `runner.py` import and test helpers, and the latest [local release-binary regression worklog](2026-10-03-T21-release-binary-path.md). The suite contains database-free tests; Docker/database calls are mocked, while its RSS test launches a short-lived local child. The workflow already invokes Python 3 for fixture and matrix tests. RTK and Ponytail guidance were applied. No reference implementation provides a useful pattern for this small workflow change.

## Acceptance

- Added `python3 -m unittest tests/performance/test_runner.py -v` as a named step in the offline CI job, alongside the existing fixture/harness/matrix Python checks.
- The same command passes locally: **37 tests passed**. Ruby 2.6 parsed `.github/workflows/ci.yml`; `rtk git diff --check` passed.
- The other offline Python commands also pass locally: fixture manifest validation (65 case IDs), harness tests (12), and matrix tests (15).
- Refreshed `project-my2pg` after this work and queried the resulting index for the CI/performance test evidence.
- This checkout has no Git remote, and `gh auth status` reports the configured GitHub token is invalid. The workflow is locally validated but cannot be dispatched or observed from this checkout. Native AMD64 and hosted job results remain separate T22/T23 evidence.
