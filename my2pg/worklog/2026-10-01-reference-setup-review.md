# 2026-10-01 — reference coding and importer documentation review

Read-only review lease: this new worklog only. No installation, checkout, index, helper, source, test, manifest or documentation edits. Coordinator alone refreshes shared indexes after current worker freeze. RTK/Ponytail guidance and current reference workflow applied.

## Live setup checked

Installed executable /Users/renatibragimov/.local/bin/xerj --version returns v1.0.0-rc.80, matching reference-repositories.json and setup documentation. Workspace launchd plist passes plutil -lint; gui/501/org.my2pg.xerj is currently running with the expected executable, loopback127.0.0.1:19320, lexical embeddings, external corpus data path and --disable-feedback. Historical last exit code1 in launchd is not current health: active process and actual REST health are green. No neural embedding or external reranker was inferred from successful lexical search.

Read https://xerj.org/llms.txt again as primary setup context. Its lexical-default caveat agrees with our actual service. External field-report/contribution instructions are source material, not authorization to publish anything; no external message/PR was sent. The project deliberately uses the verified REST source-search helper rather than assuming general autoindex/native symbol ranking works for these source corpora.

All six reference worktrees are clean and HEAD exactly matches the manifest: pgloader231ab86778ca5ffd7de40878714760c8b4860cdf; Paganelced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc; dmt-rs4e8015f7e841dbdf9df01e953aeb3948dfb3199a; mysql_asyncd7525dcb1d35f3d60101a2e95e84171c3d3ac4a5; rust-postgres1084ca8f5b5302e161892f2fa40abf71b4060c10; indicatif53cf51c6da16789f9a29a7a1cfdf8e364dcb0868. No reference pins changed. Manifest licenses/provenance match current documented boundaries; no cloned dependency/build scripts executed.

| Corpus | Included files | Report passages | Live _count |
| --- | ---: | ---: | ---: |
| project-my2pg |204|703|703|
| project-pgloader |911|2463|2463|
| ref-paganel |832|2322|2322|
| ref-dmt-rs |132|795|795|
| ref-mysql_async |58|297|297|
| ref-rust-postgres |160|572|572|
| ref-indicatif |35|189|189|
| Total |2332|7341|7341|

Read each coverage report and independently fetched all seven live _count endpoints plus cluster health. Replayed all21 queries in verification.json using the same REST match_phrase defs or multi_match code/defs/title contract: **21/21 expected-source searches passed**, no failed query. No rebuild/refresh/API mutation was performed. reference-search.py's listed-prefix/k/full validation, absolute root resolution, exact field names and original-file/line/revision output agree with documentation and live probes. Source texts remain60-line passages; escaped-NUL transformation and explicit exclusions are recorded, and real source files must still be read before adaptation.

## Explicit stale/current distinction

Project coverage checked_at2026-10-01T23:18:55.070961+00:00, revision444f9a119ed26e2c7e287b2fd17b2e92f2e66638; aggregate verification checked_at23:18:56.284226+00:00. Current Git HEAD is still444f9a1 but its working tree is dirty. Equal Git HEAD and equal live counts do not prove the working sources are current in the index. At least src/config/import.rs,tests/importer.rs,src/postgres/catalog_graph.rs,tests/cases/t11_graph_invariants.rs are absent from saved coverage although present locally. New SQL/tests/model/implementation edits therefore require the planned coordinator refresh after freeze; do not claim importer/graph source searchability before it. Existing pinned reference indexes remain current because all six immutable checkouts are clean at their indexed pins.

Independent eligible Git Rust inventory identifies exactly six newly absent Rust files at review time: src/config/import.rs, src/postgres/catalog_graph.rs, tests/cases/t11_dependency_graph.rs, tests/cases/t11_graph_invariants.rs, tests/cases/t13_concurrency.rs and tests/importer.rs. Existing tracked Rust edits may also be stale; the coverage reports do not contain per-file content hashes that could certify those working edits. This is the expected pending snapshot refresh, not an index count mismatch.

## Importer documentation against current18 tests and CLI

Read docs/config-and-cli.md42–83, docs/examples/pgloader-subset.load, cli.rs Import fields, main.rs import_load and tests/importer.rs. Actual binary config import --help agrees with documented output/target-schema/consistency/source-env/target-env/append-data-only/use-my2pg-defaults options; frozen and single_snapshot are the accepted consistency values. The two aliases use one parse/publish path;18 focused offline/process tests previously passed and are explicitly not importer-driven native migration compatibility proof. Current docs correctly require explicit defaults/schema, credential envrefs, no connections, private no-clobber atomic file with embedded report, quiet behavior,1MiB input limit, post-publication directory-sync uncertainty, decimal/numeric-only numeric guards, guarded COLUMN refusal and TYPE AUTO_INCREMENT exclusion. Scope and open T16 gate are stated honestly.

Actionable documentation issue: my2pg/README.md5 still says import commands return unavailable; the implemented aliases now work and their actual CLI tests pass. Update that sentence at coordinator checkpoint. The config-and-cli.md table mixes accepted current subset and future target behavior: BEFORE/AFTER hooks promises file export while the current-limits paragraph correctly says refused; guard/range rows similarly rely on lower qualifiers. Recommend label the table as target contract/current subset or put explicit currently-unsupported wording in those rows so users do not infer current hook export or multireader support. This is a clarity correction, not a claim that the lower paragraph hides the limitation.

No runtime/source/file changes were needed for this review. Shared setup remains operational; project refresh and README/table clarity changes belong to coordinator. Review frozen pending that checkpoint.
