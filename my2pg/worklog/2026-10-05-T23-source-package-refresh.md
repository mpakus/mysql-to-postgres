# 2026-10-05 — T23 — current source-package repeatability refresh

- Status: two current-tree source archives match; distributed-artifact gates remain open
- Agent/role: coordinator / integrator
- Task and specification links: [T23 task board](../docs/agent-tasks.md#task-board), [release checklist](../docs/checklists.md#release), [package-boundary tests](../worklog/2026-10-02-T23-source-archive-boundary.md)
- Workspace: `/Users/renatibragimov/www/pg/my2pg`
- Source revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; working tree dirty
- Environment: Darwin/ARM64; Cargo 1.99.0; locked dependencies, offline build

## Intended result

Rebuild the source package twice after the current implementation and docs updates, confirming deterministic archive bytes and an intact package boundary.

## Verification

Ran `rtk run './bin/cargo package --allow-dirty --locked --offline'` twice in sequence. Both invocations succeeded, including Cargo's default package verification, and produced `target/package/my2pg-0.1.0.crate` at 560,204 bytes with SHA-256 `6dc50668af7f1553d8ecddfb3a684463495c042a14c5b40aac6d894df6bab7fd`.

Inspection of the resulting tar archive found 223 regular files. The package contains `README.md`, the PostgreSQL license, runtime source/tests and required fixtures, `docs/scope-and-compatibility.md`, and the generated third-party notices; internal worklogs and benchmark outputs remain outside the archive. The full 35-test support suite, including package-boundary checks, passed earlier on this same source snapshot after the documentation changes.

The two sequential build logs and per-run checksums are retained in `target/package/repro-check-2026-10-05/`. Its `manifest.json` records both identical archive hashes and log hashes; manifest SHA-256 is `20edccc3caae0e5f022758b203ac82fa9c794d52b84bef03696146f8518fa2b5`.

## Limitations and handoff

This proves local deterministic source-package generation only. It does not supply native Linux/AMD64 or published binaries, distributed release checksums, hosted build results, or qualified human dependency/license and fixture-attribution approval. Keep T23 and the release gate open.

## Review

- Reviewer: `/root/t24_independent_review`.
- Findings and resolutions: reviewer confirmed the final archive hash, size, 223-file count, allowlist/provenance content, and absence of internal worklogs, target outputs, and benchmark results. A follow-up confirmed both retained build-log hashes match the manifest, both logs record successful verification/compilation and matching 223-file/547.1 KiB package output, and the manifest digest matches the retained crate. Cargo emits a non-fatal warning that documentation, homepage, and repository metadata are absent; adding external URLs was outside this task and none are configured in the checkout.

## Current-source repeatability follow-up — 2026-10-05

The source changed after the earlier 223-file result above. To test the current package independently of Cargo's shared build output, ran `CARGO_TARGET_DIR=target/t23-repeat-a ./bin/cargo package --allow-dirty --locked --offline` and the same command with `target/t23-repeat-b`. Both Cargo package verification builds passed. The extracted archives are each 566,486 bytes, contain 224 files, and have identical SHA-256 `e987551e0ce551e4442210ec9fab3bbbe208e1218e367ab4e8f5eb9fb47db664`; `cmp` confirmed byte equality.

`rtk run 'python3 -m unittest tests.support.test_package -v'` passed all three package-boundary tests against Cargo's package selection and working-tree links. Separate tar inspection of both built archives confirmed runtime sources, tests, docs/examples, generated dependency notices, and fixture attribution material are included; internal worklogs and generated run results are excluded. `rtk git diff --check` passed. The workspace was dirty at source revision `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`; this is local Darwin/ARM64 evidence only. It closes repeatable local source packaging, not native Linux, hosted CI, distribution checksums, or human legal/attribution review.
