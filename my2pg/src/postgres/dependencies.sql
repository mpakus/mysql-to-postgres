SELECT pg_catalog.pg_describe_object(d.classid,d.objid,d.objsubid)
FROM pg_catalog.pg_depend d
JOIN pg_catalog.pg_class referenced ON d.refclassid='pg_catalog.pg_class'::regclass AND d.refobjid=referenced.oid
JOIN pg_catalog.pg_namespace n ON n.oid=referenced.relnamespace
LEFT JOIN pg_catalog.pg_class dependent_relation
  ON d.classid='pg_catalog.pg_class'::regclass AND dependent_relation.oid=d.objid
LEFT JOIN pg_catalog.pg_constraint dependent_constraint
  ON d.classid='pg_catalog.pg_constraint'::regclass AND dependent_constraint.oid=d.objid
LEFT JOIN pg_catalog.pg_class constraint_relation ON constraint_relation.oid=dependent_constraint.conrelid
LEFT JOIN pg_catalog.pg_rewrite dependent_rewrite
  ON d.classid='pg_catalog.pg_rewrite'::regclass AND dependent_rewrite.oid=d.objid
LEFT JOIN pg_catalog.pg_class rewrite_relation ON rewrite_relation.oid=dependent_rewrite.ev_class
LEFT JOIN pg_catalog.pg_attrdef dependent_default
  ON d.classid='pg_catalog.pg_attrdef'::regclass AND dependent_default.oid=d.objid
LEFT JOIN pg_catalog.pg_class default_relation ON default_relation.oid=dependent_default.adrelid
WHERE n.nspname=$1 AND d.deptype='n'
  AND (COALESCE(dependent_relation.relnamespace,constraint_relation.relnamespace,
                rewrite_relation.relnamespace,default_relation.relnamespace) IS NULL
       OR COALESCE(dependent_relation.relnamespace,constraint_relation.relnamespace,
                   rewrite_relation.relnamespace,default_relation.relnamespace)<>n.oid)
ORDER BY 1
