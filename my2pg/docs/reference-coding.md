# Reference coding with XERJ

Before implementing or changing my2pg code, search this project's specifications and the relevant reference implementations. Read the original files and nearby tests, then record the evidence and the chosen adaptation in the task worklog **before coding**. Searches guide design; this project's MySQL-only contracts and independent tests determine correctness.

## Installed setup

- XERJ `v1.0.0-rc.80`, installed at `/Users/renatibragimov/.local/bin/xerj` through the official installer with its release SHA-256 verified.
- Local endpoint: `http://127.0.0.1:19320`; native listeners also use loopback ports `19321` and `19322`.
- Server embedding mode is `lexical`. These source indexes use ordinary text/BM25 queries; this setup does not provide neural semantic search.
- Data, logs, coverage reports and the launchd definition live under the sibling `.reference-coding/` directory, outside every indexed corpus. No embedding proxy or external reranker is configured.
- The macOS job is `gui/<uid>/org.my2pg.xerj`. It is registered for this login session using the workspace plist; it is not installed as a login-startup agent. After logout/reboot, run the start command below.

The [repository manifest](reference-repositories.json) records exact commits, source folders, prefixes, purposes and observed licenses. Reference repositories live in the sibling `references/` directory; the existing pgloader checkout is reused. No reference code is linked into my2pg.

| Search prefix | What to look for |
| --- | --- |
| `project-my2pg` | Scope, architecture, tests, task decisions and current Rust implementation |
| `project-pgloader` | Original MySQL casts, DDL, COPY recovery, parser and regression fixtures |
| `ref-paganel` | Rust schema conversion, batching, content verification and migration tests |
| `ref-dmt-rs` | Rust MySQL-to-PostgreSQL orchestration, bounded queues, cancellation and COPY |
| `ref-mysql_async` | MySQL row streaming, raw values, result cleanup, pool and authentication |
| `ref-rust-postgres` | tokio-postgres COPY sink completion, transaction cleanup and protocol tests |
| `ref-indicatif` | stderr drawing, TTY detection, concurrent progress and terminal tests |
| `ref` / `project` | Search all reference / project indexes respectively |

## Before each coding task

1. Read the task's contract and dependency worklogs. Confirm that the server is available and the coverage report includes the files you need.
2. Search `project-my2pg` for the requirement, then the relevant reference prefix for the problem. With lexical search, use concrete identifiers and technical terms; try several terms when a broad question misses.
3. Use `--symbol` when you know a definition name. Open the original implementation and adjacent tests with `rtk`; a returned passage may begin before the relevant function.
4. Write a brief **Reference research** section in the task worklog: query, prefix, pinned commit, file and line, observed behavior, useful test, what we will adapt or reject, and the test that will prove our behavior. Record unsuccessful searches too.
5. Implement the smallest complete solution using the required RTK and Ponytail guidance. Keep only MySQL as a source and PostgreSQL as a target. Add independent behavioral expectations, then verify the change.

From the workspace root:

```sh
rtk run 'python3 my2pg/bin/reference-search.py project-my2pg "COPY rollback uncertain commit"'
rtk run 'python3 my2pg/bin/reference-search.py ref-mysql_async stream_and_drop --symbol'
rtk run 'python3 my2pg/bin/reference-search.py ref-rust-postgres copy_in_error --symbol'
rtk run 'python3 my2pg/bin/reference-search.py ref-dmt-rs "bounded WriteJob"'
rtk run 'python3 my2pg/bin/reference-search.py project-pgloader retry-batch --symbol'
rtk run 'python3 my2pg/bin/reference-search.py ref-indicatif is_hidden --symbol'
rtk run 'sed -n "735,765p" references/rust-postgres/tokio-postgres/tests/test/main.rs'
```

The helpers resolve corpus paths from their own location, so they also work when invoked by absolute path from a worktree or another directory. Add `--json` to save raw search evidence, `-k N` for more results, and `--full N` to control passage length. Symbol detection is a small heuristic for Rust/Lisp/Clojure declarations, not a complete AST or call graph. Verify definitions in the original file.

## Start, stop and inspect

The plist contains this machine's absolute binary/workspace paths. Update those paths when moving the workspace. Start once after login; if the job is already registered but stopped, use `kickstart` instead of another `bootstrap`.

```sh
rtk run 'launchctl bootstrap gui/$(id -u) /Users/renatibragimov/www/pg/.reference-coding/xerj.plist'
rtk run 'launchctl kickstart gui/$(id -u)/org.my2pg.xerj'
rtk run 'launchctl print gui/$(id -u)/org.my2pg.xerj'
rtk run 'curl -fsS http://127.0.0.1:19320/_cluster/health'
rtk run 'launchctl bootout gui/$(id -u)/org.my2pg.xerj'
```

Run only the applicable lifecycle command, not this entire block sequentially. Without launchd, run the same binary in a foreground terminal with `--insecure --bind 127.0.0.1 --port 19320 --embed-mode lexical --data-dir /Users/renatibragimov/www/pg/.reference-coding/runtime/data --disable-feedback`. Stop the registered job first; two processes must never share a data directory. Shell sandbox restrictions on loopback networking may require the execution tool's normal approval mechanism.

## Rebuild and refresh

The coordinator owns shared index rebuilds. A rebuild temporarily replaces the selected source index; do not search it concurrently or count a failed/partial rebuild as current evidence.

```sh
# Rebuild the working project's sources after implementation changes.
rtk run 'python3 my2pg/bin/reference-index.py project-my2pg'
# Rebuild all seven pinned corpora.
rtk run 'python3 my2pg/bin/reference-index.py'
```

`reference-index.py` uses Python's standard library and XERJ's REST mapping/bulk/count APIs. Each corpus gets `<prefix>-source`, split into passages of 60 original lines with path, line, repository and revision metadata. It verifies every bulk response and final document count. Each successful run writes `.reference-coding/reports/<prefix>-coverage.json`, listing all included paths and exclusions. Existing same-prefix source documents are replaced, so removed/renamed files do not remain searchable. Read failures, pin mismatches, ingest errors and count mismatches fail the run. Preserve previous evidence and do not claim a corpus passed after an error.

Git corpora include tracked files plus non-ignored working files. Corpora without Git metadata are walked directly. The source/document/test format allowlist is explicit in the helper. Hidden files, symlinks, build directories, unsupported formats, non-UTF-8 content and pgloader's bulk `test/data/` fixtures are excluded and recorded. Source SQL, `.load` files, `.patch` files and assertion/expected-output fixtures outside that bulk-data directory remain searchable. Intentional literal NULs in UTF-8 source fixtures are displayed as `\\x00`, with the transformation recorded and original line numbers retained; read the original file for exact bytes. Large fixture databases and archives can still be read locally when a task needs them. Future formats require a reviewed allowlist change and another coverage check.

References were shallow-cloned from public HTTPS URLs and pinned in the manifest. To refresh one, the coordinator fetches its upstream, inspects the proposed commit and source changes, updates the manifest's pin deliberately, rebuilds that prefix, reruns representative searches, and records the revision and results. The helper refuses a changed HEAD until the pin is updated. Record any local reference changes separately; a commit hash alone does not prove an unmodified worktree. Do not run cloned projects' setup scripts just to search their code.

## Verified XERJ limits in this release

The official `autoindex` dry runs classified some `.load`, Clojure, Markdown and SQL files as structured datasets and extracted zero records; they also expanded SQL dumps into rows. The source importer explicitly indexes text rather than applying database/data-format inference.

The native `xerj search` command returned unrelated license passages for exact `stream_and_drop` and `copy_in_error` queries against the source mapping. Direct REST `match`, `multi_match` and `match_phrase` queries returned the expected source files. Use `reference-search.py` for this setup; its query path has been checked. Do not substitute the native search command and assume equivalent ranking. Raw probe results and the retired automatic-index logs are retained in `.reference-coding/reports/`. These are local observations, not claims about all XERJ releases.

Official sources: [LLM documentation index](https://xerj.org/llms.txt), [installation](https://xerj.org/docs/install), [CLI](https://xerj.org/docs/cli), and [automatic indexing](https://xerj.org/docs/recipes/zero-config-autoindex). The installed CLI help and local responses supplied the version-specific evidence above.

## Findings already useful to the rewrite

| Problem and task | Read implementation and test | Adaptation for my2pg |
| --- | --- | --- |
| Streaming without materializing a table — T03/T05 | [mysql_async streaming](../../references/mysql_async/src/queryable/query_result/result_set_stream.rs), definition at line 343; [query-result tests](../../references/mysql_async/src/queryable/query_result/tests.rs) | Stream one result set while preserving ownership/cleanup. Do not copy example `try_collect::<Vec<_>>` into the production reader. Spike cancellation and reuse with real MySQL. |
| COPY completion and abort — T07/T12 | [COPY sink](../../references/rust-postgres/tokio-postgres/src/copy_in.rs), finish at line 128; [COPY-abort test](../../references/rust-postgres/tokio-postgres/tests/test/main.rs), `copy_in_error` at line 739 | Explicitly finish COPY, distinguish that from transaction commit, and prove dropped/failed COPY inserts nothing in its transaction. A dropped-COPY test does not prove recovery after an uncertain commit. |
| Backpressure — T13 | [dmt-rs pipeline](../../references/dmt-rs/crates/dmt-rs/src/transfer/mod.rs), channels at lines 414–418; [signal checks](../../references/dmt-rs/scripts/test-signals.sh) | Use bounded queues; also enforce my2pg's global byte budget, since a fixed count of chunks alone does not bound bytes. Test blocked producer/consumer cancellation. |
| Unsigned integer fidelity — T10 | [Paganel converter](../../references/paganel/crates/engine-schema/src/converters/to_postgres.rs), warning mapping at line 65; [schema test cases](../../references/paganel/crates/engine-tests/src/schema/) | Study typed conversion outcomes, but reject its warned `BIGINT UNSIGNED → BIGINT` narrowing for our default contract. Preserve the full unsigned range and test maxima independently. |
| Progress when redirected — T15 | [indicatif draw target](../../references/indicatif/src/draw_target.rs), `is_hidden` at line 136 and `multi_is_hidden` at line 753; [multi-progress lifecycle](../../references/indicatif/src/multi.rs) | Draw progress on stderr only for a terminal, preserve stable plain/JSON output, and test hidden targets plus redirected CLI behavior. |
| Bad-row isolation — T12 | [pgloader Lisp retry](../../pgloader/src/pg-copy/copy-retry-batch.lisp), [Clojure batches](../../pgloader/clojure/src/pgloader/batch.clj), and [COPY tests](../../pgloader/clojure/test/pgloader/copy_test.clj) | Compare subdivision and error handling with our SQLSTATE/commit-ambiguity rules. Reuse behavior cases with provenance, not every legacy retry policy. |

These files were read and searched locally; their integration tests were not executed by the setup task. See [the setup worklog](../worklog/2026-10-01-reference-coding.md) for commands and evidence.

Paganel's root declares AGPL-3.0-or-later and components may have different licenses. dmt-rs declares MIT in Cargo metadata but has no root license file in the pinned checkout. Use these as design references and implement our reviewed contract independently. Any copied/adapted code or fixtures need provenance and an explicit license decision during T23; preserving a clone is not a decision to adopt its license or dependency structure.
