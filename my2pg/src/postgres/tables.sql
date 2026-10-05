SELECT c.relname, has_table_privilege(c.oid,'INSERT'), has_table_privilege(c.oid,'SELECT'),
       pg_has_role(c.relowner,'USAGE'), has_table_privilege(c.oid,'TRUNCATE'),
       row_security_active(c.oid),
       c.relkind='r' AND NOT c.relispartition AND NOT EXISTS (
           SELECT 1 FROM pg_catalog.pg_inherits i WHERE i.inhrelid=c.oid OR i.inhparent=c.oid
       ),
       pg_has_role(c.relowner,'USAGE') OR has_table_privilege(c.oid,'UPDATE,DELETE,TRUNCATE') AS can_lock
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
WHERE n.nspname=$1 AND c.relkind IN ('r','p') ORDER BY c.relname COLLATE "C"
