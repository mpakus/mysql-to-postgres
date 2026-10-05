# 2026-10-01 — T15 default verifier preparation

Status: finite verifier increment implemented and focused checks passed; coordinator acceptance remains pending. Original preparation authorized read-only source/tests and this log. The renewed implementation lease and actual changes/evidence are recorded below. RTK and Ponytail guidance were used; no shared model, dependency, transport or index changes.

## Reference research before implementation

The project/reference workflow, scope, architecture verification contract, testing-and-performance requirements and existing T10/T15 worklogs were read. Initial preparation used live project HEAD and indexed revision `66c92db30cc1ba0a3ecdd9c0ac3ae4fe253f270f`; renewed implementation revision is recorded below.

| Prefix/query | Revision and originals read | Finding and adaptation |
| --- | --- | --- |
| `project-my2pg`, `default pg_get_expr temporal cast`, `-k 3` | HEAD above; `src/plan/mod.rs:242–417`, `src/convert/defaults.rs`, `src/convert/mod.rs:700–810`, `src/verify/mod.rs:186–399`, `src/verify/columns.sql`; adjacent `tests/cases/t10_values.rs:1–353`, `tests/cases/t15_verify.rs:110–210`, verifier literal unit cases | Planner emits typed literals and complete CURRENT_TIMESTAMP tokens; verifier recognizes only text/bool/fixed numeric literals. Reuse resolved ColumnPlan and existing finite type normalization. Do not re-infer source mappings. |
| `ref-paganel`, `default expression compare postgres`, `-k 2` | `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`; `crates/engine-schema/src/type_registry.rs:150–255`, tests `366–399` | Cast stripping preserves quoted `::` but also strips inner `regclass` in nextval. Useful lexical boundary cases; reject general cast erasure because target identity, typmods and function semantics matter. No code copied. |
| `project-pgloader`, `default current_timestamp bytea`, `-k 2` | `231ab86778ca5ffd7de40878714760c8b4860cdf`; `src/sources/mysql/mysql-cast-rules.lisp:235–267`, nearby MySQL fixtures returned by search | Prefix normalization collapses CURRENT_TIMESTAMP precision. Reject prefix recognition; accept only the complete supported token with optional precision 0–6. No reference tests executed. |

The first local search also named nonexistent `src/convert/temporal.rs`; `rg` returned exit 2. Temporal logic actually lives in `convert/defaults.rs` and `convert/mod.rs`, subsequently read. An initial documentation lookup named nonexistent `testing-and-acceptance.md`; the actual `testing-and-performance.md` was located and read. These failed lookups are not evidence of missing implementation.

Primary PostgreSQL documentation/source was consulted separately from the pinned clone inventory:

- [Catalog expression deparse](https://www.postgresql.org/docs/16/functions-info.html): pg_get_expr reconstructs an expression rather than returning original DDL.
- [REL_16_STABLE ruleutils.c](https://raw.githubusercontent.com/postgres/postgres/REL_16_STABLE/src/backend/utils/adt/ruleutils.c), get_const_expr and SQLValueFunction cases: type output and casts determine literal spelling; CURRENT_TIMESTAMP precision survives deparse. This moving official branch was inspected read-only, not added as a new pinned reference.
- [Datetime input/output](https://www.postgresql.org/docs/16/datatype-datetime.html), [bytea formats](https://www.postgresql.org/docs/16/datatype-binary.html), [array syntax](https://www.postgresql.org/docs/16/arrays.html), [SQL string syntax](https://www.postgresql.org/docs/16/sql-syntax-lexical.html), [client settings](https://www.postgresql.org/docs/16/runtime-config-client.html): distinguish SQL escape decoding from bytea/array decoding; timezone, IntervalStyle, bytea_output and extra_float_digits can affect output. Float shortest-precise output requires extra_float_digits >= 1; do not assume a role/database override retains the server default.

## Current behavior and gaps

T10 default_sql supports signed/unsigned/decimal/finite float, text/JSON, date/datetime/timestamp/duration, boolean/BIT, binary, enum and SET literals. It accepts CURRENT_TIMESTAMP and CURRENT_TIMESTAMP(p) only for datetime/timestamp. Other expressions fail planning. The converter supplies exact fractions, UTC +00 timestamps, full BIT width, padded BINARY bytes and escaped one-dimensional text arrays.

The verifier currently treats byte-for-byte matching SQL as complete before applying its finite parser. Thus equal unknown SQL can bypass Unsupported. Unequal recognized text/bool/fixed decimal forms compare semantically; temporal, bytea, bit, enum, array, float and JSON forms otherwise remain unsupported. Its one-pair parentheses stripping is not a complete balanced-wrapper check.

T10's live test helper performs DDL/COPY/default inserts directly and never calls counts_and_schema. It proves conversion/default behavior, not verifier coverage. Existing T15 live default cases cover empty varchar, escaped text, decimal, bool, integer, changed values and random() Unsupported. They do not cover the missing type families.

## Read-only PostgreSQL observations

Earlier successful catalog SELECT on the coordinator-owned MySQL8.4/PostgreSQL16 fixture `1d68940b1228` observed these existing T10 target defaults. No data/schema changes were made for research:

| Type | pg_get_expr output |
| --- | --- |
| text[] | `'{label0,label63}'::text[]` |
| generated schema-qualified enum | `''::t10_values_acceptance.enum_t10_choices_state_f01a93cc` |
| bit(64) | `'0000000000000000000000000000000000000000000000000000000000000101'::"bit"` |
| bytea | `'\x61000000'::bytea` |
| date | `'2024-02-29'::date` |
| timestamp(6) without time zone | `'2024-02-29 01:02:03.123456'::timestamp without time zone` |
| timestamp(6) with time zone | `'2024-02-29 01:02:03.123456+00'::timestamp with time zone` |
| interval(6) | `'-838:59:59'::interval(6)` |
| varchar(3) | `'€ÿ'::character varying` |
| smallint | `0` and `2` |

Float defaults and CURRENT_TIMESTAMP were not observed in this catalog probe. The checked-in T10 test asserts float default inserts, but that is not fresh deparse evidence. A first SELECT failed because nested shell quoting removed SQL quotes around namespace names; a numeric namespace filter then succeeded. Latest attempt to read SHOW IntervalStyle/extra_float_digits/bytea_output/TimeZone/DateStyle and float catalog defaults failed at the first SHOW because the coordinator fixture's PostgreSQL service had stopped. No restart was attempted. New isolated live cases must supply this missing evidence after implementation authorization.

## Smallest safe comparator proposal

Add private `src/verify/defaults.rs`; move the existing literal helpers there without expanding the public verifier API. Suggested internal contract:

```rust
pub(super) enum DefaultComparison { Equal, Different, Unsupported }
pub(super) struct DeparseSettings { pub exact_float_output: bool }
pub(super) fn compare(
    column: &ColumnPlan,
    actual: &str,
    settings: &DeparseSettings,
) -> DefaultComparison;
```

ColumnPlan.default_sql supplies the expected expression, target_type/kind supplies the resolved domain, and enum_labels/set_labels supplies membership expectations. No shared model/Cargo change needed. At verification start, read `current_setting('extra_float_digits')` once through the existing target connection; metadata-query errors propagate as execution errors. Refuse float equivalence when it is below 1. Existing initialization already fixes standard_conforming_strings=on, timezone=UTC and DateStyle=ISO,YMD. Recognize only the finite interval/bytea output forms below; other styles remain Unsupported rather than guessing their meaning.

Parse complete tokens: one SQL literal, optionally one recognized cast, optionally balanced whole-expression wrappers. Decode SQL quotes/E escapes once, then the target-specific literal layer. Reject trailing SQL, comments, operators, chained casts, dollar quotes, arbitrary calls and unrecognized escapes. Casts must match the resolved built-in target base or exact modifier; do not erase narrowing modifiers. Reuse types::normalize_postgres_type; explicitly recognize PostgreSQL's quoted builtin `"bit"` only in the bit branch. Never trim or case-fold data.

| Family | Finite comparison | Boundary |
| --- | --- | --- |
| Existing text/bool/integer/numeric | Preserve exact text and bool; exact signed decimal canonicalization without float conversion | No char padding/truncation, chained casts, arbitrary arithmetic or exponent expansion for numeric |
| real/double precision | Parse finite decimal/exponent into the resolved f32/f64 and compare IEEE bits, with exact-output setting proof | No NaN/infinity or tolerance; preserve signed zero; float live deparse needed before enabling unfamiliar cast forms |
| Date/datetime/UTC timestamp | Parse calendar components, clock and <=6 fractional digits; normalize trailing fractional zeros; timestamp accepts only proved UTC suffix forms | Reject BC/infinity/timezone names/nonzero offsets; no SQL evaluation or general date arithmetic |
| MySQL TIME mapped to interval/time | Parse bounded signed clock form to checked microseconds, including >24-hour duration and optional fraction | Months/days/mixed signs/other interval output styles Unsupported until independently proved |
| bytea | Decode SQL string once, then even-length `\x` hexadecimal into exact bytes | Escape/octal bytea output initially Unsupported; no dropping NUL/padding |
| bit(n) | B'...' versus typed quoted bit string; exact width and bits | No automatic padding/narrowing or varbit coercion |
| enum | Exact decoded label and exact qualified target enum cast identity, with declared-label validation | Unqualified casts/search-path-dependent identity remain Unsupported; catalog-assisted resolution is a separate reviewed extension |
| SET/text[] | Parse finite 1D array text after SQL decoding; preserve order, duplicates, NULL distinction and escaped quoted members | Reject dimensions/non-1 lower bounds, nested arrays, unknown labels and SQL ARRAY constructors until covered; never reuse the membership predicate's BTreeSet as the default-value comparator |
| CURRENT_TIMESTAMP | Only full token and optional precision0–6; compare matching precision/token | now(), clock_timestamp(), LOCALTIMESTAMP and omitted-vs-explicit precision equivalence remain Unsupported unless a separate proof is accepted |

Remove the exact-string bypass for unrecognized expressions: equal unknown SQL still reports Unsupported. Same supported literal/token passes through the same comparator. Recognized unequal values return Different. Unsupported syntax never becomes a successful verification merely because strings match. Default presence and generated/identity handling retain their existing separate contracts.

JSON/jsonb and inet/custom override literals remain explicit Unsupported in this increment. JSON needs an exact arbitrary-precision semantic comparator with duplicate-key/jsonb normalization policy; a plain serde Value equality or float parser would not prove that. This is a reported T15 completion gap, not a silent pass. If the coordinator requires every T10-supported override default in this lease, expand that scope explicitly before implementation.

## Exact requested lease and proof

- Production: `src/verify/mod.rs` for comparator integration/settings lookup and `src/verify/defaults.rs` for the private finite comparator. Existing SET predicate comparison stays in place; quoted string helper can be exposed privately from defaults to avoid duplication. No changes to connection/session APIs, columns.sql, pipeline, shared model or Cargo.
- Tests: new `tests/cases/t15_defaults.rs`, plus unit cases adjacent to the new private comparator. Coordinator registers the integration module. Existing T15 tests remain untouched.
- Worklog: this preparation log and a new implementation/evidence section in it.

Independent tests must use the real planner/source metadata, generated DDL and production counts_and_schema/pipeline, not hand-normalize expected catalog strings. A new owned source table supplies literal temporal, bit, binary, enum/SET, numeric/text and float defaults plus CURRENT_TIMESTAMP(0)/(6); run through production with zero or known rows and assert Complete only for covered structures. Independently inspect native PostgreSQL catalog spelling and default insert results. Mutate one default at a time while preserving row counts and assert Different. Add tricky quote/backslash/comma/brace/NULL/empty members, high-precision decimal, binary NUL/padding, negative/large intervals and fractions, float bit boundaries/signed zero, narrowing/chained/wrong enum casts, unknown identical functions and trailing SQL. Unsupported JSON/inet/array dimensions must remain explicit.

Prove no evaluator: install an owned side-effect default function backed by an owned sequence, set it as a target default without inserting a row, verify Unsupported, then inspect sequence state unchanged. Comparator code makes no query from expression text. Test extra_float_digits=0 via a scoped native test setting and require Unsupported; production setting lookup must remain fallible. Dispose only owned schemas/tables/workspace after cases; coordinator owns fixture lifecycle and full-suite checkpoint.

No implementation or verification-completion claim. Frozen pending coordinator review of the finite coverage and exact lease.

## Renewed implementation lease — evidence before coding

Coordinator renewed exactly verify/mod.rs, new verify/defaults.rs, new tests/cases/t15_defaults.rs and this worklog after tested code checkpoint45cfca4 and documentation HEAD85b238bbe5b0b69154b923f73ab241fde8cdc696. Refreshed XERJ project has173files/564passages and14 coordinator source checks. Repeated project query `default pg_get_expr temporal cast` returned this preparation and T10 evidence at85b238b; repeated ref-paganel query `default expression compare postgres` returned type_registry.rs:61 at unchanged ced0a3c. Current originals/adjacent verifier, T10 and pipeline tests were read again. Adaptation and boundaries above remain selected; unknown exact-string bypass will be removed. Common finite type normalizer and production connection settings remain authoritative. No shared model/dependency changes, no copied reference code, no arbitrary SQL evaluation.

Implementation will first run independent pure comparator expectations red, then implement/integrate and run focused offline/live checks. The new fixture is owned by this task; old coordinator pairs stay stopped. JSON/inet/unqualified enum and unsupported rendering styles remain explicit completion gaps, not a reduced T15 acceptance contract.

Intentional red: initial independent scalar/mismatch cases returned Unsupported against the placeholder comparator; both tests failed (0/2, cargo session30661). Implemented finite comparison and removed exact-string bypass; focused verifier cases passed4/4, then the additional lexical/domain boundary suite passed3/3 comparator cases. An edit typo left an extra closing parenthesis during integration; compile caught it and it was corrected before tests. Strict Clippy initially identified the old normalized_type wrapper as unused after integration; removed it and its adjacent alias test now uses the common normalizer directly. Subsequent strict all-target Clippy passed. Root was notified that new t15_defaults integration case file exists and registration is pending.

Task-owned fixture37b86ea7243c started successfully for live checks (harness session59335). Old root pairs remain stopped. No fresh live-verifier pass claimed yet.

First live run passed the side-effect case but MySQL rejected the fixture TIME default -838:59:59.123456 (1067); corrected the synthetic value to -838:59:58.123456, inside its fractional bound. Second live run again passed side-effect proof; production migration copied1row but honestly returned Unsupported for double_value. Read-only native catalog plus persisted plan show f64MAX is emitted as a309-digit bare decimal and deparsed as a quoted `::numeric` constant, with the final numeric-to-float assignment cast implicit. Official [numeric.c](https://raw.githubusercontent.com/postgres/postgres/REL_16_STABLE/src/backend/utils/adt/numeric.c) numeric_float8/numeric_float4 cases convert numeric_out through float8in/float4in. Adaptation before correction: permit only the complete unmodified builtin numeric cast in resolved Float context; no numeric typmods/chained casts/functions. Parse finite target IEEE value, reject nonzero literals that underflow to zero. Native default insert assertions will independently prove this f64MAX path. This is a narrowly researched core spelling, not general cast stripping.

## Implemented increment and final evidence

Private module verify/defaults.rs exposes crate-visible DefaultComparison, DeparseSettings and compare(ColumnPlan,actual,settings) for future T11 reuse. Existing quoted_literal token helper moved there and is reused by the unchanged shared SET membership predicate API. verify_columns uses the finite comparator for every present default; unknown identical strings no longer pass. The verifier reads extra_float_digits metadata once and propagates query failure. No SQL is built from expression text or executed to compare values.

Implemented finite literals: text/bool/exact signed/unsigned numeric, finite f32/f64 including the independently observed numeric-cast spelling, positive calendar dates, datetime/UTC timestamp fractions, clock durations, bytea hex, fixed-width bit strings, qualified declared enum labels, one-dimensional ordered text arrays and complete CURRENT_TIMESTAMP precision tokens. Numeric casts cannot retain negative zero; comparison reflects positive zero. Nonzero float literals that round to zero, narrowing/chained casts, unsupported escapes/styles, arrays with dimensions and unqualified enum identities remain Unsupported. Expected SET defaults must use declared labels; actual NULL/membership/order differences remain observable.

Three pure comparator suites cover supported spelling pairs, changed values, malicious/unknown identical SQL, balanced wrappers, escaped data, temporal domain/precision, f32 boundary rounding, signed zero, numeric typmods/underflow, bytea case/padding, arrays and type identities. Existing shared SET/type tests are preserved. Final `bin/cargo test --locked --lib verify::` passed5/5 (session46325). Strict `bin/cargo clippy --locked --all-targets -- -D warnings` passed (session76528); owned formatting and diff checks are clean.

Coordinator registered tests/cases/t15_defaults.rs. After the recorded failures were corrected, focused new native tests passed2/2 in0.73s (session52703). Final source guards were then exercised with all existing T15 verifier/terminal cases:

    rtk run 'set -a; . my2pg/target/integration/my2pg-mysql84-pg16-37b86ea7243c/env.sh; set +a; my2pg/bin/cargo test --locked --test integration t15_ -- --include-ignored --nocapture'

Result:11passed,0failed,0ignored in1.53s (session86828). This includes2new defaults cases,4existing verifier cases and5terminal/process cases. The new actual pipeline migrated1row/608acknowledged COPY-text bytes with18defaults and persisted Complete verification/exit0. Native catalog and independent native default inserts prove high-precision65,30 decimal, u64MAX, f32/f64 values including f64MAX, binary padding, bit width, enum label, SET stringNULL and fractional negative duration. All18individual default mutations preserved row counts and returned Different; restores returned the plan's original expressions. extra_float_digits0 returned Unsupported, restored1 returned Complete. A VOLATILE function default deliberately identical in plan/catalog returned Unsupported with its backing sequence still uncalled before/after verification. No arbitrary-expression evaluator was introduced.

Fixture 37b86ea7243c was stopped successfully after focused checks (harness stop session98711); both task-owned artifact subtrees, including the failed run, were removed. Native cases cleaned their owned schemas/tables on success. Coordinator owns fresh whole-tree runtime acceptance and index refresh. No shared model/Cargo/config/transport/pipeline changes. Source/tests frozen for coordinator checkpoint.

T15 completion remains open for JSON/jsonb/inet override default equivalence, unqualified enum identity/search-path proof, other interval/bytea rendering styles and remaining integrated task dependencies. Counts/schema success is not full-content equality. This increment does not reduce those acceptance requirements or claim cross-version/platform coverage.
