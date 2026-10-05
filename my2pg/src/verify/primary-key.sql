SELECT a.attname
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid=i.indrelid
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum,position)
JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum=k.attnum
WHERE n.nspname=$1 AND c.relname=$2 AND i.indisprimary AND k.position<=i.indnkeyatts
ORDER BY k.position
