//! Required sequence DDL with fresh ownership/state checks and no rewind.
use super::{qualified, quote_ident};
use crate::model::{ColumnPlan, TablePlan};

fn literal(value: &str) -> String {
    format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

/// Runs inside the runner's ordinary required-DDL transaction. The table lock
/// protects rows and table definition, not independent direct sequence users:
/// the exclusive, idle destination contract is still required. setval itself
/// is nontransactional and is never replayed after an uncertain acknowledgement.
pub fn adjustment_sql(table: &TablePlan, column: &ColumnPlan) -> Option<String> {
    let (kind, maximum) = match column.target_type.as_str() {
        "smallint" => ("int2", i64::from(i16::MAX)),
        "integer" => ("int4", i64::from(i32::MAX)),
        "bigint" => ("int8", i64::MAX),
        _ => return None,
    };
    if !column.identity {
        return None;
    }
    let relation = qualified(&table.target_schema, &table.target_name);
    let schema = literal(&table.target_schema);
    let name = literal(&table.target_name);
    let column_name = literal(&column.target_name);
    let column_identifier = quote_ident(&column.target_name);
    let source_next = table.next_auto_increment.unwrap_or(1).max(1);
    let body = format!(
        r#"DECLARE
    col record;
    seq record;
    last_id bigint;
    called boolean;
    loaded numeric;
    current_next numeric;
    required_next numeric;
BEGIN
    LOCK TABLE ONLY {relation} IN ACCESS EXCLUSIVE MODE NOWAIT;
    SELECT c.oid AS table_oid, a.attnum, a.attidentity, a.atttypid
      INTO STRICT col
      FROM pg_catalog.pg_class c
      JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
      JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid
     WHERE n.nspname = {schema} AND c.relname = {name}
       AND c.relkind = 'r' AND c.relpersistence = 'p'
       AND NOT c.relrowsecurity AND NOT c.relforcerowsecurity
       AND a.attname = {column_name} AND a.attnum > 0 AND NOT a.attisdropped;
    IF col.attidentity <> 'd' OR col.atttypid <> 'pg_catalog.{kind}'::regtype THEN
        RAISE EXCEPTION 'sequence column definition changed' USING ERRCODE = '55000';
    END IF;
    SELECT s.*, c.oid, n.nspname, c.relname
      INTO STRICT seq
      FROM pg_catalog.pg_depend d
      JOIN pg_catalog.pg_class c ON c.oid = d.objid AND c.relkind = 'S'
      JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
      JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid
     WHERE d.classid = 'pg_catalog.pg_class'::regclass
       AND d.refclassid = 'pg_catalog.pg_class'::regclass
       AND d.refobjid = col.table_oid AND d.refobjsubid = col.attnum
       AND d.objsubid = 0 AND d.deptype = 'i';
    IF seq.seqtypid <> col.atttypid OR seq.seqincrement <> 1
       OR seq.seqmin <> 1 OR seq.seqmax <> {maximum}
       OR seq.seqcache <> 1 OR seq.seqcycle
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_depend d
                   WHERE d.classid = 'pg_catalog.pg_class'::regclass
                     AND d.objid = seq.oid AND d.deptype = 'e')
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_depend d
                   WHERE d.refclassid = 'pg_catalog.pg_class'::regclass
                     AND d.refobjid = seq.oid) THEN
        RAISE EXCEPTION 'sequence options or dependency scope changed' USING ERRCODE = '55000';
    END IF;
    IF NOT pg_catalog.has_sequence_privilege(seq.oid, 'SELECT')
       OR NOT pg_catalog.has_sequence_privilege(seq.oid, 'UPDATE') THEN
        RAISE EXCEPTION 'sequence SELECT and UPDATE privileges required' USING ERRCODE = '42501';
    END IF;
    EXECUTE pg_catalog.format('SELECT last_value,is_called FROM %I.%I', seq.nspname, seq.relname)
       INTO STRICT last_id, called;
    SELECT MAX({column_identifier})::numeric INTO loaded FROM ONLY {relation};
    current_next := last_id::numeric + CASE WHEN called THEN seq.seqincrement ELSE 0 END;
    required_next := GREATEST(current_next, seq.seqmin::numeric, {source_next}::numeric,
                             COALESCE(loaded + 1, seq.seqmin::numeric));
    IF last_id < seq.seqmin OR last_id > seq.seqmax
       OR current_next > seq.seqmax OR required_next > seq.seqmax THEN
        RAISE EXCEPTION 'sequence generation exhausted' USING ERRCODE = '22003';
    END IF;
    IF required_next > current_next THEN
        PERFORM pg_catalog.setval(seq.oid::regclass, required_next::bigint, false);
    END IF;
END"#
    );
    // A single-quoted body also protects identifiers containing a dollar tag.
    Some(format!("DO {} LANGUAGE plpgsql;", literal(&body)))
}
