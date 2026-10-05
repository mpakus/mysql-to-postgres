-- $1 schema, $2 table. One row per index part, including INCLUDE parts.
SELECT i.oid AS index_oid,
       ins.nspname::text AS index_schema,
       i.relname::text AS index_name,
       t.oid AS table_oid,
       ns.nspname::text AS table_schema,
       t.relname::text AS table_name,
       am.amname::text AS method,
       x.indisunique AS is_unique,
       x.indisprimary AS is_primary,
       x.indisexclusion AS is_exclusion,
       x.indisvalid AS valid,
       x.indisready AS ready,
       x.indislive AS live,
       x.indimmediate AS immediate,
       x.indnullsnotdistinct AS nulls_not_distinct,
       pg_catalog.pg_get_expr(x.indpred,x.indrelid) AS predicate,
       con.conname::text AS constraint_name,
       con.condeferrable AS constraint_deferrable,
       con.condeferred AS constraint_initially_deferred,
       k.ordinal AS ordinal,
       k.attnum AS column_ordinal,
       a.attname::text AS column_name,
       CASE WHEN k.attnum=0 THEN pg_catalog.pg_get_indexdef(i.oid,k.ordinal::integer,false) END AS expression,
       k.ordinal>x.indnkeyatts AS included,
       CASE WHEN am.amname='btree' AND k.ordinal<=x.indnkeyatts
            THEN (x.indoption[(k.ordinal-1)::integer]::integer & 1)=1 END AS descending,
       CASE WHEN am.amname='btree' AND k.ordinal<=x.indnkeyatts
            THEN (x.indoption[(k.ordinal-1)::integer]::integer & 2)=2 END AS nulls_first,
       opns.nspname::text AS operator_class_schema,
       opc.oid AS operator_class_oid,
       opc.opcname::text AS operator_class_name,
       opc.opcdefault AS default_operator_class,
       cns.nspname::text AS collation_schema,
       coll.oid AS collation_oid,
       coll.collname::text AS collation_name,
       coll.collprovider::text AS collation_provider,
       coll.collisdeterministic AS collation_deterministic
FROM pg_catalog.pg_index x
JOIN pg_catalog.pg_class i ON i.oid=x.indexrelid
JOIN pg_catalog.pg_namespace ins ON ins.oid=i.relnamespace
JOIN pg_catalog.pg_class t ON t.oid=x.indrelid
JOIN pg_catalog.pg_namespace ns ON ns.oid=t.relnamespace
JOIN pg_catalog.pg_am am ON am.oid=i.relam
CROSS JOIN LATERAL pg_catalog.unnest(x.indkey) WITH ORDINALITY AS k(attnum,ordinal)
LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=t.oid AND a.attnum=k.attnum AND NOT a.attisdropped
LEFT JOIN pg_catalog.pg_opclass opc ON opc.oid=x.indclass[(k.ordinal-1)::integer]
LEFT JOIN pg_catalog.pg_namespace opns ON opns.oid=opc.opcnamespace
LEFT JOIN pg_catalog.pg_collation coll ON coll.oid=x.indcollation[(k.ordinal-1)::integer]
LEFT JOIN pg_catalog.pg_namespace cns ON cns.oid=coll.collnamespace
LEFT JOIN pg_catalog.pg_constraint con ON con.conindid=i.oid AND con.conrelid=t.oid AND con.contype IN ('p','u','x')
WHERE ns.nspname=$1 AND t.relname=$2
ORDER BY i.relname COLLATE "C",k.ordinal
