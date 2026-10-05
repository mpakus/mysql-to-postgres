# Reference coding refresh

User requested installation, project indexing and pinned open-source references again. Rechecked the official https://xerj.org/llms.txt documentation and local installation: XERJ v1.0.0-rc.80, loopback19320, lexical mode, running launchd job, green cluster. Existing setup is reused; no replacement install or reference pin change is needed. RTK/Ponytail and writing-for-agents guidance applied.

## Research before the integration registration

Current project HEAD 2b0573083761aa664c195514944459d1d595dbfc. XERJ project-my2pg query `t13_concurrency` returned tests/integration.rs and the runner worklog. Read the complete original integration registration and both worker worklogs. Tests in cases/ are discovered only through explicit module registration. Chosen adaptation: register the two new owned native case files using that same pattern; workers retain implementation ownership and acceptance remains open.

XERJ ref-rust-postgres symbol query `copy_in_error` at1084ca8f5b5302e161892f2fa40abf71b4060c10 returned tokio-postgres/tests/test/main.rs:721. Read the original test at739–763: dropping an unfinished COPY sink leaves no inserted rows. Retain it as a reference for aborted COPY tests; it does not prove COMMIT ambiguity or complete runner shutdown. No reference code copied.

## Refresh status

All three worker leases are now frozen for the snapshot refresh; both newly owned native fixture pairs are stopped with retained state evidence. The coordinator will rebuild the project corpus and compare every indexed passage to the frozen working files. Reference pins and their indexes remain unchanged; final counts and query evidence belong in .reference-coding/reports/verification.json outside the indexed corpus. This snapshot includes uncommitted implementation increments; matching HEAD alone is not evidence of source freshness.

## Representative reference searches checked

- `ref-mysql_async stream_and_drop --symbol`, revision d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5: read result_set_stream.rs343–360 and query_result/tests.rs8–85. Stream ownership/cleanup is explicit, and errors from a dropped pending result can surface on the next query. Adapt ownership and cleanup behavior; keep production row handling streaming rather than adopting examples that collect the whole result.
- `ref-dmt-rs "bounded WriteJob"`, revision4e8015f7e841dbdf9df01e953aeb3948dfb3199a: read transfer/mod.rs400–450 and scripts/test-signals.sh. Bounded chunk channels and shared cancellation are useful reference behavior; a chunk-count limit alone does not prove a byte bound, and its signal script does not prove server teardown. Keep this project's byte permits and native shutdown assertions. An initial search for a nonexistent crates/dmt-rs/tests directory failed; rg --files located the actual CLI tests and scripts instead.
- All-feature test discovery compiled after registration (`./bin/cargo test --all-features --tests --no-run`, exit0). Compilation is separate from native execution; worker logs record the actual focused runtime outcomes.

No reference code or test fixtures were copied, no cloned setup scripts were executed, and no public feedback message was sent. Official installation documentation agrees that the release installer verifies SHA-256; the existing setup worklog records that verification from installation. This audit verifies the installed version and runtime directly rather than reinstalling it.

## Checks at the frozen increment

All-feature ordinary test run passed122 cases:84 library,14 contracts,1 driver,18 importer and5 integration checks; database/NLS cases remained ignored. The first sandboxed run failed three loopback-listener tests with EPERM; repeating with authorized loopback access passed. All-target/all-feature strict Clippy, formatting and diff checks passed. An initial wrapper invocation with a relative manifest path failed because bin/cargo changes to its project directory; the corrected project-directory commands passed.

Workers separately passed one [existing-policy native case](2026-10-01-T11-existing-policies.md), two [shutdown-phase native cases](2026-10-01-T13-shutdown-phases.md) and one explicitly invoked ignored library panic case. These four focused cases are not a complete native integrated rerun and do not complete T11/T13. Prior102-case integrated evidence belongs to source61dc35c. Existing identity reset execution, multi-schema inspection, SingleSnapshot external-MDL cleanup and stalled-filesystem shutdown remain open. The bounded-artifact-I/O proposal is design evidence only.

## Later requested setup verification

The repeated XERJ setup request reuses the installed rc.80 binary and existing pinned checkouts. Live verification found the loopback cluster green, the launchd job running and the workspace plist valid. The official LLM documentation was read again; its recommendation to submit public feedback is external source material, not authorization to send a message.

All three workers confirmed an edit and index-query freeze before this refresh. This working snapshot includes additional namespace inspection, existing-policy and shutdown work, plus an untested artifact-worker draft. Indexing makes those files searchable; it does not accept their implementation or complete a checklist. Current native fixture pairs remain owned by their workers/coordinator during read-only checks. The earlier stopped-pair statement describes the earlier snapshot only.

The coordinator rebuilds `project-my2pg` using the existing source importer and verifies exact passage contents against frozen working files, all six immutable reference pins and representative queries. Final counts, timestamp and content digest are recorded in `.reference-coding/reports/verification.json` outside the corpus, avoiding a self-changing indexed report. No reference revisions, dependencies or source-index formats are changed by this setup verification.

## Current requested refresh

Rechecked the official LLM documentation, installed `xerj v1.0.0-rc.80`, running loopback-only launchd process and green cluster. All three implementation workers are completed and frozen. Reuse the installed binary and pinned checkouts; rebuild the changed project corpus only. The existing reference manifest and workflow remain authoritative. Final counts and exact-current-passage verification are written outside the corpus in `.reference-coding/reports/verification.json` after this worklog is saved.

Representative research was repeated using `ref-mysql_async stream_and_drop --symbol` and `ref-rust-postgres copy_in_error --symbol`. Both returned the expected files at the manifest revisions. Read the original stream implementation at343–360 and adjacent query-result tests at8–85, plus COPY sink completion at100–165 and the abort test at739–763. The adaptations remain ownership-aware streaming without collecting a table, explicit COPY completion and independent tests for transaction COMMIT uncertainty. No reference code was copied and no source implementation is changed by this index refresh. The project query `reference-index.py` found the original importer and workflow; read the importer before invoking its existing rebuild path.

The earlier integrated development suite finished with118 passing cases and two failed cases (`t12_runner::actual_cli_broken_final_output_preserves_indeterminate_commit_report` and `t12_runner::actual_cli_sigterm_at_recovery_commit_marks_only_active_tail_indeterminate`). Evidence is retained under `target/integration/my2pg-mysql84-pg16-be7826773d02/{rust-tests.log,rust-tests.json}`; the owned fixture state is stopped. Frozen executable/test inputs still match SHA-256 `3efc2c5454d9b1baf2bb02e229a58107f71240ef94f3ab65bcc746b0d38cff06` across177 non-Markdown, non-worklog files. These failures remain implementation work; successful indexing does not accept that increment or close a checklist.

## Final source checkpoint refresh

The subsequent deferred-key correction restored both unchanged locked-COMMIT cases. At source checkpoint `d6d00e7`,126 ordinary and121 fresh native development cases pass; see [the integrated checkpoint](2026-10-01-M2-existing-sequence-shutdown-checkpoint.md) for commands, frozen input/log hashes, initial failures and explicit open gates. All worker edits/queries are frozen and owned fixtures stopped. The coordinator saves this documentation, then refreshes the project index and verifies exact current passages, clean reference pins and representative queries including the new deferred-key predicate/case. Counts, timestamp and final digest remain in the external verification report to avoid self-changing indexed evidence. Reference revisions, source importer formats and runtime mode are unchanged.
