SELECT t.typname,e.enumlabel
FROM pg_catalog.pg_type t
JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace
JOIN pg_catalog.pg_enum e ON e.enumtypid=t.oid
WHERE n.nspname=$1 ORDER BY t.typname COLLATE "C",e.enumsortorder
