SELECT ic.relname
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid=i.indrelid
JOIN pg_catalog.pg_class ic ON ic.oid=i.indexrelid
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
WHERE n.nspname=$1 AND c.relname=$2 AND (NOT i.indisvalid OR NOT i.indisready)
UNION ALL
SELECT con.conname
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class c ON c.oid=con.conrelid
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
WHERE n.nspname=$1 AND c.relname=$2 AND NOT con.convalidated
