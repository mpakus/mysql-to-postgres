-- $1 exact namespace; LEFT headers preserve unused types and zero-label enums.
SELECT n.oid AS namespace_oid, n.nspname::text AS namespace_name,
       pg_catalog.has_schema_privilege(n.oid, 'USAGE') AS namespace_can_use,
       pg_catalog.has_schema_privilege(n.oid, 'CREATE') AS namespace_can_create,
       t.oid AS type_oid, tn.nspname::text AS type_schema, t.typname::text AS type_name,
       t.typowner AS type_owner_role_oid, pg_catalog.pg_get_userbyid(t.typowner)::text AS type_owner_role,
       t.typtype::text AS type_kind, t.typcategory::text AS type_category,
       t.typisdefined AS type_defined, t.typbyval AS type_by_value, t.typlen AS type_length,
       t.typalign::text AS type_alignment, t.typstorage::text AS type_storage,
       t.typrelid AS type_relation_oid, t.typelem AS type_element_oid, t.typarray AS type_array_oid,
       t.typbasetype AS type_base_oid, t.typtypmod AS type_modifier, t.typndims AS domain_array_dimensions,
       t.typcollation AS type_collation_oid, t.typnotnull AS domain_not_null,
       pg_catalog.pg_get_expr(t.typdefaultbin, 0::oid) AS type_default_expression,
       pg_catalog.obj_description(t.oid, 'pg_type') AS type_comment,
       CASE WHEN t.oid IS NOT NULL THEN pg_catalog.has_type_privilege(t.oid, 'USAGE') END AS can_use,
       CASE WHEN t.oid IS NOT NULL THEN pg_catalog.pg_has_role(t.typowner, 'USAGE') END AS can_alter,
       ext.oid AS extension_oid, ext.extname::text AS extension_name,
       e.oid AS enum_label_oid, e.enumlabel::text AS enum_label, e.enumsortorder AS enum_sort_order,
       c.oid AS domain_constraint_oid, c.conname::text AS domain_constraint_name,
       c.contype::text AS domain_constraint_kind, c.convalidated AS domain_constraint_validated,
       c.condeferrable AS domain_constraint_deferrable, c.condeferred AS domain_constraint_initially_deferred,
       pg_catalog.pg_get_constraintdef(c.oid, false) AS domain_constraint_definition,
       pg_catalog.pg_get_expr(c.conbin, 0::oid) AS domain_constraint_expression
FROM pg_catalog.pg_namespace n
LEFT JOIN pg_catalog.pg_type t ON t.typnamespace = n.oid
LEFT JOIN pg_catalog.pg_namespace tn ON tn.oid = t.typnamespace
LEFT JOIN pg_catalog.pg_enum e ON e.enumtypid = t.oid
LEFT JOIN pg_catalog.pg_constraint c ON c.contypid = t.oid
LEFT JOIN pg_catalog.pg_depend member ON member.classid = 'pg_catalog.pg_type'::regclass
  AND member.objid = t.oid AND member.refclassid = 'pg_catalog.pg_extension'::regclass AND member.deptype = 'e'
LEFT JOIN pg_catalog.pg_extension ext ON ext.oid = member.refobjid
WHERE n.nspname = $1::text
ORDER BY t.typname COLLATE "C", t.oid, e.enumsortorder, e.oid, c.oid
