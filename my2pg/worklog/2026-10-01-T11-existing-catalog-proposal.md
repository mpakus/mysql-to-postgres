# 2026-10-01 — T11 — observed target catalog proposal

Status: preparation only, awaiting coordinator contract review. Lease: read-only source/tests plus this preparation worklog. No production, test, shared model, module-registration or index changes. RTK/Ponytail guidance applies; implement the smallest complete PostgreSQL-only observed contract after approval. This does not enable an existing-target policy or close T11/M2.

## Reference research before proposal

Read docs/reference-coding.md, scope-and-compatibility.md:77–87, testing-and-performance.md DDL/DDL-BEHAVIOR/TARGET and verification gates, current model.rs:110–165/SchemaExpectations, postgres/mod.rs:509–630 and all adjacent inspection SQL, plan existing-target and collision decisions, verify/indexes.sql and verify/mod.rs default/structure/sequence checks, tests/cases/t07_postgres.rs:278–333 and t15_verify.rs default/sequence matrix, and T11 native structure evidence. Current inspector has ordered columns and table privileges but erases identity generation modes; it lacks defaults/generated expressions/comments, actual indexes/constraints/triggers/sequence ownership and state, and occupied-index owners. Incoming edges only cover FKs/views; human-readable external dependency descriptions are not a complete recreation proof.

XERJ project-my2pg `TARGET data_only recreate sequence metadata` returned plan/mod.rs:901 and the T11/T10 worklogs at indexed revision66c92db30cc1ba0a3ecdd9c0ac3ae4fe253f270f plus working snapshot. `introspect indexes foreign_keys sequences` in ref-paganel at ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc returned schema planning rather than target introspection; local rg located and original-read connectors/src/drivers/postgres/introspector.rs:45–101, its index_metadata.sql/fk_metadata.sql, sql/metadata/index.rs and table.rs, engine-schema/planner.rs:240–268 and plan.rs:315–355/470–485/1280–1325 adjacent tests. It represents richer index parts and sequence ownership, but its metadata decoder silently defaults malformed/missing values and its index SQL joins pg_class by index name without namespace and orders aggregated parts by attnum. Reject those behaviors; our query failures stay errors, indexes join by OID, and parts order by explicit ordinal. Do not adopt dropping existing FKs in data_only.

XERJ project-pgloader `list-pgsql-sequences table-dependencies` returned schema creation and unrelated load/regress passages; narrower `list-all-indexes` located pgsql-schema.lisp:181. Pin231ab86778ca5ffd7de40878714760c8b4860cdf. Read original src/pgsql/sql/list-all-indexes.sql, list-all-fkeys.sql and list-all-sqltypes.sql, pgsql-create-schema.lisp:540–595 and clojure/test/pgloader/ddl_test.clj:420–485. Adapt explicit owner/type/extension provenance and sequence-bound tests independently. Reject comma aggregates losing identifier/order, generic reset-by-nextval-SQL rewriting, and DROP CASCADE. References remain read-only; no copied code or fixtures.

Primary PostgreSQL16 catalog references confirm the observations underlying the proposed fields: ordered key/include parts and readiness/validity are separate index facts ([pg_index](https://www.postgresql.org/docs/16/catalog-pg-index.html)); identity mode and generation expression differ from defaults ([pg_attribute](https://www.postgresql.org/docs/16/catalog-pg-attribute.html)); FK/check validation and deferral matter ([pg_constraint](https://www.postgresql.org/docs/16/catalog-pg-constraint.html)); trigger enable state and internal ownership are distinct ([pg_trigger](https://www.postgresql.org/docs/16/catalog-pg-trigger.html)). Normal/automatic/internal/extension dependency edges have different DROP behavior, so an object's schema alone cannot prove safe replacement ([pg_depend](https://www.postgresql.org/docs/16/catalog-pg-depend.html)).

Sequence generation options come from pg_sequence while last_value/is_called require state reads; SELECT, UPDATE and ownership are separate capabilities ([pg_sequence](https://www.postgresql.org/docs/16/catalog-pg-sequence.html), [sequence functions](https://www.postgresql.org/docs/16/functions-sequence.html)). TRUNCATE defaults include inheritance descendants, RESTRICT prevents unlisted FK tables, and ON TRUNCATE triggers execute ([TRUNCATE](https://www.postgresql.org/docs/16/sql-truncate.html)). Our proposed basic policy blocks inheritance/partition handling until selected closure is proved and preserves user triggers without claiming their functions are translated or side-effect-free.

## Proposed additive Rust contract

Keep current fields for compatibility. Add serde-default Option observations; absent legacy JSON metadata must mean unknown, never an empty inspected schema. The coordinator owns final naming/model additions and literal updates. Definitions below are a proposed interface, not source files or compiled code.

```rust
// Add to existing structures:
TargetCatalog.observed: Option<TargetNamespaceMetadata>
TargetTable.observed: Option<TargetTableMetadata>
TargetColumn.observed: Option<TargetColumnMetadata>

struct RelationIdentity { schema: String, name: String }
struct ColumnIdentity { table: RelationIdentity, column: String }
struct TypeIdentity { schema: String, name: String }
struct CatalogObjectKey { catalog: String, oid: u32, sub_id: i32 }

struct TargetNamespaceMetadata {
    schema: String,
    complete: bool,
    relations: Vec<TargetRelationMetadata>,
    types: Vec<TargetTypeMetadata>,
    objects: Vec<TargetCatalogObject>,
    dependencies: Vec<TargetObjectDependency>,
    event_triggers: Vec<TargetEventTriggerMetadata>,
}
struct TargetRelationMetadata {
    object: CatalogObjectKey,
    identity: RelationIdentity,
    kind: String, // pg_class.relkind, never coerced to ordinary table
    owning_table: Option<RelationIdentity>, // index table, not role owner
    extension: Option<String>,
}
struct TargetCatalogObject {
    object: CatalogObjectKey,
    kind: String,
    schema: Option<String>,
    name: String,
    owning_table: Option<RelationIdentity>,
    description: String,
}
struct TargetObjectDependency {
    dependent: CatalogObjectKey,
    referenced: CatalogObjectKey,
    dependency_kind: String, // preserve n/a/i/e/x/P/S or unknown
}
struct TargetTypeMetadata {
    object: CatalogObjectKey,
    identity: TypeIdentity,
    kind: String, // enum/domain/composite/base/array/etc; preserve actual typtype
    element_type: Option<TypeIdentity>,
    relation: Option<RelationIdentity>, // composite owning relation
    enum_labels: Option<Vec<String>>, // Some([]) is an inspected empty enum
    can_use: bool,
    can_alter: bool,
    extension: Option<String>,
}
struct TargetColumnMetadata {
    ordinal: i16,
    type_identity: TypeIdentity,
    type_modifier: i32,
    array_dimensions: i16,
    identity_mode: String, // "", "a", "d", or future unknown raw code
    generated_kind: String, // "", "s", "v", or future unknown raw code
    default_expression: Option<String>,
    generation_expression: Option<String>,
    comment: Option<String>,
    collation: Option<TargetCollationMetadata>,
}
struct TargetCollationMetadata {
    identity: TypeIdentity, // schema/name identity, not a PostgreSQL type claim
    provider: String,
    deterministic: bool,
    locale: Option<String>,
    version: Option<String>,
    actual_version: Option<String>,
}
struct TargetTableMetadata {
    complete: bool,
    object: CatalogObjectKey,
    relation_kind: String,
    is_partition: bool,
    inheritance_parents: Vec<RelationIdentity>,
    inheritance_children: Vec<RelationIdentity>,
    comment: Option<String>,
    indexes: Vec<TargetIndexMetadata>,
    foreign_keys: Vec<TargetForeignKeyMetadata>,
    checks: Vec<TargetCheckMetadata>,
    unsupported_constraints: Vec<TargetConstraintMetadata>,
    triggers: Vec<TargetTriggerMetadata>,
    sequences: Vec<TargetSequenceMetadata>,
}
struct TargetIndexMetadata {
    object: CatalogObjectKey,
    identity: RelationIdentity,
    table: RelationIdentity,
    simple_definition: Option<IndexExpectation>,
    parts: Vec<TargetIndexPart>,
    method: String,
    unique: bool,
    primary: bool,
    valid: bool,
    ready: bool,
    live: bool,
    immediate: bool,
    nulls_not_distinct: bool,
    predicate: Option<String>,
    constraint: Option<String>,
    constraint_deferrable: bool,
    constraint_initially_deferred: bool,
}
struct TargetIndexPart {
    ordinal: usize,
    column: Option<String>,
    expression: Option<String>,
    included: bool,
    descending: bool,
    nulls_first: bool,
    collation: Option<TypeIdentity>,
    operator_class: TypeIdentity,
    default_operator_class: bool,
}
struct TargetForeignKeyMetadata {
    object: CatalogObjectKey,
    definition: ForeignKeyExpectation,
    validated: bool,
    match_type: String,
    deferrable: bool,
    initially_deferred: bool,
    delete_set_columns: Option<Vec<String>>,
    supporting_index: RelationIdentity,
    equality_operators: Vec<TypeIdentity>, // schema/name plus signatures in implementation
}
struct TargetCheckMetadata {
    object: CatalogObjectKey,
    definition: CheckExpectation,
    validated: bool,
    no_inherit: bool,
}
struct TargetConstraintMetadata {
    object: CatalogObjectKey,
    name: String,
    kind: String,
    definition: String,
}
struct TargetTriggerMetadata {
    object: CatalogObjectKey,
    name: String,
    internal: bool,
    enabled: String, // O/D/R/A or unknown, no boolean collapse
    event_flags: i16,
    definition: String,
    constraint_name: Option<String>,
    function_identity: String, // qualified signature, not name-only
    function_definition_sha256: Option<String>,
}
struct TargetEventTriggerMetadata {
    name: String,
    enabled: String,
    event: String,
    tags: Option<Vec<String>>,
    function_identity: String,
    function_definition_sha256: Option<String>,
}
struct TargetSequenceMetadata {
    object: CatalogObjectKey,
    identity: RelationIdentity,
    owner: Option<ColumnIdentity>,
    ownership_dependency: Option<String>, // i identity / a serial, never inferred by name
    default_references: Vec<ColumnIdentity>, // inspect across schemas, including other users
    sequence_type: TypeIdentity,
    start: i64,
    increment: i64,
    min_value: i64,
    max_value: i64,
    cache: i64,
    cycle: bool,
    state: Option<TargetSequenceState>,
    can_select: bool,
    can_use: bool,
    can_update: bool, // required for setval, independently of table/sequence ALTER
    can_alter: bool,
    extension: Option<String>,
}
struct TargetSequenceState { last_value: i64, is_called: bool }
```

Use a distinct schema/name object-identity alias instead of TypeIdentity for collation/operator names in final model if the coordinator prefers; operator identity must include argument/result types (overloaded names are insufficient). CatalogObjectKey OIDs identify observed dependency edges within one snapshot only; never derive generated names or stable plan/table IDs from OIDs. Use direct BTreeMap/Vec stable sorting rather than hashing observed order. Namespace complete is set only after all discovery queries and dependency closure succeed; table complete only after every structural query succeeds. Missing sequence state remains explicit even in a complete definition inventory. Known permission denial to read state is distinguishable from no sequence; unrelated failed catalog queries must propagate errors.

Reuse IndexExpectation only for the independently recognized ordinary simple shape in simple_definition: method btree, ordered simple key columns, no INCLUDE/expression/predicate and reviewed operator classes/collations. Readiness/validity/NULL/deferral remain observed state and are checked separately; None is unsupported shape, never an empty equivalent index. Reuse ForeignKeyExpectation for ordered endpoints/actions and CheckExpectation for actual expression plus recognized built-in SET membership; state is wrapped rather than fabricated in SchemaExpectations. Do not reuse SequenceExpectation.next_minimum as an observed next value. Keep source TablePlan.primary_key original names; expected target keys remain structure.indexes.

## Query and code ownership

| Owner / file lease after approval | Query / rule | Required result |
| --- | --- | --- |
| Coordinator model.rs/contracts | Add reviewed observed types; update owned literal leases | serde-default Option unknown; fixture with known-empty inventory distinct from missing |
| E/PostgreSQL inspection, postgres/columns.sql | pg_attribute + pg_attrdef + pg_type/namespace + pg_description/collation | Ordered ordinal, exact qualified type identity/typemod, default vs generated expression, identity/generation raw modes, comments/collations |
| E/PostgreSQL inspection, new indexes.sql | pg_index + pg_class OID + pg_am + ordinality indkey/indclass/indcollation/indoption + pg_constraint | Ordered parts, key vs included, expression/predicate, operator/collation provenance, all validity/immediacy flags; no comma or name-only join |
| E/PostgreSQL inspection, new constraints.sql | pg_constraint conkey/confkey paired by ordinal, FK target/index/operators, pg_get_expr/constraintdef | Validated state, deferral/match/actions/subset SET NULL, checks including SET membership, unsupported exclusions/constraint triggers |
| E/PostgreSQL inspection, triggers.sql | pg_trigger + pg_proc qualified signature, pg_get_triggerdef; pg_event_trigger separately | Preserve internal/user enable state and definition/hash; do not execute function, expose body, disable trigger or set replication_role |
| E/PostgreSQL inspection, sequences.sql | pg_sequence options + pg_depend ownership + pg_attrdef dependency references + ACLs | Identity/owned serial/unowned/shared binding, all outside-column users, independent SELECT/USAGE/UPDATE/owner capabilities |
| E/PostgreSQL inspection, state reads | SELECT last_value,is_called from safely quoted discovered sequence relation | Read-only state; never nextval/currval/setval during inspect/plan; None with denied capability remains unknown |
| E/PostgreSQL inspection, relations/types.sql | pg_class/pg_index owner, pg_type LEFT JOIN pg_enum ordered enumsortorder and pg_depend extension/type users | Name occupancy belongs to specific selected owner or unrelated object; empty enum, domains/composites/arrays distinct |
| E/PostgreSQL inspection, object_dependencies.sql | pg_depend class/object/subobject identities and recursive incoming closure; pg_rewrite mapped to owning view; inheritance pg_inherits | All scopes for selected relation/type/owned-sequence dependencies; automatic/internal closure vs normal external blockers, unknown kinds block |
| D/planner + types.rs after renewed lease | Pure signatures/defaults/shape matching and policy decision from observed facts | No queries, no structural equality from generated SQL strings; bind existing actual names into verification expectations where semantics match |
| G/verifier after renewed lease | Reuse observed inspection and reviewed shape/default comparison | Before/after structures and preserved user trigger fingerprints; no silently narrowed equality or duplicated incompatible parsers |
| C/runner after renewed lease | Acquire scoped locks and re-inspect/revalidate before first mutation; final sequence state/maximum reads | Stale plan fails before mutation; ordering/report failures kept explicit; no CASCADE, DDL target only |

Proposed pure extension for default comparison after coordinator review: `equivalent_postgres_defaults(actual:Option<&str>, expected:Option<&str>, target_type:&str)->Option<bool>`. Reuse verifier's complete typed literal parser, add independently tested binary/temporal/ENUM/SET defaults as needed; None means unsupported expression, not different/identical. Exact string equality is insufficient for arbitrary SQL semantics. Known literals may canonicalize pg_catalog casts/parentheses only through the reviewed parser; do not evaluate target defaults just to compare them.

## Policy decisions this metadata enables

1. **Data_only / append:** require observed compatible ordered columns/types/nullability/generation/defaults, actual required keys/FKs/checks and validity. Match shape rather than generated name; attach the existing name to expectations for later verification. Inventory additional constraints and triggers, preserve them and report their potential COPY effects; never remove them or claim their source behavior equivalence. Unknown extra restrictive shape/default/generated semantics block absent a separately reviewed explicit policy. Actual COPY/commit failures remain failures, including trigger-rejected or changed rows; metadata cannot predict arbitrary trigger function effects. User-trigger fingerprints before/after prove preservation only, not equivalent business semantics.
2. **Truncate:** perform all above compatible-target checks and require TRUNCATE privilege and complete incoming FK closure; every referencing table must be explicitly selected in the single TRUNCATE ... RESTRICT statement, regardless of whether empty. Block inheritance/partition cases in the basic increment. Keep CONTINUE IDENTITY explicit; resetting sequences is a distinct chosen action. Detect ON TRUNCATE trigger effects, preserve them and require reviewed policy if their side effects cannot be scoped. No CASCADE, no automatic addition of tables or turning off constraints/triggers.
3. **Recreate:** require ownership and complete database-object dependency graph. Drop explicitly selected tables together using RESTRICT. Reuse a occupied index/relation/composite-type identifier only when its automatic/internal ownership proves it disappears with a selected replaced table. Same-schema unselected owners, cross-schema users, unknown dependencies, extension members, inheritance and partition ownership stay blockers. Inspect enabled event triggers before DDL; block until reviewed execution policy can account for their effects. Do not drop unrelated enum/domain/composite types.
4. **Enum reuse:** require exact schema/type identity, enum kind and exact ordered labels, schema/type USAGE and complete known dependencies. A compatible existing enum may be reused without ALTER or DROP; mismatched labels/order/type kind block. Recreate tables can also retain the compatible enum. No enum CASCADE or opportunistic alternate type name; unselected users must survive. Current enums map is a display/compatibility view, not sufficient proof of kind/ownership/empty-enum presence.
5. **Existing identity:** initially accept inspected BY DEFAULT identity only with exact integer type/range, increment1/min1/cache1/nocycle and exactly one column-owned internal sequence with no outside users. Record ALWAYS/owned serial/unowned nextval separately and leave them blocked until a reviewed generation-policy expectation exists; a bool cannot express those differences. Source unsigned u64 next must fit the actual signed/narrow sequence range. A non-resetting data_only run preserves state and does not need UPDATE, but verification must not falsely certify a too-low next value. A resetting run requires sequence UPDATE and state visibility. Compute the final next lower bound from sourceNext, loaded/existing MAX+1, sequence minimum and the existing actual next value; never rewind a higher target sequence. Re-read under the runner's documented target-quiescence/locking policy after COPY rather than emitting stale-state SQL from planning. setval is not rolled back by transaction rollback, so sequence mutations/report evidence are separate from batch transactional accounting. Data_only reset intent must be explicit; current default-true bool cannot distinguish an omitted option from intentional true, so coordinator/config policy must resolve this before enabling that path.
6. **Snapshot/race limit:** multi-query READ COMMITTED inspection is not a coherent immutable schema snapshot. Use a read-only coherent snapshot for planning, then acquire required scoped locks and revalidate before execution. Locks/state inspection do not prove external writers or cached sequence callers are quiescent; retain that operational limitation and block unsupported cache/concurrency policies. Catalog dependency proof covers PostgreSQL recorded objects, not arbitrary external application SQL/dynamic trigger references.

## Required independent behavioral cases before enabling gates

- Legacy catalog JSON omitting observed fields blocks requested existing-policy execution; inspected empty arrays allow known-empty structures. Metadata query permission/error cannot appear as an empty result. Inspect/plan leave rows, sequences, namespace objects and trigger counters unchanged.
- Existing compatible schema_only-created target → data_only append keeps original rows and exact renamed composite key/FK actions/comments, all constraints enabled, and user trigger O/D/R/A state/definition intact. Same semantic index with different existing name maps to real name; no attempted duplicate index/constraint DDL. Compatible typed timestamp(6)/numeric spacing works; changed modifiers/default/identity mode/generation expressions fail before any DDL/COPY.
- Missing primary/unique/FK/check, different composite order, INCLUDE/expression/predicate/nondefault opclass/collation, NULLS NOT DISTINCT, invalid/unready indexes, unvalidated or deferred constraints, FK MATCH FULL/partial SET NULL and exclusion constraints are independently identified and rejected or covered by an explicit reviewed policy. Don't modify pg_catalog directly to fabricate invalid state; use failed concurrent index build, NOT VALID and supported SQL to obtain real cases.
- Append and truncate preserve an active INSERT trigger; actual returned row changes/rejections and trigger side effects are observed and reported according to recovery/accounting policy. ON TRUNCATE trigger case proves detection and declared behavior; DELETE trigger must not be advertised as firing on TRUNCATE. Enabled DDL event trigger blocks unreviewed recreate; inactive/internal trigger inventory is not confused with user behavior.
- Truncate one referenced parent fails before mutation even if the unselected child is empty; truncate an explicitly selected closed parent/child set succeeds with RESTRICT and unrelated tables/schemas unchanged. Ordinary inheritance parent/child and partition descendants block before mutation, proving no hidden descendant truncation.
- Recreate selected graph handles its own old generated index/composite-type/owned identity names, but fails on identical index name owned by an unselected table, same/cross-schema view, incoming FK, function returning composite type, default referencing owned sequence, domain/extension dependency or unknown graph edge. Selected mutual FKs recreate together without CASCADE. Sentinel object/row/type users survive every failure and success.
- Enum reuse exercises labels with empty/comma/quotes/backslashes and identical case-sensitive qualified identity, mismatched order/labels, empty enum, domain/composite collision, denied type USAGE and unrelated table using the compatible enum. All unrelated type users remain valid and their rows unchanged after recreate.
- Existing BY DEFAULT identity proves empty/negative-only/deleted-high-ID/sourceNext cases, current next above source/MAX (never rewind), narrow signed exhaustion, last_value/is_called false/true, increment/cache/cycle mismatches, denied SELECT vs denied UPDATE vs nonowner, shared/unowned serial/default sequence users and ALWAYS mode. reset=false preserves sequence state byte-for-byte; explicit reset=true enables actual next INSERT at the bound. Failure after setval records nontransactional sequence effect accurately.
- Target schema changes between plan and execution are caught by locked revalidation before mutation; test external ADD COLUMN/type/default/index/FK/trigger/sequence changes individually. Never declare the readonly preparation or mock equality tests sufficient for runtime acceptance.

## Suggested smallest next implementation lease

Coordinator freezes the additive observed contract and grants E target-query files/inspection integration cases, then D src/plan/ plus types/default comparison and T11 existing-policy cases. Start with ordinary single-schema tables, typed defaults, complete basic indexes/FKs/checks and BY DEFAULT identity1/cache1; retain explicit blockers for inheritance/partition/expression/trigger execution ambiguity and existing serial/ALWAYS generation. G adopts the same observed contract/comparison functions; C handles locked revalidation and monotonic sequence action. No cross-schema execution or advanced T17 policy is implied by this preparation.
