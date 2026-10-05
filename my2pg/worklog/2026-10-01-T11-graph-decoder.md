# 2026-10-01 — T11 — typed type and dependency observations

Status: implementation in progress; no existing-target policy or milestone accepted. Coordinator lease: shared model, new PostgreSQL graph decoder, inspector integration and registration. D owns query SQL and native graph cases. Base `444f9a119ed26e2c7e287b2fd17b2e92f2e66638`.

## Reference research before coding

RTK and Ponytail guidance read; reuse the existing fallible typed-row inspection and the driver-owned repeatable-read, read-only transaction. No dependency added. XERJ `project-my2pg`, query `dependency closure object subobject enum reuse`, returned the existing-catalog proposal and planner at `444f9a119ed26e2c7e287b2fd17b2e92f2e66638`. Read that proposal, the observed model and decoder, inspector, native decoder cases and contract fixtures. `project-pgloader`, query `list-all-sqltypes`, at `231ab86778ca5ffd7de40878714760c8b4860cdf` did not rank the SQL definition first; read original `src/pgsql/sql/list-all-sqltypes.sql` and adjacent Clojure enum/sequence DDL tests at lines420–485. Its labels retain sort order, but its used-column roots omit unused/empty types and do not prove a recursive dependency closure. `ref-paganel`, query `introspector enum dependencies`, at `ced0a3c953bcf63bba6085bf2b8b9fc75cbbeddc` returned graph expansion/provider code. Read PostgreSQL introspector, schema manager and adjacent metadata/planner tests already identified in the proposal. Reject default-on-decode-error and duplicate-object suppression as positive compatibility proof. No source or fixtures copied.

D's [query contract](2026-10-01-T11-dependency-closure.md) is approved. Read the original PostgreSQL catalog/identity behavior documented there. Preserve actual class OIDs, object/subobject OIDs, raw flags, type/enum order and all fallible query outcomes. Broad incoming discovery and internal-owner promotion are observations, never mutation authorization.

## Adaptation and independent checks

Add optional catalog observations; absent legacy JSON means uninspected, while successful empty inventory is explicit. Keep flat typed rows, matching current index/sequence/trigger observations, rather than introducing an abstract graph engine. A required graph header records counts and root validity. Header-only rows must not fabricate zero-valued objects. Decode every required field explicitly, reject invalid/contradictory headers, identities or incomplete endpoint pairs, and retain repeated incident facts with their exact provenance.

Full namespace inspection supplies all discovered table/type OIDs as inspection roots because the current inspector does not receive planner selection. The planner must derive selected mutation roots independently. Requested type roots and discovered dependents must not imply DROP permission. Query/decoder errors propagate inside the existing read-only transaction; no nextval, setval, DDL or CASCADE is introduced.

Verify with independent serialization known-empty/unknown contracts plus D's actual PostgreSQL multi-hop view, function/type/default/shared-sequence, role/event and invalid-root cases. Existing typed decoder and schema-only migration cases must still pass. This does not close dynamic function-body tracking, concurrent-DDL revalidation, existing identity reset, or full existing-target policy gates.

## XERJ setup recheck

Live local XERJ `1.0.0-rc.80` recheck passed all7 corpus counts, all6 clean reference pins and21 expected-source queries; cluster green. Project snapshot remains `444f9a1`:204 files/703 passages; total2332 files/7341 passages. Active worker changes are not represented until coordinator refresh after handoff. No external embeddings, reranking or feedback publication used.

## Independent review before corrective changes

F's read-only review identified orphan fields on NULL type headers and unvalidated incident-edge/node or owner/extension relationships. Accept the findings and require native injected-query regressions. Extend validation without inventing ownership or mutation selection. Also re-read primary [pg_attribute](https://www.postgresql.org/docs/16/catalog-pg-attribute.html) and [pg_depend](https://www.postgresql.org/docs/16/catalog-pg-depend.html): system-column attnums are negative, and dependency subids are actual column numbers. Our initial nonnegative subid check was too strict. Preserve signed pg_class column identifiers, reject nonzero subids on non-column catalogs, and validate incoming/outgoing endpoint coverage against the actual reached node. F's native view-system-column oracle will prove the adaptation before acceptance. Broad graph facts remain unconsumed by existing-target mutation policies.
## Coordinator checkpoint

Source61dc35c includes these decoders and the peer's native malformed/header/incident-edge/system-column regressions. Fresh complete default database lane119e172ad26a passes102cases, including all4new graph cases; pair stopped. Shared models and offline empty-versus-unknown contract pass within121ordinary cases. Strict all-target/all-feature Clippy and formatting pass. [Checkpoint](2026-10-01-M2-parallel-graph-import-checkpoint.md) retains future policy/serialization/lock/sequence gates; metadata observations alone do not finish T11.
