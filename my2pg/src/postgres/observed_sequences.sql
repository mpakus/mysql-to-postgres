-- $1 schema, $2 table. Headers survive even when a dependency join is empty.
-- State is deliberately separate: never call nextval/currval/setval here.
WITH target AS (
    SELECT c.oid FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = $1 AND c.relname = $2
), bindings AS (
    SELECT d.objid AS sequence_oid, d.refobjsubid AS column_ordinal
    FROM pg_catalog.pg_depend d JOIN target t ON t.oid = d.refobjid
    WHERE d.classid = 'pg_catalog.pg_class'::regclass
      AND d.refclassid = 'pg_catalog.pg_class'::regclass
      AND d.deptype IN ('a', 'i') AND d.refobjsubid > 0
    UNION
    SELECT d.refobjid, ad.adnum::integer
    FROM pg_catalog.pg_attrdef ad JOIN target t ON t.oid = ad.adrelid
    JOIN pg_catalog.pg_depend d ON d.classid = 'pg_catalog.pg_attrdef'::regclass
      AND d.objid = ad.oid AND d.refclassid = 'pg_catalog.pg_class'::regclass
), headers AS (
    SELECT b.*, c.relname, n.nspname, c.relowner, s.*,
           ty.typname, tn.nspname AS type_namespace,
           a.attname, a.attidentity, pg_catalog.pg_get_expr(ad.adbin, ad.adrelid) AS default_expression
    FROM bindings b JOIN pg_catalog.pg_class c ON c.oid = b.sequence_oid AND c.relkind = 'S'
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid
    JOIN pg_catalog.pg_type ty ON ty.oid = s.seqtypid
    JOIN pg_catalog.pg_namespace tn ON tn.oid = ty.typnamespace
    JOIN target t ON true
    JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid AND a.attnum = b.column_ordinal
      AND NOT a.attisdropped
    LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
)
SELECT h.sequence_oid, h.seqtypid AS sequence_type_oid,
       h.nspname::text AS sequence_schema, h.relname::text AS sequence_name,
       h.type_namespace::text AS type_schema, h.typname::text AS type_name,
       pg_catalog.pg_get_userbyid(h.relowner)::text AS sequence_owner_role,
       h.attname::text AS bound_column_name, h.column_ordinal::smallint AS bound_column_ordinal,
       h.attidentity::text AS bound_identity_mode, h.default_expression AS bound_default_expression,
       own_ns.nspname::text AS owner_table_schema, own_table.relname::text AS owner_table_name,
       own_att.attname::text AS owner_column_name, own_att.attnum AS owner_column_ordinal,
       own.deptype::text AS ownership_dependency,
       h.seqstart AS start_value, h.seqincrement AS increment_by, h.seqmin AS min_value,
       h.seqmax AS max_value, h.seqcache AS cache_size, h.seqcycle AS cycle,
       pg_catalog.pg_has_role(h.relowner, 'USAGE') AS can_alter,
       pg_catalog.has_sequence_privilege(h.sequence_oid, 'USAGE') AS can_use,
       pg_catalog.has_sequence_privilege(h.sequence_oid, 'SELECT') AS can_select,
       pg_catalog.has_sequence_privilege(h.sequence_oid, 'UPDATE') AS can_update,
       ext.extname::text AS extension_name,
       edge.direction AS dependency_direction, edge.deptype::text AS dependency_kind,
       edge.catalog AS dependency_catalog,
       CASE WHEN dep_class.oid IS NOT NULL THEN pg_catalog.format('%I.%I', dep_ns.nspname, dep_class.relname) END AS dependent_catalog,
       edge.objid AS dependent_oid, edge.objsubid AS dependent_sub_id,
       CASE WHEN ref_class.oid IS NOT NULL THEN pg_catalog.format('%I.%I', ref_ns.nspname, ref_class.relname) END AS referenced_catalog,
       edge.refobjid AS referenced_oid, edge.refobjsubid AS referenced_sub_id,
       dep_id.type AS dependent_type, dep_id.schema AS dependent_schema,
       dep_id.name AS dependent_name, dep_id.identity AS dependent_identity,
       CASE WHEN ref_role.oid IS NOT NULL THEN 'role' ELSE ref_id.type END AS referenced_type,
       ref_id.schema AS referenced_schema,
       COALESCE(ref_role.rolname::text, ref_id.name) AS referenced_name,
       CASE WHEN ref_role.oid IS NOT NULL THEN pg_catalog.quote_ident(ref_role.rolname) ELSE ref_id.identity END AS referenced_identity,
       default_ns.nspname::text AS default_reference_schema,
       default_table.relname::text AS default_reference_table,
       default_att.attname::text AS default_reference_column,
       default_att.attnum AS default_reference_ordinal,
       pg_catalog.pg_get_expr(default_ad.adbin, default_ad.adrelid) AS default_reference_expression
FROM headers h
LEFT JOIN pg_catalog.pg_depend own ON own.classid = 'pg_catalog.pg_class'::regclass
  AND own.objid = h.sequence_oid AND own.refclassid = 'pg_catalog.pg_class'::regclass
  AND own.refobjsubid > 0 AND own.deptype IN ('a', 'i')
LEFT JOIN pg_catalog.pg_class own_table ON own_table.oid = own.refobjid
LEFT JOIN pg_catalog.pg_namespace own_ns ON own_ns.oid = own_table.relnamespace
LEFT JOIN pg_catalog.pg_attribute own_att ON own_att.attrelid = own.refobjid AND own_att.attnum = own.refobjsubid
LEFT JOIN pg_catalog.pg_depend ext_dep ON ext_dep.classid = 'pg_catalog.pg_class'::regclass
  AND ext_dep.objid = h.sequence_oid AND ext_dep.refclassid = 'pg_catalog.pg_extension'::regclass AND ext_dep.deptype = 'e'
LEFT JOIN pg_catalog.pg_extension ext ON ext.oid = ext_dep.refobjid
LEFT JOIN LATERAL (
    SELECT 'incoming'::text AS direction, 'pg_catalog.pg_depend'::text AS catalog, d.* FROM pg_catalog.pg_depend d
    WHERE d.refclassid = 'pg_catalog.pg_class'::regclass AND d.refobjid = h.sequence_oid
    UNION ALL
    SELECT 'outgoing'::text, 'pg_catalog.pg_depend'::text, d.* FROM pg_catalog.pg_depend d
    WHERE d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = h.sequence_oid
    UNION ALL
    SELECT 'outgoing', 'pg_catalog.pg_shdepend', d.classid, d.objid, d.objsubid,
           d.refclassid, d.refobjid, 0::integer AS refobjsubid, d.deptype
    FROM pg_catalog.pg_shdepend d
    WHERE d.dbid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
      AND d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = h.sequence_oid
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
LEFT JOIN pg_catalog.pg_attrdef default_ad ON edge.classid = 'pg_catalog.pg_attrdef'::regclass
  AND edge.objid = default_ad.oid AND edge.direction = 'incoming'
LEFT JOIN pg_catalog.pg_class default_table ON default_table.oid = default_ad.adrelid
LEFT JOIN pg_catalog.pg_namespace default_ns ON default_ns.oid = default_table.relnamespace
LEFT JOIN pg_catalog.pg_attribute default_att ON default_att.attrelid = default_ad.adrelid AND default_att.attnum = default_ad.adnum
ORDER BY h.nspname COLLATE "C", h.relname COLLATE "C", h.column_ordinal,
         edge.direction, edge.catalog, edge.classid, edge.objid, edge.objsubid, edge.refclassid, edge.refobjid, edge.refobjsubid
