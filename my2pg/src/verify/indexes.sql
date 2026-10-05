SELECT a.attname,i.indisunique,i.indisprimary,((i.indoption[(k.position-1)::integer]::integer & 1)=1),
       am.amname,i.indisvalid AND i.indisready AND i.indimmediate AND NOT i.indnullsnotdistinct
       AND i.indpred IS NULL AND i.indnatts=i.indnkeyatts,
       CASE WHEN k.attnum=0 THEN pg_catalog.pg_get_indexdef(i.indexrelid,k.position::integer,false) END
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid=i.indrelid
JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_class ic ON ic.oid=i.indexrelid
JOIN pg_catalog.pg_am am ON am.oid=ic.relam
CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum,position)
LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum=k.attnum AND NOT a.attisdropped
WHERE n.nspname=$1 AND c.relname=$2 AND ic.relname=$3 AND k.position<=i.indnkeyatts
ORDER BY k.position
