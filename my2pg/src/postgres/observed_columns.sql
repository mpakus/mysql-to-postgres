-- $1 schema, $2 table. One explicitly typed row per live column, in physical order.
SELECT a.attnum AS ordinal,
       a.attname::text AS name,
       pg_catalog.format_type(a.atttypid,a.atttypmod) AS data_type,
       ty.oid AS type_oid,
       ty.typtype::text AS type_kind,
       ty.typbasetype AS type_base_oid,
       ty.typelem AS type_element_oid,
       tn.nspname::text AS type_schema,
       ty.typname::text AS type_name,
       a.atttypmod AS type_modifier,
       a.attndims AS array_dimensions,
       NOT a.attnotnull AS nullable,
       a.attidentity::text AS identity_mode,
       a.attgenerated::text AS generated_kind,
       CASE WHEN a.attgenerated='' THEN pg_catalog.pg_get_expr(d.adbin,d.adrelid) END AS default_expression,
       CASE WHEN a.attgenerated<>'' THEN pg_catalog.pg_get_expr(d.adbin,d.adrelid) END AS generation_expression,
       pg_catalog.col_description(t.oid,a.attnum) AS comment,
       co.oid AS collation_oid,
       cn.nspname::text AS collation_schema,
       co.collname::text AS collation_name,
       co.collprovider::text AS collation_provider,
       co.collisdeterministic AS collation_deterministic,
       COALESCE(pg_catalog.to_jsonb(co)->>'colllocale',
                pg_catalog.to_jsonb(co)->>'colliculocale',co.collcollate) AS collation_locale,
       co.collversion AS collation_version,
       CASE WHEN co.oid IS NOT NULL THEN pg_catalog.pg_collation_actual_version(co.oid) END AS collation_actual_version
FROM pg_catalog.pg_attribute a
JOIN pg_catalog.pg_class t ON t.oid=a.attrelid
JOIN pg_catalog.pg_namespace n ON n.oid=t.relnamespace
JOIN pg_catalog.pg_type ty ON ty.oid=a.atttypid
JOIN pg_catalog.pg_namespace tn ON tn.oid=ty.typnamespace
LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
LEFT JOIN pg_catalog.pg_collation co ON co.oid=a.attcollation
LEFT JOIN pg_catalog.pg_namespace cn ON cn.oid=co.collnamespace
WHERE n.nspname=$1 AND t.relname=$2 AND a.attnum>0 AND NOT a.attisdropped
ORDER BY a.attnum
