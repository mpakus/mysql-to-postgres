-- $1 relation inspection root OIDs, $2 existing type root OIDs, both non-NULL oid[].
-- Discovery is NOT authorization to drop reached objects. No depth/row truncation.
WITH RECURSIVE header AS (
    SELECT (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database()) AS database_oid,
           (SELECT count(DISTINCT oid) FROM unnest($1::oid[]) x(oid)) AS relation_root_count,
           (SELECT count(DISTINCT oid) FROM unnest($2::oid[]) x(oid)) AS type_root_count,
           (SELECT count(*) FROM pg_catalog.pg_event_trigger) AS event_trigger_count,
           $1::oid[] IS NOT NULL AND $2::oid[] IS NOT NULL
           AND COALESCE(array_ndims($1::oid[]), 0) <= 1 AND COALESCE(array_ndims($2::oid[]), 0) <= 1
           AND NOT EXISTS (
               SELECT 1 FROM unnest($1::oid[]) x(oid)
               LEFT JOIN pg_catalog.pg_class c ON c.oid = x.oid
               WHERE x.oid IS NULL OR x.oid = 0 OR c.oid IS NULL
           ) AND NOT EXISTS (
               SELECT 1 FROM unnest($2::oid[]) x(oid)
               LEFT JOIN pg_catalog.pg_type t ON t.oid = x.oid
               WHERE x.oid IS NULL OR x.oid = 0 OR t.oid IS NULL
           ) AS roots_valid
), roots AS (
    SELECT 'pg_catalog.pg_class'::regclass::oid AS classid, x.oid AS objid, 0::integer AS objsubid
    FROM header h CROSS JOIN unnest($1::oid[]) x(oid) WHERE h.roots_valid
    UNION
    SELECT 'pg_catalog.pg_type'::regclass::oid, x.oid, 0
    FROM header h CROSS JOIN unnest($2::oid[]) x(oid) WHERE h.roots_valid
    UNION
    SELECT 'pg_catalog.pg_event_trigger'::regclass::oid, e.oid, 0
    FROM header h CROSS JOIN pg_catalog.pg_event_trigger e WHERE h.roots_valid
), reached(classid, objid, objsubid) AS (
    SELECT * FROM roots
    UNION
    SELECT next.classid, next.objid, next.objsubid
    FROM reached w JOIN pg_catalog.pg_depend d ON
        (d.refclassid = w.classid AND d.refobjid = w.objid AND (w.objsubid = 0 OR d.refobjsubid = w.objsubid))
        OR (d.classid = w.classid AND d.objid = w.objid AND (w.objsubid = 0 OR d.objsubid = w.objsubid) AND d.deptype = 'i')
    CROSS JOIN LATERAL (
        SELECT d.classid, d.objid, d.objsubid
        WHERE d.refclassid = w.classid AND d.refobjid = w.objid AND (w.objsubid = 0 OR d.refobjsubid = w.objsubid)
        UNION
        -- Internal owner promotion reveals rewrite-rule owners and further view users.
        SELECT d.refclassid, d.refobjid, d.refobjsubid
        WHERE d.classid = w.classid AND d.objid = w.objid AND (w.objsubid = 0 OR d.objsubid = w.objsubid) AND d.deptype = 'i'
    ) next
)
SELECT h.database_oid, h.relation_root_count, h.type_root_count, h.event_trigger_count, h.roots_valid,
       w.classid AS node_class_oid, w.objid AS node_oid, w.objsubid AS node_sub_id,
       CASE WHEN node_class.oid IS NOT NULL THEN pg_catalog.format('%I.%I', node_class_ns.nspname, node_class.relname) END AS node_catalog,
       node_id.type AS node_type, node_id.schema AS node_schema, node_id.name AS node_name, node_id.identity AS node_identity,
       COALESCE(w.classid = 'pg_catalog.pg_class'::regclass AND w.objsubid = 0 AND w.objid = ANY($1::oid[]), false) AS relation_root,
       COALESCE(w.classid = 'pg_catalog.pg_type'::regclass AND w.objsubid = 0 AND w.objid = ANY($2::oid[]), false) AS type_root,
       COALESCE(w.classid = 'pg_catalog.pg_event_trigger'::regclass, false) AS global_event_root,
       own_rel.oid AS node_owner_relation_oid, own.column_ordinal AS node_owner_column_ordinal,
       own_ns.nspname::text AS node_owner_relation_schema, own_rel.relname::text AS node_owner_relation_name,
       COALESCE(rel.relowner, typ.typowner, proc.proowner, evt.evtowner, ext_obj.extowner, own_rel.relowner, domain_type.typowner) AS node_owner_role_oid,
       pg_catalog.pg_get_userbyid(COALESCE(rel.relowner, typ.typowner, proc.proowner, evt.evtowner, ext_obj.extowner, own_rel.relowner, domain_type.typowner))::text AS node_owner_role,
       member_ext.oid AS node_extension_oid, member_ext.extname::text AS node_extension_name,
       rel.relkind::text AS node_relation_kind, typ.typtype::text AS node_type_kind,
       con.contype::text AS node_constraint_kind, proc.prokind::text AS node_function_kind,
       CASE WHEN proc.prokind IN ('f', 'p', 'w') THEN pg_catalog.pg_get_functiondef(proc.oid) END AS node_function_definition,
       CASE WHEN proc.oid IS NOT NULL THEN proc.prosqlbody IS NOT NULL END AS node_function_body_parsed,
       evt.evtenabled::text AS node_event_enabled, evt.evtevent::text AS node_event,
       edge.catalog AS dependency_catalog, edge.direction AS dependency_direction, edge.deptype::text AS dependency_kind,
       edge.classid AS dependent_class_oid, edge.objid AS dependent_oid, edge.objsubid AS dependent_sub_id,
       edge.refclassid AS referenced_class_oid, edge.refobjid AS referenced_oid, edge.refobjsubid AS referenced_sub_id,
       CASE WHEN dep_class.oid IS NOT NULL THEN pg_catalog.format('%I.%I', dep_ns.nspname, dep_class.relname) END AS dependent_catalog,
       CASE WHEN ref_class.oid IS NOT NULL THEN pg_catalog.format('%I.%I', ref_ns.nspname, ref_class.relname) END AS referenced_catalog,
       dep_id.type AS dependent_type, dep_id.schema AS dependent_schema, dep_id.name AS dependent_name, dep_id.identity AS dependent_identity,
       CASE WHEN ref_role.oid IS NOT NULL THEN 'role' ELSE ref_id.type END AS referenced_type,
       ref_id.schema AS referenced_schema, COALESCE(ref_role.rolname::text, ref_id.name) AS referenced_name,
       CASE WHEN ref_role.oid IS NOT NULL THEN pg_catalog.quote_ident(ref_role.rolname) ELSE ref_id.identity END AS referenced_identity
FROM header h
LEFT JOIN reached w ON true
LEFT JOIN pg_catalog.pg_class node_class ON node_class.oid = w.classid
LEFT JOIN pg_catalog.pg_namespace node_class_ns ON node_class_ns.oid = node_class.relnamespace
LEFT JOIN LATERAL pg_catalog.pg_identify_object(w.classid, w.objid, w.objsubid) node_id ON true
LEFT JOIN pg_catalog.pg_class rel ON w.classid = 'pg_catalog.pg_class'::regclass AND rel.oid = w.objid
LEFT JOIN pg_catalog.pg_type typ ON w.classid = 'pg_catalog.pg_type'::regclass AND typ.oid = w.objid
LEFT JOIN pg_catalog.pg_proc proc ON w.classid = 'pg_catalog.pg_proc'::regclass AND proc.oid = w.objid
LEFT JOIN pg_catalog.pg_event_trigger evt ON w.classid = 'pg_catalog.pg_event_trigger'::regclass AND evt.oid = w.objid
LEFT JOIN pg_catalog.pg_extension ext_obj ON w.classid = 'pg_catalog.pg_extension'::regclass AND ext_obj.oid = w.objid
LEFT JOIN pg_catalog.pg_constraint con ON w.classid = 'pg_catalog.pg_constraint'::regclass AND con.oid = w.objid
LEFT JOIN pg_catalog.pg_type domain_type ON domain_type.oid = con.contypid
LEFT JOIN LATERAL (
    SELECT i.indrelid AS relation_oid, NULL::smallint AS column_ordinal FROM pg_catalog.pg_index i
    WHERE w.classid = 'pg_catalog.pg_class'::regclass AND i.indexrelid = w.objid
    UNION
    SELECT w.objid, w.objsubid::smallint
    WHERE w.classid = 'pg_catalog.pg_class'::regclass AND w.objsubid <> 0
    UNION
    SELECT d.refobjid, a.attnum FROM pg_catalog.pg_depend d
    JOIN pg_catalog.pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid
    WHERE rel.relkind = 'S' AND d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = w.objid
      AND d.refclassid = 'pg_catalog.pg_class'::regclass AND d.refobjsubid > 0 AND d.deptype IN ('a', 'i')
    UNION
    SELECT a.adrelid, a.adnum FROM pg_catalog.pg_attrdef a WHERE w.classid = 'pg_catalog.pg_attrdef'::regclass AND a.oid = w.objid
    UNION
    SELECT con.conrelid, NULL::smallint WHERE con.conrelid <> 0
    UNION
    SELECT t.tgrelid, NULL::smallint FROM pg_catalog.pg_trigger t WHERE w.classid = 'pg_catalog.pg_trigger'::regclass AND t.oid = w.objid
    UNION
    SELECT r.ev_class, NULL::smallint FROM pg_catalog.pg_rewrite r WHERE w.classid = 'pg_catalog.pg_rewrite'::regclass AND r.oid = w.objid
    UNION
    SELECT typ.typrelid, NULL::smallint WHERE typ.typrelid <> 0
) own ON true
LEFT JOIN pg_catalog.pg_class own_rel ON own_rel.oid = own.relation_oid
LEFT JOIN pg_catalog.pg_namespace own_ns ON own_ns.oid = own_rel.relnamespace
LEFT JOIN pg_catalog.pg_depend member ON member.classid = w.classid AND member.objid = w.objid
  AND member.objsubid = 0 AND member.refclassid = 'pg_catalog.pg_extension'::regclass AND member.deptype = 'e'
LEFT JOIN pg_catalog.pg_extension member_ext ON member_ext.oid = member.refobjid
LEFT JOIN LATERAL (
    SELECT 'pg_catalog.pg_depend'::text AS catalog,
           CASE WHEN d.refclassid = w.classid AND d.refobjid = w.objid AND (w.objsubid = 0 OR d.refobjsubid = w.objsubid) THEN 'incoming' ELSE 'outgoing' END AS direction,
           d.*
    FROM pg_catalog.pg_depend d
    WHERE (d.refclassid = w.classid AND d.refobjid = w.objid AND (w.objsubid = 0 OR d.refobjsubid = w.objsubid))
       OR (d.classid = w.classid AND d.objid = w.objid AND (w.objsubid = 0 OR d.objsubid = w.objsubid))
    UNION ALL
    SELECT 'pg_catalog.pg_shdepend', 'outgoing', d.classid, d.objid, d.objsubid, d.refclassid, d.refobjid, 0::integer, d.deptype
    FROM pg_catalog.pg_shdepend d
    WHERE d.dbid = h.database_oid AND d.classid = w.classid AND d.objid = w.objid AND (w.objsubid = 0 OR d.objsubid = w.objsubid)
) edge ON true
LEFT JOIN pg_catalog.pg_class dep_class ON dep_class.oid = edge.classid
LEFT JOIN pg_catalog.pg_namespace dep_ns ON dep_ns.oid = dep_class.relnamespace
LEFT JOIN pg_catalog.pg_class ref_class ON ref_class.oid = edge.refclassid
LEFT JOIN pg_catalog.pg_namespace ref_ns ON ref_ns.oid = ref_class.relnamespace
LEFT JOIN LATERAL pg_catalog.pg_identify_object(edge.classid, edge.objid, edge.objsubid) dep_id ON true
LEFT JOIN pg_catalog.pg_roles ref_role ON edge.refclassid = 'pg_catalog.pg_authid'::regclass AND ref_role.oid = edge.refobjid
LEFT JOIN LATERAL pg_catalog.pg_identify_object(CASE WHEN edge.refclassid <> 'pg_catalog.pg_authid'::regclass THEN edge.refclassid END, edge.refobjid, edge.refobjsubid) ref_id ON true
ORDER BY w.classid, w.objid, w.objsubid, edge.catalog, edge.classid, edge.objid, edge.objsubid, edge.refclassid, edge.refobjid, edge.refobjsubid, edge.deptype
