# T15 — verify the live ordered ENUM label catalog

Status: research recorded before code; narrow lease verify/mod.rs, new verify/enum-labels.sql, new cases/t15_enum_catalog.rs and this worklog. Coordinator owns test registration and all native database runs. No database startup, shared model/config/planner changes or index rebuild.

## Reference research and contract

RTK and Ponytail guidance reread. Current project HEAD/index revision db28c459a8a3caf45b07e9e72bcbc9fd3194c762; the working tree contains newer integrated changes, so indexed passages guide lookup while current originals determine behavior. Fresh project-my2pg queries `T15 T16 acceptance remaining persistence importer` and `verify_columns enum_labels pg_enum enumsortorder` return docs/agent-tasks61, verify/mod.rs181 and contracts/plan.json. Read docs/checklists40–59, scope enum/structure contract, testing103, current verifier215–345/415–510, columns.sql, adjacent t15_verify and t15_default_complete tests and default completion worklog. T15 must verify supported structure and refuse differences; row counts are not content proof.

Fresh ref-rust-postgres queries `pg_enum enumlabel enumsortorder` and `TYPEINFO_ENUM_QUERY enumlabel enumsortorder` return pinned1084ca8f5b5302e161892f2fa40abf71b4060c10, tokio-postgres/src/prepare.rs34–40/214–240. Originals read: enum variants are parameterized by type OID and ordered by enumsortorder. The legacy fallback ordering by OID serves PostgreSQL9.0 and is outside our server contract. Adapt that catalog observation, without copying driver cache/lifecycle machinery; use a fresh query so ALTER TYPE drift cannot hide behind cached driver types.

Current verifier proves actual column/default OIDs against the planned qualified ENUM identity but never compares ColumnPlan.enum_labels with pg_enum. ALTER TYPE ADD VALUE preserves type OID, existing row count/default and all other basic schema expectations, so verification can incorrectly return Complete. Existing same-label/wrong-type-OID test proves a different boundary; it does not cover label mutation within the same type. Expected ordered labels already exist in the concrete model; no shared extension needed.

## Smallest adaptation and independent acceptance

For ValueKind::Enum with explicit expected labels, query pg_catalog.pg_enum by the actual observed column OID, ordered by enumsortorder, and compare the complete ordered Vec exactly (including empty, punctuation and Unicode labels). Missing expected labels must return Unsupported rather than silently treating an OID as full structural evidence. Non-enum columns and existing type/default proof stay unchanged; query/decode errors propagate sanitized VerifyError. No SQL expression evaluation or arbitrary identifier interpolation.

Native case uses the existing counts_and_schema seam after the actual planner/production pipeline creates a uniquely owned source fixture and target schema/type/table. Baseline exact ordered labels/default/counts must pass. Ordinary ALTER TYPE ADD VALUE BEFORE adds a middle label while independent catalog observations prove same type OID/count/default/row. The verifier must then return Different with explicit ENUM label drift. No shared/system catalog writes; pure target reorder/removal is not claimed because PostgreSQL has no ordinary same-OID DDL for it. Root approved add-only drift as decisive regression. Include special label bytes, a reviewed order-only expectation mismatch, and missing expected metadata boundaries. Leave failed owned objects/evidence for coordinator inspection; successful test cleans only its unique source table/target schema and disconnects its guards.

Research-only T15/T16 audit: latest126-case native log10a9d7a6f734 confirms the console/default/schema/snapshot/durable and both importer cases, plus actual CLI physical ENOSPC. The physical sync failure gate is distinct and remains open. The ordered ENUM gap is independent of T10 supplementary-label recovery, T11 guarded preparation and T17 expressions. T16 still needs accepted OR/default/null/signed/auto-increment guard behavioral coverage and future capability translations. Old preparation docs/logs saying no native proof or standalone writer are historical; taskboard79–83 records the current integrated success. No full-task checklist closed.

## Executed native RED before production edit

Coordinator registered the new native module and executed the exact test on its owned pair8d0726c02f10. Production run exited0 and CountsAndSchema verification reported Complete. Ordinary ALTER TYPE ADD VALUE BEFORE then went undetected: verifier differences were[] and the native assertion expecting Different failed. This is actual behavioral RED, not an absent-environment panic or an ignored test counted as passed. The coordinator retained the unique failed fixture/evidence; this agent ran no database command. Proceeding with the catalog comparison under the approved lease.

## Implementation and GREEN evidence

The production change is22 lines in verify_columns plus a four-line catalog query. It compares exact ordered labels for the observed ENUM column OID and explicitly reports Unsupported if complete expected labels are absent. Existing ENUM OID/default proof and ordinary columns remain unchanged. Query/row-decode failures propagate existing sanitized VerifyError. No dependency, metadata model, catalog mutation, expression evaluation or driver type cache is added.

Coordinator executed identical RED/GREEN commands on its owned pair:

```sh
set -a
. target/integration/my2pg-mysql84-pg16-8d0726c02f10/env.sh
set +a
bin/cargo test --offline --locked --test integration t15_enum_catalog::live_enum_label_addition_preserves_oid_count_default_but_fails_schema_verification -- --ignored --exact --nocapture --test-threads=1
```

RED: the initial production run exited0, then added label yielded Complete/[] instead of Different. Retained report: `target/integration/my2pg-mysql84-pg16-8d0726c02f10/t15_enum_e8b6_18da9f15d4f038a0/run-18da9f16066d2e88-e8b6-0/report.json`.

GREEN after the query:1passed/0failed. Actual ALTER TYPE ADD VALUE BEFORE now returns Different while independent observation proves identical type OID, count, default and stored row. Valid original labels (including empty, comma, quote and BMP Unicode) pass; a complete matching extended expectation passes; an expectation-only order mismatch returns Different; missing expected labels returns Unsupported. Actual target label reorder/removal is not claimed. This agent did not start/run/stop any database pair; coordinator owns retained RED fixture/evidence and broader suite/cleanup.

Local checks through RTK: all-target/all-feature offline check passed; `bin/cargo clippy --offline --all-targets --all-features -- -D warnings` passed2.63s; `bin/cargo test --offline --all-features --lib verify:: -- --nocapture` passed7/7 (zero ignored) in0.01s; scoped Rust1.99 rustfmt check passed. These are checks for this increment; full T15/T16/checklist and matrix acceptance remain open.

Final four leased files frozen. SHA256 verify/mod.rs `cc80fbc12b87d561590d471f1aba694ef060c6740138ccd8b03bc3f8bb58a663`; verify/enum-labels.sql `361a4340be247104dd27e7735e54ead49459d0fd7e55dbd76a54a025a2a0ce70`; cases/t15_enum_catalog.rs `121fee0ab689908093c64fc48d7f3281acf756503502d1b38e1641e7c4ddc647`. Coordinator owns final registration/taskboard/index evidence. No shared files or pinned references changed by this lease.
