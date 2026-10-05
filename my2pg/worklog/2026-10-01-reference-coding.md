# 2026-10-01 — XERJ installation and reference coding

- Status: done — installation, source indexing and reference workflow; no Rust migration implementation.
- Agent/role: coordinator.
- Workspace: `/Users/renatibragimov/www/pg`.
- Specification: user request to install XERJ, index project sources, clone/index close open-source references, and research existing solutions before coding.
- Project/upstream revisions: [repository manifest](../docs/reference-repositories.json). my2pg has no Git/Cargo project yet; pgloader and all five reference repositories retain their pinned revisions.

## Result

Updated the existing XERJ `v1.0.0-rc.74` installation to `v1.0.0-rc.80` through the inspected official installer. Its release archive SHA-256 verification passed. The installed executable is `/Users/renatibragimov/.local/bin/xerj`.

Created a local loopback search service on `127.0.0.1:19320`, with sibling ports `19321` and `19322`, under the workspace launchd definition `.reference-coding/xerj.plist`. The job is registered in the current GUI login session as `org.my2pg.xerj`; it is not placed in a login-startup directory. Server storage is outside indexed corpus folders. Embedding mode is lexical; source searches use text/BM25, with no external embedding/reranking endpoint configured.

Shallow-cloned Paganel, dmt-rs, mysql_async, rust-postgres and indicatif under `references/`. Reused the existing pgloader checkout. Recorded commits, reasons for selection and observed license declarations in the manifest. Reference files were inspected, not executed as setup scripts or linked as dependencies.

## Reference research

| Query / prefix | Original evidence inspected | Finding and adaptation |
| --- | --- | --- |
| `stream_and_drop` / `ref-mysql_async`, symbol | `src/queryable/query_result/result_set_stream.rs:343`; adjacent query-result tests | Owned result streams stop at one result-set boundary. Our reader needs streaming, preserved raw values and tested cleanup/cancellation; example collection into a whole vector is unsuitable. |
| `copy_in_error` / `ref-rust-postgres`, symbol | `tokio-postgres/tests/test/main.rs:739`; `tokio-postgres/src/copy_in.rs:128` | Explicit COPY finish is required; upstream's dropped-sink test expects zero inserted rows. Add transaction/commit-ambiguity cases in T03/T07/T12 rather than assuming sink completion proves commit. |
| `bounded WriteJob` / `ref-dmt-rs` | `crates/dmt-rs/src/transfer/mod.rs:414–418`; signal test script | Reader/writer queues are bounded by item count. Our T13 also needs global byte permits and blocked-channel cancellation tests. |
| `convert_type` / `ref-paganel`, symbol | `crates/engine-schema/src/converters/to_postgres.rs:25,65`; schema-test inventory | Typed conversion outcomes are useful, but unsigned integers are narrowed with warnings. Our contract requires full-range preservation or explicit rejection and independent maxima tests. |
| `is_hidden` / `ref-indicatif`, symbol | `src/draw_target.rs:136,753`; multi-progress lifecycle docs | Terminal detection suppresses drawing; the hidden multi-progress test covers propagation. T15 must additionally test the actual redirected CLI/plain/JSON behavior. |
| `retry-batch` / `project-pgloader`, symbol | `src/pg-copy/copy-retry-batch.lisp`; `clojure/src/pgloader/batch.clj`; COPY test inventory | Preserve useful bad-row subdivision cases while enforcing our stricter error classification and uncertain-commit policy. |

Raw successful searches are saved as `.reference-coding/reports/search-mysql-stream.json`, `search-copy-abort.json`, `search-backpressure.json`, `search-paganel-types.json`, `search-console.json`, and `search-pgloader-retry.json`. Passage starts can precede definition lines; read the original file for exact context.

## Changes

Coverage review before the final importer correction found that `pgloader/test/xzero.load` is valid UTF-8 source containing intentional literal NULs. The previous binary heuristic excluded it. A `from_utf8_lossy` search in `ref-mysql_async` and inspection of `src/conn/routines/query.rs:48` showed lossy conversion is used for diagnostic display, not a fidelity guarantee. The chosen correction represents literal NULs as visible `\\x00` in indexed passages, keeps the original file unchanged and original line numbers, and records that display transformation. Acceptance: the source coverage inventory includes `test/xzero.load`, and a query returns that file after rebuilding. Non-UTF-8 inputs remain explicit exclusions.

A subsequent full source-inventory check found `dmt-rs/crates/dmt-rs/src/target/mod.rs` incorrectly excluded by the build-directory name filter. A `target write_chunk COPY` search and inspection of that module confirmed it contains target value types and writer traits, not generated Cargo output; `git check-ignore` did not ignore it. The correction delegates Git-corpus build exclusions to `git ls-files --exclude-standard`, preserving tracked domain modules named `target`. Acceptance: every eligible tracked `.rs`, `.clj`, `.lisp` and `.load` source appears in the coverage manifest, including this module; rebuild/count/query checks must pass again.

- [Reference workflow](../docs/reference-coding.md): search/read/worklog requirements, commands, startup/shutdown, indexing exclusions, updates, provenance and known XERJ limits.
- [Manifest](../docs/reference-repositories.json): seven corpora, pinned upstreams and license observations.
- [Source importer](../bin/reference-index.py): standard-library REST mapping/bulk/count ingestion, 60-line passages, pin validation, explicit coverage and full index replacement.
- [Search helper](../bin/reference-search.py): validated REST query path, scoped search and heuristic symbol lookup, original absolute paths/line metadata.
- Root and my2pg `AGENTS.md`, task dispatch preamble, completion checklist and worklog template require reference research before coding. Documentation indexes point to the workflow.
- Local state/reference checkouts are excluded by the root `.gitignore`; no upstream source changes were made. The temporary pgloader `.xerjignore` used during investigation was removed.

## Indexing evidence

Initial verified source-only build, before the final setup documents were added:

| Corpus | Included text files | Verified passages | Explicit exclusions |
| --- | ---: | ---: | ---: |
| project-my2pg | 17 | 32 | 0 |
| project-pgloader | 909 | 2,461 | 106 |
| ref-paganel | 832 | 2,322 | 88 |
| ref-dmt-rs | 131 | 793 | 5 |
| ref-mysql_async | 58 | 297 | 6 |
| ref-rust-postgres | 160 | 572 | 23 |
| ref-indicatif | 35 | 189 | 8 |

These totals describe that build, not future working-tree changes. The final project refresh and current file lists/counts are in `.reference-coding/reports/<prefix>-coverage.json`. Every source index's accepted document count was checked with XERJ's `_count` API. Exclusions cover hidden/build/binary/non-UTF-8/unsupported-format files and pgloader's bulk `test/data/` fixture directory; source SQL and `.load` tests elsewhere are indexed as text.

## Verification and deviations

| Check | Command or evidence | Result |
| --- | --- | --- |
| Installation | Inspected `https://xerj.org/get`, ran its installer; `rtk run 'xerj --version'` | Passed: rc.80, release archive SHA-256 verified |
| Clone/pins | HTTPS shallow clones; `git rev-parse HEAD`; manifest pin checks | Passed: five clones plus existing pinned pgloader |
| Source import | `rtk run 'python3 my2pg/bin/reference-index.py'` | Passed: all seven bulk responses/counts verified |
| Query correctness | `reference-search.py` commands above; original source inspection | Passed: expected driver, migration and terminal files returned |
| Service persistence | launchd bootstrap/kickstart; search after old process stopped; `_cluster/health` | Passed: source search persisted after restart; cluster green; loopback listeners confirmed |
| Python syntax/help | `PYTHONPYCACHEPREFIX=.../.reference-coding/runtime/pycache python3 -m py_compile ...`; both `--help` commands | Passed on Python 3.9; no third-party Python dependencies |
| macOS service definition | `plutil -lint .reference-coding/xerj.plist` | Passed |
| Independent source inventory | Tracked `.rs`/`.clj`/`.lisp`/`.load` paths compared with per-corpus `included` lists | Passed: every eligible regular source covered, including `xzero.load` and dmt-rs's target module |
| Search regressions | Eight exact expected-file assertions; escaped-NUL fixture assertion; `.reference-coding/reports/verification.json` | Passed: 8/8, with all seven `_count` results equal to their coverage reports |
| Links and clean pins | Local Markdown target checker; Git revisions/status | Passed: 17 Markdown files, 126 local links, six clean pinned upstream checkouts |

XERJ automatic discovery misclassified some source/docs as structured data and extracted no records; a fixture SQL dump expanded into hundreds of MB of prepared database rows. Those automatic jobs were stopped, and their partial user-created indexes were retired. The permanent importer indexes selected source text explicitly and records exclusions; automatic discovery logs/dry-run diagnostics remain in `.reference-coding/reports/`.

The native `xerj search` command returned unrelated license passages for exact symbols against the source mapping. Direct REST `match`/`multi_match`/`match_phrase` probes returned matching implementations. The search helper uses that verified path. Definition detection is heuristic and requires original-source inspection.

The first launchd start raced the old server's data-directory lock and exited. Once the old process released it, `kickstart` started the job successfully and the persisted indexes were searched again. Normal sandbox execution also denied loopback binding/network access; approved tool escalation was used for installation, clones and local networking. The ordinary first syntax check targeted macOS's user cache, so the successful check redirected its cache into workspace state.

## Limits and handoff

No Rust application, Cargo dependency choice, pgloader runtime baseline, MySQL/PostgreSQL migration test, performance benchmark or cutover was executed. T01–T24 remain planned. Reference software's own integration tests were inspected but not run; its README performance claims were not adopted.

References remain broader than our product. Keep only MySQL-to-PostgreSQL behavior when implementing. Paganel's root is AGPL-3.0-or-later; dmt-rs declares MIT but lacks a root license file in this checkout. Research is not approval to copy code or adopt a license. T23 still owns any actual adaptation/provenance decision.

The coordinator starts/rebuilds the service and refreshes pins. Workers use the search helper and record their own reference research before changing code. After logout/reboot, bootstrap the workspace launchd job again as documented. Index rebuilds replace a selected source index temporarily, so schedule them outside active searches and rerun the relevant queries afterward.

## Requested setup recheck — 2026-10-01

Rechecked the official LLM documentation index and the installed binary: `v1.0.0-rc.80`. The local cluster is green. All six upstream checkout revisions match the manifest and have clean Git worktrees. Existing checkouts were reused without advancing their pins.

Refreshed `project-my2pg` after the Rust foundation and fixture files appeared: 43 selected files, 90 passages, three recorded exclusions. The seven indexes now contain 2,171 selected files and 6,728 passages, and every server count matches its coverage manifest. These numbers describe this snapshot; workers continue adding files and the coordinator will refresh it after subsequent changes.

Nine expected-source searches passed using the documented helper: shared `RawValue`, the `single_snapshot` config contract, MySQL `stream_and_drop`, PostgreSQL `copy_in_error`, dmt-rs `WriteJob`, Paganel `convert_type`, indicatif `is_hidden`, pgloader `retry-batch`, and the intentional-NUL `xzero.load` fixture. The initial `single_snapshot` probe expected the scope document, which does not contain that literal; inspection located it in `config-and-cli.md`, and the corrected expected-file check passed. Current verification evidence is in `.reference-coding/reports/verification.json`.

The workflow and both project instruction files already require searching relevant pinned references, reading original implementation and adjacent tests, then recording query/revision/findings/adaptation before coding. The earlier implementation limits above describe the original setup task; the separate active rewrite goal has since started Rust foundation, fixture and transport work. This recheck makes no claim that migration or release checklists are complete.

### Refresh after implementation sources appeared

Re-read `https://xerj.org/llms.txt` and verified the installed binary remains `v1.0.0-rc.80`. All six upstream checkouts are clean and still match their exact manifest pins. Reused those verified checkouts and indexes; no duplicate clones or pin changes were needed.

Paused project-index queries while rebuilding `project-my2pg`: 138 selected source/document/test files, 341 passages and 11 explicit exclusions in this snapshot. All seven server counts match their coverage reports: 2,266 files and 6,979 passages. Eleven expected-file searches passed, including newly indexed `counts_and_schema` in `src/verify/mod.rs` and `copy_bytes` in `src/postgres/mod.rs`, alongside the previous nine checks. The local cluster remains green. The complete query/result inventory is `.reference-coding/reports/verification.json`; these totals are a snapshot while implementation continues.

Verified that the coding instructions and per-task worklogs require searching before coding, reading original implementations and adjacent tests, and recording findings and adaptations. The verifier registration research in T01 demonstrates the workflow: its first project query exposed this stale index, the pinned reference test was inspected, and the integration decision was recorded before the module was wired. `cargo check --locked --all-targets` passed after registration. This setup check does not certify migration correctness or finish release checklists.

### Final requested setup verification after CLI integration

Refreshed the project's index again after the executable and pipeline files appeared: 146 selected files, 405 passages, 11 recorded exclusions. All seven indexes' server counts matched their coverage manifests: 2,274 files and 7,043 passages. All 12 expected-source searches passed, including the new `PipelineError` definition in `src/pipeline/mod.rs`; the cluster was green. Timestamped commands, paths and query matches are in `.reference-coding/reports/verification.json`. This is a working-tree snapshot; subsequent edits require another coordinator refresh.

The six upstream checkouts remain pinned and clean. Five closest reference repositories were already cloned and indexed, with the existing pgloader checkout reused. Installation and the search/read/worklog instructions are complete; no duplicate installation or clones were needed for the repeated request. Indexing is lexical/BM25, and the service remains registered for the current login session rather than automatic login startup.

### Requested setup recheck after M2 module handoffs

Re-read the official LLM documentation index and verified installed XERJ `v1.0.0-rc.80`, running launchd state, valid workspace plist, loopback endpoint and green cluster. All six reference repositories still match their manifest pins and have clean worktrees. Agents froze source edits before the coordinator rebuilt the project index; the existing reference checkouts/indexes were reused.

The refreshed snapshot includes 159 project files and 497 passages, with 11 explicit exclusions. Across all seven corpora, every server count matches its coverage report: 2,287 files and 7,135 passages. All 14 expected-source queries passed, including the new `copy_with_recovery` and `transformation_applied` definitions. An independent Git source inventory confirmed every eligible project Rust source is included. Exact snapshot counts and query paths are recorded in `.reference-coding/reports/verification.json`; subsequent worklog/source edits trigger another coordinator refresh.

The project's workflow, instruction files, manifest and per-task research entries provide the requested search-before-coding setup. Original source and adjacent tests are read after retrieval; the adaptation is recorded before editing. This setup recheck does not close T10–T24 implementation or release gates. Focused M2 behavior has separate worklogs, and recovery executor wiring, transform counters, cross-version checks and performance acceptance remain open.

### Requested setup recheck after recovery and console integration

Re-read the official `https://xerj.org/llms.txt` and the project's reference workflow. Verified the installed binary is still `v1.0.0-rc.80`, the workspace launchd definition passes `plutil -lint`, and the registered service is running with loopback binding and lexical embeddings. Reused the installation and five cloned references plus the existing pgloader checkout. All six upstream checkouts are clean and match their recorded commits; no pins were advanced.

Paused agent queries and edits for the coordinator's project rebuild. The first refreshed snapshot covered 168 project files and 544 passages, with 11 explicit exclusions. Across seven corpora, server counts matched their coverage reports: 2,296 files and 7,182 passages. All 14 expected-file searches passed, including recovery, conversion, verification, driver streaming, COPY completion, bounded queues and terminal behavior. Independent Git inventory confirmed every eligible project Rust source is indexed. After recording this entry and updating the project corpus description to reflect existing Rust sources, rebuilt and verified again; final timestamped counts and complete query results are authoritative in `.reference-coding/reports/verification.json`.

Root and my2pg instructions both require reference searches, original implementation and adjacent test inspection, and query/revision/findings/adaptation entries before coding. Existing per-task research worklogs demonstrate that sequence. Search uses the checked REST helper; lexical/BM25 retrieval and heuristic symbol matching remain explicit limits. The launchd job remains registered for this login session, with restart instructions documented for logout/reboot. This setup verification does not certify the unfinished rewrite, release, cross-version or performance checklists.

### Requested setup recheck after catalog/default/resource checkpoint

Read the official LLM index again and checked installed XERJ `v1.0.0-rc.80`. All worker source edits froze before refreshing the project index at source checkpoint7aeae7a1c474b04fdf67c298d730327bea489c8f.183 selected project files are included, with11 explicit exclusions; every eligible Git Rust source is covered. All seven server counts match their reports, all six reference checkouts remain clean at their manifest pins, and17 expected-source queries pass with a green local cluster. New probes locate MemoryLayout, zero_mantissa and the typed FK operator SQL alongside the prior14 cases. The source/read/worklog sequence was followed for test/module registration and shared resource validation; see the linked increment evidence. The setup is installed and operational; the full rewrite remains active with honest open gates. Documentation changes are followed by another coordinator refresh; final counts/query matches are authoritative in `.reference-coding/reports/verification.json`.

### Requested setup recheck after observed/default/durability checkpoint

Re-read the official llms.txt and local workflow. Verified installed XERJ v1.0.0-rc.80, valid plist, running launchd process92978 on loopback19320 with lexical mode. Existing five cloned Rust references and pgloader remain clean at all six recorded pins. Reused installation/checkouts; no duplicate clone, reference pin change, external embedding/reranking or public feedback submission.

After worker handoffs froze and source checkpoint `bd76d81f39271e6c22ad3cad204c5ccbb2181d55` saved passing code, refreshed project coverage:204source/document/test files,703passages,11explicit exclusions. All seven server counts matched coverage:2,332files/7,341passages. All21expected-source queries passed with green health, including new read_sequence_state, compare_with_catalog, native-default extraction and LOSSY_LABEL_METADATA. Every eligible Git Rust file is covered; pinned reference worktrees are clean. Complete counts/query paths/timestamp remain in `.reference-coding/reports/verification.json`. Documentation is saved and refreshed again after this entry; that final report is authoritative.

Reference retrieval materially improved the rewrite: original MySQL server code and independent native inserts identified NUL/default and supplementary-label metadata loss that a plan-based verifier missed. Task worklogs record query/pin/original/adjacent tests/adaptation before changing code. Correct literal defaults are now independently proven, and ambiguous ordered labels refuse conversion. The [coordinator checkpoint](2026-10-01-M2-observed-durability-checkpoint.md) records100ordinary/87fresh development cases plus the separate NLS case. Full rewrite milestones, parallelism, full matrix and benchmark/release acceptance remain open; XERJ setup completion does not close them.

### Repeated setup request — graph, importer and runner working-tree refresh

Re-read official llms.txt, the checked REST helpers and original pinned dmt-rs transfer361–442 / rust-postgres COPY-abort test738–770. Installed rc.80 and the running loopback lexical launchd job are verified; the plist passes lint. Reuse the existing installation and pinned checkouts. The independent [setup review](2026-10-01-reference-setup-review.md) verifies six clean pins, seven matching server/coverage counts, green health and21/21 expected-source searches against the prior444f9a1 snapshot. It correctly identifies six new Rust files absent from that snapshot. No reference pins or code were changed for this repeated setup request.

All workers acknowledge a brief edit/query freeze before the coordinator rebuilds project-my2pg. The refresh includes current graph SQL/decoders, importer and concurrency work, plus tests and worklogs. This is an **in-progress working-tree snapshot at HEAD444f9a1**, not a new passing implementation checkpoint. Native tests may continue reading frozen sources during indexing. Add explicit expected-file searches for decode_dependency_graph, ImportOptions and ExecutionRegistry alongside the21 previous cases; verify every eligible Git Rust path, all server/coverage counts and exact current project passages. Final timestamped counts, query matches and working-tree/content verification are authoritative in `.reference-coding/reports/verification.json`. Subsequent source changes require another coordinator refresh.

README/importer documentation now reflects the implemented offline subset and explicitly refused future hooks/readers. Reference coding is operational: search the project and relevant pinned implementation, read originals and adjacent tests, record query/revision/findings/adaptation before code, then prove the chosen behavior independently. Lexical retrieval, heuristic symbols, current-session service registration and licensing boundaries remain explicit. The full rewrite goal and unfinished milestone/release gates remain active.

### Final refresh after the reviewed source checkpoint

Worker handoffs and root integration are frozen at source61dc35cd8279afcde79b3bb6bd423f1159958775. The [coordinator checkpoint](2026-10-01-M2-parallel-graph-import-checkpoint.md) records121ordinary/102fresh development database passes, a separately strengthened durable-cap case, stopped fixtures, unchanged lock and honest remaining gates. After saving these docs, refresh project-my2pg again and rerun all24expected-file queries, six clean-pin checks, seven count checks, eligible-Rust coverage and exact current project passage comparison. The final report timestamp/revision/counts/hash are authoritative; earlier221/810 and2349/7448 totals are the prior working snapshot. No pins or reference indexes need rebuilding. No external feedback, embedding proxy, reranking or public publication is part of setup.
