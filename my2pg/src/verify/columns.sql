SELECT a.attname, pg_catalog.format_type(a.atttypid,a.atttypmod),
       NOT a.attnotnull, a.attgenerated::text, a.attidentity::text,
       pg_catalog.pg_get_expr(d.adbin,d.adrelid),pg_catalog.col_description(c.oid,a.attnum),a.atttypid
FROM pg_catalog.pg_attribute a
JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
WHERE n.nspname=$1 AND c.relname=$2 AND a.attnum>0 AND NOT a.attisdropped
ORDER BY a.attnum
