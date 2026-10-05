SELECT 'foreign_key'::text AS kind, con.conname::text AS name,
       depns.nspname::text AS dependent_schema, dep.relname::text AS dependent_table,
       refns.nspname::text AS referenced_schema, ref.relname::text AS referenced_table
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class dep ON dep.oid=con.conrelid
JOIN pg_catalog.pg_namespace depns ON depns.oid=dep.relnamespace
JOIN pg_catalog.pg_class ref ON ref.oid=con.confrelid
JOIN pg_catalog.pg_namespace refns ON refns.oid=ref.relnamespace
WHERE con.contype='f' AND refns.nspname=$1
UNION ALL
SELECT DISTINCT 'view', own.relname::text, ownns.nspname::text, own.relname::text,
       refns.nspname::text, ref.relname::text
FROM pg_catalog.pg_rewrite rewrite
JOIN pg_catalog.pg_depend dep ON dep.classid='pg_catalog.pg_rewrite'::regclass AND dep.objid=rewrite.oid
JOIN pg_catalog.pg_class own ON own.oid=rewrite.ev_class
JOIN pg_catalog.pg_namespace ownns ON ownns.oid=own.relnamespace
JOIN pg_catalog.pg_class ref ON dep.refclassid='pg_catalog.pg_class'::regclass AND ref.oid=dep.refobjid
JOIN pg_catalog.pg_namespace refns ON refns.oid=ref.relnamespace
WHERE own.relkind IN ('v','m') AND refns.nspname=$1 AND ref.oid<>own.oid
