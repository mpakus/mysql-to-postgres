SELECT e.enumlabel::text
FROM pg_catalog.pg_enum e
WHERE e.enumtypid=$1
ORDER BY e.enumsortorder
