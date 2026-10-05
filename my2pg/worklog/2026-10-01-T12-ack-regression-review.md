# 2026-10-01 — T12 acknowledged-prefix regression review

Status: read-only peer review; production/tests/indexes unchanged by this reviewer. HEAD `444f9a119ed26e2c7e287b2fd17b2e92f2e66638` with coordinator/worker changes in the working tree. Reviewed the coordinator's two repaired `tests/cases/t12_runner.rs` cases and the newest section of `worklog/2026-10-01-T13-integration-review.md`. Production SHA recorded by the coordinator: `d6dce1f9f87eaaf89210b0d606a71978c346c0c2b3ef1687729bdbf6636a43ea`. This is not a new native execution result.

## Reference evidence

RTK/Ponytail guidance applied. Read `docs/reference-coding.md`, the actual fixture/setup/run/persisted-report validators, current producer/consumer and teardown, recovery rejection admission, source ordering and planner table traversal. XERJ `project-my2pg`, query `acknowledged prefix conversion reject_limit`, at indexed revision `444f9a119ed26e2c7e287b2fd17b2e92f2e66638`, returned the T13 review, producer rejection-limit branch and earlier recovery evidence. The shared project index is an earlier working-tree snapshot; live originals supply the current repaired tests and runner behavior.

XERJ `ref-dmt-rs`, query `WriteJob channel`, pin `4e8015f7e841dbdf9df01e953aeb3948dfb3199a`, located bounded transfer channels. Read original `references/dmt-rs/crates/dmt-rs/src/transfer/mod.rs:405–440` and adjacent `references/dmt-rs/scripts/test-signals.sh`. Channel enqueue and a fixed delay do not establish PostgreSQL COMMIT acknowledgement. Chosen test adaptation is to establish an acknowledged prefix through completed ordered table execution; no reference implementation was copied and no product timing controls were introduced.

## Fixture and ordering proof

The fixture uses `single_snapshot`, includes its `_a` and `_b` source tables, and creates empty targets in a schema-only run before switching to data-only append. `src/mysql/tables.sql:3` orders by binary table name; planner traversal preserves selected source order. Thus `_a` precedes `_b`.

`src/pipeline/mod.rs:949` executes SingleSnapshot tables sequentially and awaits `table_pipeline` before advancing. `table_pipeline` awaits both producer and consumer with `tokio::try_join!` (`:1327`). A completed A therefore establishes acknowledged consumer COPY/COMMIT results before B is read. The new fixture does not depend on within-table read-ahead timing. Independent target SELECTs prove rows were actually inserted, rather than merely queued or inherited from setup.

One shared `RejectBudget` is constructed before table execution (`src/pipeline/mod.rs:934`). Conversion and COPY recovery both reserve that budget before recording artifacts and commit the ticket only after successful durable artifact recording. The same budget reaches both tables. The cap fixture retains A's four source rows: good id1, conversion-invalid id2, target CHECK-invalid id3, good id4; B contains only conversion-invalid id2. Expected accounting is A `(read, committed, rejected, unresolved, indeterminate) = (4,2,2,0,0)`, B `(1,0,0,1,0)`. B cannot spend another rejection ticket after A completes two mixed rejects.

The stop fixture retains only good id1 in A and bad id2 in B. Expected tuples are A `(1,1,0,0,0)` and B `(1,0,0,1,0)`. Its actual target A `[1]` and B empty, exact read/committed/unresolved tuples, zero aggregate rejection count, and absence of all reject files independently cover the acknowledged-prefix and no-reject obligations. `Fixture::validate_report` checks actual `report.json` equality, private permissions and `accounted_rows == rows_read`; together with those exact tuples this also excludes hidden indeterminate rows.

## Review finding and limits

No product defect or invalid fixture ordering found. The original queued-prefix assertions were invalid under concurrent producer/consumer execution; the repaired two-table fixtures preserve the stronger observable obligation of retaining an already acknowledged earlier table.

At review time, the cap case asserts target A `[1,4]`, B empty, aggregate committed2/rejected2/unresolved1, B read1/committed0/unresolved1 and absence of B's reject metadata. One remaining test-strength gap was reported to the coordinator: it does not independently inspect A's two durable reject records or assert A's full tuple. Add the expected A tuple above, two metadata records with conversion/COPY kinds, and exact A COPY reject bytes `3\tserver\n`, using the existing helper as in the adjacent exact-global-limit case. Report self-consistency alone does not prove an independent source row count or durable rejection inventory. This recommendation requires no product change.

The coordinator accepted the recommendation. Its current full native rerun is executing frozen source; it will add only the stronger A/B accounting and durable-artifact assertions after that run ends, record exact test hashes, and execute the strengthened cap case before checkpoint. That refinement is not covered by the already running full suite.

The coordinator's pre-repair fresh native run was reported as 100 passed/2 failed; those failures and the stopped pair are recorded in the integration worklog. This reviewer did not start a database pair, rebuild an index, or run native tests. Focused repaired cases and the coordinator's fresh complete native rerun remain required. This review worklog is frozen; no source/test edits were made by the reviewer.
