-- $1 schema, $2 table. Keep every constraint header, even when conkey is empty/NULL.
SELECT con.oid AS constraint_oid,
       con.conname::text AS name,
       con.contype::text AS kind,
       pg_catalog.pg_get_constraintdef(con.oid,false) AS definition,
       CASE WHEN con.contype='c' THEN pg_catalog.pg_get_expr(con.conbin,con.conrelid,false) END AS check_expression,
       con.convalidated AS validated,
       con.condeferrable AS deferrable,
       con.condeferred AS initially_deferred,
       con.connoinherit AS no_inherit,
       con.confmatchtype::text AS match_type,
       con.confupdtype::text AS update_action,
       con.confdeltype::text AS delete_action,
       k.ordinal AS ordinal,
       a.attname::text AS column_name,
       ra.attname::text AS referenced_column,
       r.oid AS referenced_table_oid,
       rn.nspname::text AS referenced_schema,
       r.relname::text AS referenced_table,
       ixns.nspname::text AS supporting_index_schema,
       ix.oid AS supporting_index_oid,
       ix.relname::text AS supporting_index_name,
       CASE WHEN con.confdelsetcols IS NOT NULL THEN
           ARRAY(SELECT sa.attname::text
                 FROM pg_catalog.unnest(con.confdelsetcols) WITH ORDINALITY AS sk(attnum,ordinal)
                 JOIN pg_catalog.pg_attribute sa ON sa.attrelid=t.oid AND sa.attnum=sk.attnum
                 ORDER BY sk.ordinal)
       END AS delete_set_columns,
       pf.oid AS pk_fk_operator_oid,
       pp.oid AS pk_pk_operator_oid,
       ff.oid AS fk_fk_operator_oid,
       CASE WHEN pf.oid IS NOT NULL THEN
           pg_catalog.format('%I.%I(%I.%I,%I.%I)',pfn.nspname,pf.oprname,pfln.nspname,pfl.typname,pfrn.nspname,pfr.typname)
       END AS pk_fk_operator,
       CASE WHEN pp.oid IS NOT NULL THEN
           pg_catalog.format('%I.%I(%I.%I,%I.%I)',ppn.nspname,pp.oprname,ppln.nspname,ppl.typname,pprn.nspname,ppr.typname)
       END AS pk_pk_operator,
       CASE WHEN ff.oid IS NOT NULL THEN
           pg_catalog.format('%I.%I(%I.%I,%I.%I)',ffn.nspname,ff.oprname,ffln.nspname,ffl.typname,ffrn.nspname,ffr.typname)
       END AS fk_fk_operator,
       CASE WHEN k.attnum=0 AND ix.oid IS NOT NULL THEN pg_catalog.pg_get_indexdef(ix.oid,k.ordinal::integer,false) END AS column_expression
FROM pg_catalog.pg_constraint con
JOIN pg_catalog.pg_class t ON t.oid=con.conrelid
JOIN pg_catalog.pg_namespace ns ON ns.oid=t.relnamespace
LEFT JOIN LATERAL pg_catalog.unnest(con.conkey) WITH ORDINALITY AS k(attnum,ordinal) ON true
LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=t.oid AND a.attnum=k.attnum AND NOT a.attisdropped
LEFT JOIN pg_catalog.pg_class r ON r.oid=con.confrelid
LEFT JOIN pg_catalog.pg_namespace rn ON rn.oid=r.relnamespace
LEFT JOIN pg_catalog.pg_attribute ra ON ra.attrelid=r.oid AND ra.attnum=con.confkey[k.ordinal::integer] AND NOT ra.attisdropped
LEFT JOIN pg_catalog.pg_class ix ON ix.oid=con.conindid
LEFT JOIN pg_catalog.pg_namespace ixns ON ixns.oid=ix.relnamespace
LEFT JOIN pg_catalog.pg_operator pf ON pf.oid=con.conpfeqop[k.ordinal::integer]
LEFT JOIN pg_catalog.pg_namespace pfn ON pfn.oid=pf.oprnamespace
LEFT JOIN pg_catalog.pg_type pfl ON pfl.oid=pf.oprleft
LEFT JOIN pg_catalog.pg_namespace pfln ON pfln.oid=pfl.typnamespace
LEFT JOIN pg_catalog.pg_type pfr ON pfr.oid=pf.oprright
LEFT JOIN pg_catalog.pg_namespace pfrn ON pfrn.oid=pfr.typnamespace
LEFT JOIN pg_catalog.pg_operator pp ON pp.oid=con.conppeqop[k.ordinal::integer]
LEFT JOIN pg_catalog.pg_namespace ppn ON ppn.oid=pp.oprnamespace
LEFT JOIN pg_catalog.pg_type ppl ON ppl.oid=pp.oprleft
LEFT JOIN pg_catalog.pg_namespace ppln ON ppln.oid=ppl.typnamespace
LEFT JOIN pg_catalog.pg_type ppr ON ppr.oid=pp.oprright
LEFT JOIN pg_catalog.pg_namespace pprn ON pprn.oid=ppr.typnamespace
LEFT JOIN pg_catalog.pg_operator ff ON ff.oid=con.conffeqop[k.ordinal::integer]
LEFT JOIN pg_catalog.pg_namespace ffn ON ffn.oid=ff.oprnamespace
LEFT JOIN pg_catalog.pg_type ffl ON ffl.oid=ff.oprleft
LEFT JOIN pg_catalog.pg_namespace ffln ON ffln.oid=ffl.typnamespace
LEFT JOIN pg_catalog.pg_type ffr ON ffr.oid=ff.oprright
LEFT JOIN pg_catalog.pg_namespace ffrn ON ffrn.oid=ffr.typnamespace
WHERE ns.nspname=$1 AND t.relname=$2
ORDER BY con.conname COLLATE "C",con.oid,k.ordinal
