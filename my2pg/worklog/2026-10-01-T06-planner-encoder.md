# 2026-10-01 — T06 — pure planner and COPY row encoder

- Status: review
- Agent/role: D (fixtures_harness reassigned by coordinator after T02)
- Dependencies: T01 shared contracts evolve under coordinator review; T02 fixtures handed off with passing independent harness checks. C/T03 cancellation repair independently passed the fresh nine-test suite before T06 development.
- Claimed files: `src/plan/`, `src/convert/`, task-specific `tests/cases/t06*`, this log. Coordinator owns module wiring, Cargo/model/config changes.
- Instructions read: root AGENTS.md, Ponytail SKILL.md, RTK.md, reference coding, architecture/config/type/default/collation/target policies and T06/T07/T10/T11 ownership. RTK wrappers in use; no separate RTK SKILL.md supplied.

## Reference research before coding

| Query/prefix | Pin and original source/tests | Findings and adaptation |
| --- | --- | --- |
| `collisions target existing qualification ColumnPlan`, project-my2pg | current indexed spec/model and scope-and-compatibility.md:61–95; architecture planning section | Planner preserves source identity, uses exact target schema, emits diagnostics before any mutation; unsigned widening and interval durations are required. Plan is pure and diagnostic errors prevent runnable output. |
| `BigInt Unsigned numeric`, ref-paganel | `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc`, crates/engine-schema/src/converters/to_postgres.rs:27–82; adjacent engine-tests/schema/objects.rs and verify_objects.rs | Reference narrows unsigned64 with warning, serializes geometry into bytea and maps FULLTEXT toGIN. Reject those defaults; independently implement numeric(20,0), optional spatial blocking and explicit unsupported-index diagnostics. Reference FK tests show separate create/copy/addFK phases and exercise actual rows. No AGPL code adapted/copied. |
| `format-copy-column bytea` and `copy encode bytea escape`, project-pgloader | `231ab86778ca5ffd7de40878714760c8b4860cdf`, clojure/src/pgloader/copy.clj and adjacent copy_test.clj; src/pg-copy/copy-format.lisp | Escape COPY control bytes once after type serialization; reference silently pads missing row fields and ignores extras. Reject both arity mismatches. T02 found actual v4 BLOB corruption despite green count; our test expects exact bytea hex and literalNULL/empty/\\N distinctions. |
| `quote_identifier postgres`, ref-dmt-rs | `4e8015f7e841dbdf9df01e953aeb3948dfb3199a` | Broad lexical search returned general registry/docs, no useful quote helper. Use independent PostgreSQL double-quote escaping, UTF8byte-aware name allocation and deterministic SHA-256 source identity. |

## Intended acceptance

Basic synthetic fixture planning resolves integer/unsigned/decimal/text/binary/date/datetime/timestamp/time/JSON/BIT mappings, qualified table/PK/identity SQL and safe naming. Invalid/unsupported structures and target conflicts produce blocking diagnostics. Pure encoder preserves exact decimals/u64/binary and COPY escaping; malformed/NULLcontradictory rows reject explicitly. Later M2/M3 fidelity remains separate acceptance work and will not be claimed from unit snapshots alone.

API proposal sent to coordinator: `plan::build(&MigrationConfig,&SourceCatalog,&TargetCatalog)->Result<MigrationPlan,PlanError>`; `convert::encode_row(&TablePlan,&[RawValue])->Result<Vec<u8>,ConversionError>`; row inputs contain exactly COPY-selected columns. Config field and root module additions remain coordinator-owned.

Coordinator approved `encode_value(&ColumnPlan,&RawValue)->Result<Option<String>,ConversionError>` as the shared unescaped PostgreSQL text representation. Only `encode_row` applies COPY escaping. Application pipeline checks the final encoded row length; the source adapter bounds decoded raw row size. SQL NULL, empty text/bytea and a literal backslash-N remain distinct.

### Privilege contract increment before implementation

Searched current `project-my2pg` for `can_alter can_truncate privileges`: architecture.md:61–95 at indexed revision6e9d7a4 requires complete catalog/privilege preflight before mutation. Read original architecture and new shared TargetTable fields in model.rs. `ref-rust-postgres` query `has_table_privilege` at pin from reference manifest returned no matching source/test; privilege metadata is a project-specific catalog contract. Adaptation: deny missing/false INSERT/SELECT/ownership/TRUNCATE metadata when required by the selected operation; row security blocks row copy/verification until explicit support. Existing table privileges cannot establish constraint compatibility. Pure negative tests and real fixture migration will verify this increment.

## Implementation and interim verification

- Pure planner preserves original MySQL database/table/column identity, resolves target names and byte-limited deterministic identifiers, checks collisions and privileges, applies first-match casts, and emits qualified schema/table/PK/index/FK/identity-reset SQL. Table and column comments are preserved. Typed schema expectations are produced alongside the matching DDL from catalog metadata, never by parsing generated SQL.
- Signed/unsigned widening, exact decimal, UTF8 text, bytea, date, DATETIME wall time, TIMESTAMP UTC, TIME duration, BIT and JSON conversion are implemented. Unsupported transforms/types/objects/default expressions block planning or conversion explicitly. Routine/event/trigger inventory remains reported as manual work. Ordinary text collations warn; uniqueness requires an explicit collation policy acknowledgment.
- Existing-table privilege and RLS metadata fails closed. Destructive policies reject dependencies outside the selected set, including same-schema dependents. Occupied view/sequence/index names block creation. Cross-schema renames currently block because the target catalog inspects one schema. Existing generated-index occupancy during recreate conservatively blocks until the catalog can establish the index's owning table; this is a known target-fidelity limitation.
- Numeric/text/JSON default literals use the same typed converter to reject narrowed ranges, malformed JSON, NUL and typemod violations before mutation. Complex temporal/default/enum/set/spatial/generated/check/index-expression fidelity remains T10/T11/T17 work. No M2 completion is claimed.
- `bin/cargo test --locked --lib plan::`: initial9 tests passed after privilege checks. Additional typed-structure/comment/collision/dependency/default/cast tests are being compiled with the new shared contracts.
- Mandatory harness run `57019b2e3663` failed compilation on an ambiguous empty-vector assertion; corrected owned assertion to `is_empty()`. Containers were cleaned.
- Mandatory harness run `58865545f9c1` failed compilation when coordinator introduced `TablePlan.structure` and `MigrationPlan.on_existing/reset_sequences` during the run; owned literals updated. Containers were cleaned.
- Mandatory harness run `9983367a9783` failed compilation when coordinator introduced `ColumnPlan.comment` and target dependency/name metadata during the run. Owned literals updated; C target initializer still pending at this snapshot. Containers were cleaned. These failed runs do not constitute database acceptance.

Latest stable checks: planner14/14, encoder6/6, `bin/cargo clippy --locked --all-targets -- -D warnings` passed; `bin/cargo test --locked --test integration --no-run` passed. New checks preserve time-zone suffixes when dropping typemods, normalize MySQL integer/numeric aliases, and ignore unsupported storage/index metadata belonging to unselected tables (source ownership read in mysql/mod.rs). Binary collations still require uniqueness-policy review because source PAD SPACE behavior is not established by the `_bin` suffix.

### Mandatory fresh runtime acceptance

After E released the repaired COPY gate and coordinator froze the shared model, ran `rtk run './my2pg/tests/run-integration.sh mysql84 pg16'`: owned run `my2pg-mysql84-pg16-6b80a6baa497` passed all12 seed assertions and all20 discovered database tests (11 driver/source;9 integration including T06, five T07, two T15 verifier and TLS smoke), then scoped cleanup completed; connections.json state is `stopped`. The caught intentional source-reader panic printed its hook but its test passed; it is not a skipped failure.

`t06_plan::pure_plan_and_text_encoder_execute_real_relational_fixture` reads actual source and target catalogs, produces a pure plan, executes seven-table qualified DDL, streams actual binary-protocol rows through the shared encoder into PostgreSQL COPY transactions, finalizes indexes/FKs and identities, then independently checks exact DECIMAL/u64/JSON/BIT/TIME/BLOB/all256bytes/emoji/control text, NULL vs empty values, keyless duplicate multiplicity, empty-table default/identity1, source users next-value lower bound42, separate PK/UNIQUE/FK violations, and the public sentinel. This establishes the basic T06 fixture gate, not CLI orchestration/T09 or complete T10/T11 fidelity.

Evidence: `target/integration/my2pg-mysql84-pg16-6b80a6baa497/{rust-tests.json,rust-tests.log,seed-checks.json,connections.json}` retains case names in raw output, working revision/dirty flag, exact lock/fixture/harness/log digests and cleanup state. It does not currently hash each executed test binary or snapshot the entire Rust source tree. Earlier failed runs remain intact and separately described above.

## T06 checklist

- [x] Reference queries, original implementations, adjacent tests and adaptation recorded.
- [x] Pure `build` and immutable source-to-target mapping API supplied.
- [x] Qualified deterministic table, key, index, FK and identity DDL.
- [x] Collision, existing-object, privilege, row-security and outside-dependency diagnostics.
- [x] Shared pure value representation and single COPY escaping pass.
- [x] Typed schema expectations and comments align with generated object names.
- [x] Owned unit tests, integration compilation and strict Clippy pass.
- [x] Mandatory fresh real fixture migration and all discovered database tests pass together.
- [ ] Coordinator review and task-board handoff.
