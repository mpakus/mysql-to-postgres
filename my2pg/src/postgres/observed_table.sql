-- $1 exact schema, $2 exact table; no row means a changed/missing relation.
SELECT c.oid,
       pg_catalog.pg_get_userbyid(c.relowner)::text AS owner,
       c.relkind::text AS relation_kind,
       am.amname::text AS access_method,
       c.relpersistence::text AS persistence,
       pg_catalog.obj_description(c.oid,'pg_class') AS comment,
       c.relrowsecurity AS row_security_enabled,
       c.relforcerowsecurity AS row_security_forced,
       c.relreplident::text AS replica_identity
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
LEFT JOIN pg_catalog.pg_am am ON am.oid=c.relam
WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind IN ('r','p')
