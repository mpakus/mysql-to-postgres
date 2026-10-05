SELECT a.attname,rn.nspname,rc.relname,ra.attname,con.confupdtype::text,con.confdeltype::text,
       con.convalidated AND NOT con.condeferrable AND NOT con.condeferred
       AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_trigger t WHERE t.tgconstraint=con.oid AND t.tgenabled<>'O')
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class c ON c.oid=con.conrelid
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_class rc ON rc.oid=con.confrelid
JOIN pg_catalog.pg_namespace rn ON rn.oid=rc.relnamespace
CROSS JOIN LATERAL unnest(con.conkey,con.confkey) WITH ORDINALITY AS k(local,remote,position)
JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum=k.local
JOIN pg_catalog.pg_attribute ra ON ra.attrelid=rc.oid AND ra.attnum=k.remote
WHERE n.nspname=$1 AND c.relname=$2 AND con.conname=$3 AND con.contype='f'
ORDER BY k.position
