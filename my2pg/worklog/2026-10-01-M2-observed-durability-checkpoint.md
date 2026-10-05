# M2 observed catalog, source defaults and durability checkpoint

Source checkpoint: `bd76d81`. This is bounded progress toward the active full rewrite goal, not M2 or release acceptance. T10 is reopened; T11/T12/T13/T15 remain in progress. T14 remains accepted on the development pair. No speed, cross-version, cutover or physical storage-sync claim.

## Changes and reviewed evidence

- [Typed target inspector](2026-10-01-T11-observed-decoder.md): actual table/column/type/index/constraint OIDs and ordinals, sequence bindings/options/privileges/state, local/shared dependency facts, trigger/function provenance and all database event triggers. One driver-owned RR read-only catalog transaction; missing/bad binary aliases propagate errors; uninspected None differs from required known-empty inventory. Successful rollback ends inspection. Metadata does not prevent external DDL or prove recursive/dynamic dependency safety; sequence state is nontransactional.
- [Sequence/trigger SQL](2026-10-01-T11-sequence-inspection.md): independently tested headers/direct dependencies, restricted privileges, O/D/R/A and internal triggers, global event inventory. Root integrated the production decoders/state reads and obtained an independent peer review.
- [Source literal defaults](2026-10-01-T10-binary-default-metadata.md): independent default inserts exposed cached NUL truncation and supplementary-Unicode loss. Guarded native DEFAULT(column) reads recover actual binary/character values on empty tables without source writes or arbitrary expression evaluation. Source inspection, production plan and actual PostgreSQL default inserts equal independently inserted MySQL defaults.
- [Ordered label safety](2026-10-01-T10-enum-label-metadata.md): native empty ENUM/SET catalogs cannot distinguish supplementary-label replacement from literal question marks. Selected ambiguous labels block before casts/ordinal conversion/target DDL. Known BMP-only charset proof permits literal question marks; absent/unknown/supplementary charsets refuse them. Lossless supplementary label recovery remains unsupported; unselected tables stay usable.
- [Finite default comparison](2026-10-01-T15-default-completion.md): JSONB exact semantics/duplicate keys/reserved keys/numeric boundaries, INET address/prefix, all4 interval styles, bytea hex/escape, enum OID identity and CURRENT_TIMESTAMP precision. No arbitrary expression evaluation. The original source-default failures remain preserved, and separate source corrections now pass together with this verifier.
- [Actual durability and locale faults](2026-10-01-T12-durable-faults.md): physical owned64MiB ENOSPC28 preserves prior durable ACK reports and prevents replay; child-only verified production descriptor fsync EINVAL22 prevents second rejection/budget acknowledgement. Separate official NLS image observes a real French builtin CHECK error and SQLSTATE23514 recovery. Descriptor failure is not physical storage-sync failure; forced-detach fsync success is negative evidence.

## Integrated checks

Source edits froze before final checks; no source edits followed these passing commands before the source checkpoint. Documentation and research notes are saved separately.

```sh
rtk run 'bin/cargo fmt --all --check'
rtk run 'bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings'
rtk run 'bin/cargo test --offline --locked --all-targets --all-features'
rtk run 'python3 tests/support/test_harness.py'
rtk run 'python3 tests/support/fixture_manifest.py'
rtk run 'tests/run-integration.sh mysql84 pg16'
```

All exit0. Ordinary Cargo tests100=81library+13contracts+1driver+5integration;76 database cases and the1feature-required NLS case remain ignored in this ordinary lane, not counted as executed. Harness isolation/evidence cases5/5 pass. Upstream manifest still verifies13load files/18parser cases/19assertion cases/65case IDs and all pinned fixture hashes. Formatting, strict all-feature lint and diff checks pass. Cargo.lock SHA256 remains `9614e0c812299388633ef0cf218f7b305ae79ab10ad494c5cd3bebdc80ea2d23`.

Fresh native fixture `my2pg-mysql84-pg16-599d0586f3f4`: native aarch64 Oracle MySQL8.4.11/PostgreSQL16.15, unchanged development image pins,14seed checks,87passing actual database cases=11driver+76integration,0failures. Tests inside this owned fixture are serial because event-trigger DDL is database-global; independent worker fixtures remain parallel. Native suite29.70s for the integration binary. Source evidence records parent `776901a1377fbea654bdf19f7a201014af757fbc` with dirty=true; checkpoint bd76d81 saves that exact tested source. Fixture metadata independently reads stopped. Log SHA256 `8dbb1036ed0a3a94999f6e08fee8517a30a539d7f94d30a84579645bae112fa6`; harness SHA256 `fc551ef619c4d97a19091e55e3b3fcce87691e7c2a09cff8a8682021e903e692`.

Separate NLS case1/1 passed0.31s on owned `my2pg-mysql84-pg16-nls-c26ea35d8f34`, official PG16.15 Debian ARM64 manifest `1c2f3efc9c5ab63fe557565c9443dbcfc0cecd2b28a3989244dfc80eb6cb96f9`, actual --enable-nls/catalog/fr_FR locale. Its exact prerequisite/restart/port update and structured error evidence are in the task log. It tests unchanged production COPY/recovery/report paths; it is not part of the87-case Alpine run or a full Debian matrix. All worker/coordinator fixtures are stopped; task-owned disk-image states are detached and evidence retained.

## Failures retained and corrections

The first fresh integrated fixture d196dc46df6e passed86cases but failed the old test-only DEFAULT_UNSUPPORTED assertion compiled before its owner changed it to the earlier LOSSY_LABEL_METADATA refusal. Original failed evidence/log is retained; that run is not acceptance. Final frozen rerun above passes the corrected independent expectation. Its --nocapture output also exposed failure-ID capture missing interrupted status lines; the harness now parses the stable final failure list. A regression and independent replay of the original captured failed log identify the exact failed case. Mid-edit formatting/Clippy/descriptor-probe failures are documented in their task logs and not counted as acceptance.

## Next required work

T11 needs complete object/recursive dependency/type inventories and reviewed existing-target policy/identity behavior. T13 resource helpers are not a parallel runner: bounded table/index tasks, common connections, byte permits, shutdown and RSS/fault evidence remain required. T12 physical storage-sync and combined uncertain-commit/storage failure proof remain open. T10 lossless supplementary ENUM/SET label support needs a real source of ordered labels; current explicit refusal prevents corruption. T15/T18 full required verification and content comparison remain distinct. T16–T24, all version/platform lanes and declared correct-run benchmarks remain planned gates. No grouped milestone box is checked by this increment.

Coordinator refreshes XERJ after recording documentation; final corpus counts/pins/query matches are in `.reference-coding/reports/verification.json` and the [reference setup log](2026-10-01-reference-coding.md).
