# 2026-10-01 — T16 — offline importer command integration

Status: preparation/integration, not full M3 acceptance. Base444f9a119ed26e2c7e287b2fd17b2e92f2e66638. Root owns cli.rs/main.rs/module registration; F owns the parser/publisher and importer tests. See [parser research](2026-10-01-T16-load-import.md).

## Reference research before code

RTK/Ponytail guidance applied; reuse the existing clap command and config types, offline strict parser and single-artifact publisher. XERJ `project-my2pg`, `import config source spans unsupported atomic`, at444f9a1 returned the T04 worklog and classified fixture manifest. `project-pgloader`, symbol `load-mysql-command`, at231ab86778ca5ffd7de40878714760c8b4860cdf returned no hits: heuristic symbol detection does not cover DEFRule. Read original command-mysql.lisp including the actual LOAD grammar and credential fallback/evaluator, original test/mysql/my.load and our subset fixture, config-and-cli.md importer table, CLI and adjacent parsing/config tests, main dispatch and F's proposed API. We do not execute legacy Lisp, resolve environment credentials, connect databases or reproduce implicit casts during import.

## Chosen adaptation

Expose explicit target namespace, consistency, credential variable names, data-only append choice and reviewed my2pg default opt-in. Keep RequireExplicitChoice as parser default: implicit pgloader type/default differences require an explicit choice. Both existing command aliases invoke the same parse/publish path. Read at most1MiB+1 using stdlib before the parser; oversized/non-UTF8/read errors are secret-free diagnostics. Fully parse/validate before creating output; publish private atomic no-clobber TOML with embedded compatibility report, then emit sanitized compatibility JSON unless quiet. Input/output paths and raw URIs never appear in generated diagnostics. No new dependencies.

F's tests must exercise CLI success, default-policy refusal, unknown/non-MySQL clauses, secret redaction, alias parity and preservation of an existing output. Parser success does not prove database migration behavior; T16 remains a preparation task until dependent M2 contracts and accepted-clause behavior are complete.

## Final integrated evidence

All18importer tests pass in the121-case ordinary suite, including both actual CLI aliases, offline check, strict guards, classification and planner/encoder behavior. Strict all-target/all-feature Clippy and formatting pass. Source61dc35c saves the implementation; operator README/config docs show mandatory explicit schema/review flags and refuse unimplemented hooks/readers/view/decoding translations. [Checkpoint](2026-10-01-M2-parallel-graph-import-checkpoint.md) records native core integration separately; it is not importer-driven native compatibility proof. T16 is in_progress preparation, not complete.
