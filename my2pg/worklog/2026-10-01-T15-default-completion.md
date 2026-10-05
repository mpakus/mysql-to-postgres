# 2026-10-01 — T15 complete supported defaults

Status: implementation lease accepted, research recorded before code. Exclusive ownership: verify/defaults.rs, verify/mod.rs, verify/columns.sql (coordinator extended lease), new tests/cases/t15_default_complete.rs and this log. No shared model/Cargo/config/connection/pipeline edits. Root owns registration and final acceptance.

## Reference research

RTK/Ponytail guidance, reference-coding workflow and scope/testing contracts were reread. Live and indexed project revision: `776901a1377fbea654bdf19f7a201014af757fbc`.

- XERJ project-my2pg query `json default enum interval verifier` returned previous default preparation and plan/mod.rs at this revision. Originals read: defaults.rs comparator, mod.rs catalog integration, columns.sql, plan/mod.rs override/default resolution, convert/json.rs validation/domain checks, convert/mod.rs and adjacent transform tests, prior T15/T10 live cases. Existing supported planner targets are JSONB/INET; CIDR is rejected by current config, normalizer and planner. Root declined a CIDR scope upgrade for this increment.
- ref-paganel `json default numeric` at `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc` returned unrelated pagination; no useful equality comparator. Local original engine-core/src/plan/execution.rs:130–168 and adjacent tests show sorted object keys for plan hashing. Adapt only structural object comparison; reject textual scalar serialization as sufficient for exact numeric equivalence. No reference code copied.
- project-pgloader `bytea interval inet` at `231ab86778ca5ffd7de40878714760c8b4860cdf` returned broader PostgreSQL-source mapping in clojure/source/pgsql.clj, read at23–78; not a MySQL-only verifier. Original mysql-cast-rules.lisp:107–123 supplies byte-vector-to-bytea mappings, not output comparison. Keep MySQL-only scope; no additional source adapters.
- Installed serde_json1.0.151 value/de.rs:120–145 confirms Map insertion replaces duplicate keys and arbitrary_precision Number deserializes from its original decimal string. Existing Cargo already enables that feature. Adapt library parsing plus independent exact coefficient/exponent normalization, not f64 conversion or exponent expansion. JSON storage keeps text/duplicates/order; JSONB uses last-key-wins and structural numeric equality.

Primary sources: [JSON storage/JSONB semantics](https://www.postgresql.org/docs/16/datatype-json.html), [network input](https://www.postgresql.org/docs/16/datatype-net-types.html), [interval styles](https://www.postgresql.org/docs/16/datatype-datetime.html), [bytea hex/escape](https://www.postgresql.org/docs/16/datatype-binary.html). No SQL evaluator will be used.

## API and actual deparse proof before changes

Coordinator approved keeping compare(ColumnPlan,actual,DeparseSettings) unchanged and adding compare_with_catalog with concrete DefaultCatalogContext: actual IntervalStyle, target-type proof and enum OID proof (expected type, actual column type, actual finite cast type). Only fully parsed finite identifier tokens reach parameterized pg_catalog.to_regtype. Actual atttypid is added to the existing columns catalog query. A same-label/wrong-OID enum differs; an unknown expression remains Unsupported.

Fresh owned fixture `63e4d9d619c6` started for this task. Fixture-only known-literal DDL created t15_probe.defaults, then pg_get_expr inspected catalog metadata without executing defaults:

| Setting | Negative clock default rendering |
| --- | --- |
| postgres | `'-838:59:58.123456'::interval(6)` |
| postgres_verbose | `'@ 838 hours 59 mins 58.123456 secs ago'::interval(6)` |
| sql_standard | `'-838:59:58.123456'::interval(6)` |
| iso_8601 | `'PT-838H-59M-58.123456S'::interval(6)` |

Positive12:34:56.123456 uses corresponding positive components. bytea_output=escape renders known bytes00095c7fff as `'\000\011\\\177\377'::bytea`. CURRENT_TIMESTAMP(0), omitted precision on timestamp(6), and explicit precision6 remain distinct deparse tokens. Comparator will recognize those output grammars with checked exact microseconds; calendar months are never guessed as fixed durations. Zero-clock spellings in each style and actual mutation cases still need native tests.

CURRENT_TIMESTAMP omitted precision is6 at PostgreSQL timestamp resolution. Omitted/explicit6 match. Omitted/explicit target precision can match only with actual column typmod proof and one final rounding step; do not blindly use min(expression_precision,column_precision), because intermediate rounding can change the final result.

Implementation inventory: preserve already proved core scalar/temporal/BIT/SET defaults; add exact JSONB object/number normalization and JSON storage text, INET address/prefix normalization, catalog-proved unqualified enums, bytea escape output and all four interval output styles generated from supported MySQL clock defaults. Native planner/pipeline cases and target mutations/settings must prove each; unknown SQL remains unsupported. No completion claim yet.

Before JSON implementation, deeper installed Value::visit_map/KeyClassifier and NumberDeserializer inspection found a reserved-key collision: a real object key `$serde_json::private::Number` can be treated as an arbitrary_precision number token. Chosen adaptation: a small bounded JSON value reader with quoted strings decoded by serde_json::from_str::<String>, exact numeric lexemes normalized to coefficient/exponent, BTreeMap last-wins objects and ordered arrays. No f64, synthetic reserved key, exponent expansion, unsafe library-type discrimination or extra dependency. Independent reserved-key object-vs-number tests will prove the boundary. JSON text is validated and compared as decoded stored text, retaining key order/duplicates/whitespace.

Native known-INET-literal fixture probes accepted001.002.003.004 and IPv4-mapped IPv6, rejected abbreviated192.168/24 and127.1. Stdlib IP parsing rejects leading-zero IPv4 despite PostgreSQL decimal input accepting it. Adaptation before fix: finite exactly-four-decimal-octet fallback (no octal/abbreviated/CIDR inference) and equivalent embedded IPv4 normalization for IPv6. Independent native default case will include leading-zero IPv4. Initial complete-default unit run passed3/5 because2older tests still expected Unsupported for JSONB/octet syntax now supported; updated those assertions and all5pass. No native complete-case success claimed yet.

First native complete-default run failed an independent default-insert assertion after pipeline exit0: source BINARY(5) DEFAULT0x00095c7fff inserts00095C7FFF, but HEX(information_schema.COLUMN_DEFAULT) is3078 (ASCII0x only). Persisted plan incorrectly emits E'\\x0000000000' and target inserts0000000000. Exact reproducer: CREATE TABLE source.t15_complete(id INT PRIMARY KEY,bytes_value BINARY(5) DEFAULT 0x00095c7fff); INSERT with id only; inspect HEX(bytes_value) and HEX(COLUMN_DEFAULT). Existing source metadata/planner issue is outside this lease; root assigned D's new t10_binary_default regression and explicitly refused full fidelity acceptance pending its fix. Failure session99194 and task-owned report are preserved as evidence. Following root direction, the planner/pipeline test uses printable BINARY default'a' (6100000000), while a separate known typed literal plus actual catalog settings proves all bytea escape forms. This does not silently remove the original source-fidelity gate.

Second native run4031 passed all8 interval/bytea rendering combinations, then correctly detected different payload. Independent source probe shows supplementary Unicode metadata loss: utf8mb4 VARCHAR DEFAULT JSON with literal emoji stores source bytes F09F9880 when inserted, but information_schema.COLUMN_DEFAULT contains3F ('?'). The plan faithfully copies that already-lossy metadata; target inserted default differs from source semantics. Root notified for metadata recovery alongside binary gap. Exact failed fixture and report remain in owned63e4d9d619c6 artifacts. Before changing the fixture, compared actual stored row HEX, metadata HEX and target pg_get_expr. Adaptation for verifier-only proof: source JSON string now uses ASCII surrogate escapes, whose JSONB decoded semantic string must equal literal supplementary Unicode in target mutation. This proves JSON parser equality without claiming source metadata fidelity is fixed.

## Required-default inventory and actual proof

| Supported contract | Evidence and boundary |
| --- | --- |
| Signed/unsigned integers and exact DECIMAL | Existing t15_defaults native pipeline case, extreme unsigned and60-digit decimal; mutations differ. Finite exact numeric grammar, no tolerance. |
| FLOAT/DOUBLE | Existing native float and max-double deparse proof; extra_float_digits boundary must bepositive. NaN/infinity/underflow-tozero remain unsupported. |
| Boolean, empty and escaped text | Existing native pipeline/default mutation cases; complete SQL token consumption. |
| DATE/DATETIME/UTC TIMESTAMP | Existing native leap-day/fraction/UTC cases and mutations; target session enforces UTC/ISO. |
| CURRENT_TIMESTAMP | New actual timestamp0/6 target type proof accepts omitted/explicit6 and final targetprecision equivalence; precision3-to0 remains Different to avoid double-rounding inference. |
| MySQL TIME clock to interval | New actual negative838-hour, positive andzero clocks under all4 IntervalStyles; exact bounded microseconds. Calendar months/day reinterpretations are outside supported clock planner domain. |
| BIT | Existing native fixed width and mutations; complete bit grammar and width proof. |
| Bytea | Printable BINARY native pipeline proof; new typed known bytes00095c7fff under hex andescape output including NUL/tab/backslash/DEL/255. Upstream binary metadata defect remains unresolved. |
| ENUM | Actual native qualified/unqualified deparse, search_path shadowing, same-label/wrong-columnOID mutation. Fully parsed identifier only, parameterized to_regtype catalog resolution; no label-only acceptance. |
| SET/text array | Existing native memberships/empty/quoted NULL and mutation/domain tests; shared SET membership helper preserved. |
| JSONB override | New native planner/pipeline/deparse: exact arbitrary decimal, exponents1e131071/1e-16383, duplicate-key last-wins, key-order independence, ordered/multiplicity-preserving arrays, missing vsNULL, decoded supplementary Unicode, reserved-key object. Mutation comparisons distinguish all required semantics. |
| JSON text | New actual catalog plan proves decoded stored text retains duplicatekeys/order/whitespace; semantically equivalent JSONB-form text differs. This is comparator coverage, not a config target-scope expansion (configured target remains JSONB). |
| INET override | New native IPv6 and leading-zero decimal IPv4 address/prefix normalization; hostbit/prefix mutation differs. Family retained. CIDR remains rejected by current config/planner, coordinator declined new scope. |
| Unknown expression | Existing native nextval side-effect sentinel proves Unsupported without consuming sequence even for exact repeated expression text. No arbitrary SQL evaluation or exact-string bypass. |

Public API compatibility: existing crate-visible compare(ColumnPlan,actual,DeparseSettings) remains unchanged. Added compare_with_catalog with concrete DefaultCatalogContext (IntervalStyle,target_type_proven,EnumIdentity expected_oid/column_oid/cast_oid). Catalog query now returns actual atttypid. Type resolution accepts a complete one/two-component SQL identifier; every catalog query parameterizes it. Settings metadata errors propagate sanitized VerifyError::Metadata. Common normalize_postgres_type reused; shared SET/recovery interfaces unchanged.

JSON parser matches the existing serde JSON decoder127-container boundary, rejects malformed surrogate/NUL strings andkeys, enforces existing PostgreSQL numeric domain, and never expands exponents. Reader allocations are bounded by the supplied catalog literal's size/depth; this is verifier metadata processing, not a new row encoder memory contract.

## Validation and completion limits

- Unit red/green: initial3/5 passed (two old Unsupported assertions became valid coverage); adjusted those expectations. Final verify::defaults5/5 passed, including independent duplicate/missing/array/number/string/depth boundaries.
- Fresh owned native case passed in session34863 after preserving both diagnosed source-fidelity failures. Full T15 focused suite session49425 passed12/12 on MySQL8.4.11/PostgreSQL16.15, including existing no-evaluation, session/float, schema/count/snapshot, terminal/JSON/accounting cases and new complete-default proof.
- All-target strict Clippy session39040 was blocked only by concurrent owner's new t10_binary_default.rs metadata tuple type_complexity; owner notified. No owned verifier warning reported. Owned files rustfmt applied. Coordinator owns final shared-tree checks and XERJ rebuild.

Full default fidelity is NOT accepted: MySQL information_schema default extraction loses embedded-NUL binary data and supplementary Unicode. Those independently reproduced source observations require source/planner recovery or explicit planning rejection; passing typed comparator against a lossy plan cannot prove source fidelity. Root/D own the fixes and new independent t10_binary_default regression. Unknown SQL/functions, CIDR and calendar-month interval literals remain outside supported planner contract, not silently accepted defaults. No additional comparator coverage blocker within the supported finite inventory is known after native12/12 proof.

Final handoff: source/tests frozen. Owner repaired the unrelated Clippy warning; fresh all-target strict Clippy session70358 now passed, lib-only strict Clippy passed, owned diff --check clean. Fixture63e4d9d619c6 stopped successfully (connections.json state=stopped). Failure evidence retained in private t15_complete/run-18da8989a66a6d58-7fce-0 and run-18da89a988b45740-80d2-0 plan/report files; successful native-case artifacts were cleaned by their owned tests. Coordinator may refresh shared index after other owners freeze.
