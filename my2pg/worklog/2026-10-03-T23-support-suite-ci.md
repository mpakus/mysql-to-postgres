# T23 — run packaging, source-scope, and container regressions in CI

- Status: workflow change and local validation complete; hosted execution remains unavailable from this checkout
- Owner: coordinator
- Task: [T23](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release)
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree is dirty

## Reference research before implementation

| Query and prefix | Pinned revision and evidence | Finding and adaptation |
| --- | --- | --- |
| `offline CI test discovery package boundary source scope installed binary support suite`; `project-my2pg` | Project index `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; `worklog/2026-10-02-T23-source-exclusion-regression.md` plus current `.github/workflows/ci.yml:18-24`. | Existing workflow calls fixture, harness, matrix, and T21 tests individually, but omits `test_package.py`, `test_source_scope.py`, and `test_container_smoke.py`. Add standard-library unittest discovery for every `tests/support/test_*.py` so archive boundaries, installed-binary MySQL-only source scope, and container cleanup/mount regressions run in the offline job. Keep the fixture manifest command explicit. |
| `test discovery test suite GitHub workflow test runner`; `project-pgloader` | pgloader pin `231ab86778ca5ffd7de40878714760c8b4860cdf`; `clojure/tests/Makefile` has explicit server-version suites, including MySQL 5.7. | The original's DB-backed Lisp/Clojure targets do not fit this database-free release regression suite; do not adopt its runtimes or version-suite setup. |
| `cargo test ci github actions tests workspace`; `ref-paganel` | Paganel pin `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`; search returned its local multi-database Compose fixture, not a CI test-discovery pattern. | No reusable discovery pattern or code is needed; use Python's standard unittest discovery. |

Read `.github/workflows/ci.yml`, `tests/support/test_package.py`, `test_source_scope.py`, `test_container_smoke.py`, `test_harness.py`, and `test_matrix.py`. The full support discovery command passed locally before the workflow edit: **35 tests passed**. `test_source_scope` installs the release binary offline in a temporary root, checks the normal dependency graph, and rejects alternate source markers/help; `test_package` checks required/excluded archive paths and packaged links; container smoke tests mock Docker. RTK and Ponytail guidance were applied.

Before finalizing step order, XERJ query `test_source_scope Cargo tree offline target all cargo fetch CI order` against `project-my2pg` returned the existing T23 package/source-scope evidence and T22 CI worklog. Direct source at `tests/support/test_source_scope.py:63-75` confirms `cargo tree --target all --offline`; the workflow currently runs `cargo fetch --locked` later at lines 34-35. Query `cargo fetch offline cargo tree target all CI` against pinned `ref-paganel` did not return an applicable cache-order rule. Keep the support discovery step after the workflow's all-target `cargo fetch` so a fresh runner has locked target metadata available before `cargo tree --offline`.

## Acceptance

- Replaced the hand-listed harness/matrix unit commands with `python3 -m unittest discover -s tests/support -p 'test_*.py' -v`; fixture provenance remains explicit and T21 performance tests retain their own named step.
- `python3 tests/support/fixture_manifest.py` passed, verifying 65 case IDs and the fixture/pin hashes. Full support discovery passed: **35 tests**, including archive boundaries, installed-binary source exclusion, harness isolation, matrix discovery, and mocked container lifecycle/mount checks.
- `python3 -m unittest tests/performance/test_runner.py -v` passed: **37 tests**. Ruby 2.6 parsed the workflow YAML and `rtk git diff --check` passed.
- Refreshed `project-my2pg` after this work and queried the resulting index for this worklog.
- Hosted Actions remains unobserved: this checkout has no Git remote and `gh auth status` reports its configured token is invalid. Local workflow validation is not hosted CI evidence.
