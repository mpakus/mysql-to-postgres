-- $1 schema; $2 table. NULL table requests ALL database event triggers.
-- Function source is provenance, not a proof of absence of dynamic side effects.
WITH headers AS (
    SELECT 'table'::text AS scope, t.oid AS trigger_oid, t.tgname::text AS trigger_name,
           t.tgenabled::text AS enabled, t.tgisinternal AS internal,
           c.oid AS table_oid, n.nspname::text AS table_schema, c.relname::text AS table_name,
           NULLIF(t.tgconstraint, 0::oid) AS constraint_oid, con.conname::text AS constraint_name,
           NULLIF(t.tgparentid, 0::oid) AS trigger_parent_oid,
           pg_catalog.pg_get_triggerdef(t.oid, false) AS definition,
           t.tgtype AS event_flags, NULL::text AS event, NULL::text[] AS event_tags,
           t.tgfoid AS function_oid, 'pg_catalog.pg_trigger'::regclass::oid AS object_class
    FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    LEFT JOIN pg_catalog.pg_constraint con ON con.oid = t.tgconstraint
    WHERE $2::text IS NOT NULL AND n.nspname = $1 AND c.relname = $2
    UNION ALL
    SELECT 'event', e.oid, e.evtname::text, e.evtenabled::text, NULL::boolean,
           NULL::oid, NULL::text, NULL::text, NULL::oid, NULL::text, NULL::oid,
           NULL::text, NULL::smallint, e.evtevent::text, e.evttags, e.evtfoid,
           'pg_catalog.pg_event_trigger'::regclass::oid
    FROM pg_catalog.pg_event_trigger e WHERE $2::text IS NULL
)
SELECT h.scope, h.trigger_oid, h.trigger_name, h.enabled, h.internal,
       h.table_oid, h.table_schema, h.table_name, h.constraint_oid, h.constraint_name,
       h.trigger_parent_oid, h.definition, h.event_flags, h.event, h.event_tags,
       h.function_oid, pn.nspname::text AS function_schema, p.proname::text AS function_name,
       pg_catalog.pg_get_function_identity_arguments(p.oid) AS function_arguments,
       pg_catalog.format('%I.%I(%s)', pn.nspname, p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid)) AS function_identity,
       pg_catalog.pg_get_functiondef(p.oid) AS function_definition,
       lang.lanname::text AS function_language, p.provolatile::text AS function_volatility,
       pg_catalog.pg_get_userbyid(p.proowner)::text AS function_owner_role,
       p.prosecdef AS function_security_definer, p.proconfig AS function_config,
       edge.subject AS dependency_subject, edge.direction AS dependency_direction,
       edge.deptype::text AS dependency_kind, edge.catalog AS dependency_catalog,
       CASE WHEN dep_class.oid IS NOT NULL THEN pg_catalog.format('%I.%I', dep_ns.nspname, dep_class.relname) END AS dependent_catalog,
       edge.objid AS dependent_oid, edge.objsubid AS dependent_sub_id,
       CASE WHEN ref_class.oid IS NOT NULL THEN pg_catalog.format('%I.%I', ref_ns.nspname, ref_class.relname) END AS referenced_catalog,
       edge.refobjid AS referenced_oid, edge.refobjsubid AS referenced_sub_id,
       dep_id.type AS dependent_type, dep_id.schema AS dependent_schema,
       dep_id.name AS dependent_name, dep_id.identity AS dependent_identity,
       CASE WHEN ref_role.oid IS NOT NULL THEN 'role' ELSE ref_id.type END AS referenced_type,
       ref_id.schema AS referenced_schema,
       COALESCE(ref_role.rolname::text, ref_id.name) AS referenced_name,
       CASE WHEN ref_role.oid IS NOT NULL THEN pg_catalog.quote_ident(ref_role.rolname) ELSE ref_id.identity END AS referenced_identity
FROM headers h
JOIN pg_catalog.pg_proc p ON p.oid = h.function_oid
JOIN pg_catalog.pg_namespace pn ON pn.oid = p.pronamespace
JOIN pg_catalog.pg_language lang ON lang.oid = p.prolang
LEFT JOIN LATERAL (
    SELECT 'trigger'::text AS subject, 'outgoing'::text AS direction, 'pg_catalog.pg_depend'::text AS catalog, d.* FROM pg_catalog.pg_depend d
    WHERE d.classid = h.object_class AND d.objid = h.trigger_oid
    UNION ALL
    SELECT 'trigger', 'incoming', 'pg_catalog.pg_depend', d.* FROM pg_catalog.pg_depend d
    WHERE d.refclassid = h.object_class AND d.refobjid = h.trigger_oid
    UNION ALL
    SELECT 'function', 'outgoing', 'pg_catalog.pg_depend', d.* FROM pg_catalog.pg_depend d
    WHERE d.classid = 'pg_catalog.pg_proc'::regclass AND d.objid = h.function_oid
    UNION ALL
    SELECT 'function', 'incoming', 'pg_catalog.pg_depend', d.* FROM pg_catalog.pg_depend d
    WHERE d.refclassid = 'pg_catalog.pg_proc'::regclass AND d.refobjid = h.function_oid
    UNION ALL
    SELECT CASE WHEN d.classid = h.object_class THEN 'trigger' ELSE 'function' END,
           'outgoing', 'pg_catalog.pg_shdepend', d.classid, d.objid, d.objsubid,
           d.refclassid, d.refobjid, 0::integer AS refobjsubid, d.deptype
    FROM pg_catalog.pg_shdepend d
    WHERE d.dbid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
      AND ((d.classid = h.object_class AND d.objid = h.trigger_oid)
           OR (d.classid = 'pg_catalog.pg_proc'::regclass AND d.objid = h.function_oid))
) edge ON true
LEFT JOIN pg_catalog.pg_class dep_class ON dep_class.oid = edge.classid
LEFT JOIN pg_catalog.pg_namespace dep_ns ON dep_ns.oid = dep_class.relnamespace
LEFT JOIN pg_catalog.pg_class ref_class ON ref_class.oid = edge.refclassid
LEFT JOIN pg_catalog.pg_namespace ref_ns ON ref_ns.oid = ref_class.relnamespace
LEFT JOIN LATERAL pg_catalog.pg_identify_object(edge.classid, edge.objid, edge.objsubid) dep_id ON true
LEFT JOIN pg_catalog.pg_roles ref_role ON edge.refclassid = 'pg_catalog.pg_authid'::regclass AND ref_role.oid = edge.refobjid
LEFT JOIN LATERAL pg_catalog.pg_identify_object(
    CASE WHEN edge.refclassid <> 'pg_catalog.pg_authid'::regclass THEN edge.refclassid END,
    edge.refobjid, edge.refobjsubid) ref_id ON true
ORDER BY h.scope, h.trigger_name COLLATE "C", h.trigger_oid, edge.subject, edge.direction,
         edge.catalog, edge.classid, edge.objid, edge.objsubid, edge.refclassid, edge.refobjid, edge.refobjsubid
