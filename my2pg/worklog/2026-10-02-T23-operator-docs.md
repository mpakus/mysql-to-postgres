# T23 operator documentation and examples

## Scope and baseline

- Task: close the operator-documentation and documentation-example release checklist items using behavior present in the current source.
- Baseline revision: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`.
- XERJ searches / code reference adaptation: none; this is documentation and acceptance evidence only, with no code edits.
- Read `docs/config-and-cli.md`, `docs/architecture.md`, `docs/checklists.md`, `docs/examples/mysql-to-postgres.toml`, `docs/examples/pgloader-subset.load`, credential resolution in `src/config/validation.rs`, and the package link test in `tests/support/test_package.py`.

## Evidence and changes

- `my2pg check docs/examples/mysql-to-postgres.toml --output json` succeeded with disposable localhost URL values supplied through the example's environment variable names; the command did not connect to either database.
- `my2pg config import docs/examples/pgloader-subset.load --output /private/tmp/my2pg-doc-import-check.toml --target-schema legacy --consistency frozen --use-my2pg-defaults --quiet` succeeded. The first generated-config check correctly failed because the default credential variables were unset; supplying disposable localhost values then made `my2pg check /private/tmp/my2pg-doc-import-check.toml --output json` succeed. This is evidence of the documented environment-variable contract, not a migration test.
- `tests/support/test_package.py` checks links for Markdown shipped in the Cargo archive. Run that suite after doc edits.
- Source evidence confirms URL credentials may use `url_env` or owner-private `url_file`; PostgreSQL `passfile` is supported. There is no implicit MySQL option-file lookup.
- Architecture documents no automatic resume; reruns must start from a fresh or explicitly reviewed/recreated target. Reject records are evidence and are not automatically replayed.

## Adaptation

Update the stale configuration-document introduction and command framing to match implemented behavior, clarify credential file choices, and add actionable manual restart guidance that preserves partial artifacts and requires review before destructive target policies. Mark the two T23 documentation checklist items complete only after CLI parser/import checks and shipped-link tests pass.

## Validation

- `python3 tests/support/test_package.py` passed 3/3. Its first run found two links from shipped `docs/config-and-cli.md` into maintainer-only docs excluded from the Cargo archive; those links were replaced with a source-checkout note, and the rerun passed.
- `MY2PG_MYSQL_URL=mysql://reader:secret@localhost:3306/shop MY2PG_POSTGRES_URL=postgresql://writer:secret@localhost:5432/warehouse target/debug/my2pg check docs/examples/mysql-to-postgres.toml --output json` returned `status: valid`; it used the configuration parser and did not connect.
- `target/debug/my2pg config import docs/examples/pgloader-subset.load --output /private/tmp/my2pg-doc-import-check-2.toml --target-schema legacy --consistency frozen --use-my2pg-defaults --quiet` completed; checking that generated config with the documented `MY2PG_SOURCE_URL` and `MY2PG_TARGET_URL` references returned `status: valid`.
- Removed the temporary generated configuration after validation. No live migration was run.
- `git diff --check` passed. The initial sandboxed XERJ refresh could not reach the loopback service; the coordinator reran the project-only refresh with local-network access and it completed at 385 files, 1,346 passages, and 11 exclusions. The worklog update itself will be included by the final refresh.

## Limitations

These checks validate local parsing, import, and links in the packaged Markdown set. They do not certify a database migration, every `.load` clause, or the remaining release gates listed in `docs/checklists.md`.
