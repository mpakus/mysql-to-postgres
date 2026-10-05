//! Exact row counts and live PostgreSQL catalog verification. Content is a separate scope.
pub mod canonical;
pub mod content;
pub(crate) mod defaults;
use crate::{
    config::{ExistingPolicy, MigrationMode, VerificationMode},
    model::*,
    mysql::{self, SourceConnection},
    postgres::{self, TargetConnection},
};
use defaults::{
    DefaultCatalogContext, DefaultComparison, DeparseSettings, EnumIdentity, IntervalStyle,
    quoted_literal,
};
use mysql_async::prelude::Queryable;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("verification source query failed")]
    Source(#[source] mysql::SourceError),
    #[error("verification target query failed (SQLSTATE {0:?})")]
    Target(Option<String>),
    #[error("verification count metadata is invalid")]
    Count,
    #[error("verification session metadata is invalid")]
    Metadata,
}
impl From<tokio_postgres::Error> for VerifyError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::Target(error.code().map(|code| code.code().to_owned()))
    }
}

pub async fn counts_and_schema(
    source: &mut SourceConnection,
    target: &TargetConnection,
    plan: &MigrationPlan,
    reports: &[TableReport],
    baseline: &BTreeMap<String, u64>,
) -> Result<VerificationReport, VerifyError> {
    let mut result = VerificationReport {
        mode: plan.verification,
        status: VerificationStatus::NotRun,
        tables_checked: 0,
        differences: Vec::new(),
    };
    if plan.verification == VerificationMode::None {
        return Ok(result);
    }
    if plan.verification == VerificationMode::Content {
        result.status = VerificationStatus::Unsupported;
        result.differences.push(
            "complete content verification is unavailable in the counts/schema verifier".into(),
        );
        return Ok(result);
    }
    let session = target
        .client
        .query_one(
            "SELECT pg_catalog.current_setting('extra_float_digits'),pg_catalog.current_setting('IntervalStyle')",
            &[],
        )
        .await?;
    let float_output: String = session.try_get(0)?;
    let interval_style =
        IntervalStyle::parse(&session.try_get::<_, String>(1)?).ok_or(VerifyError::Metadata)?;
    let settings = DeparseSettings {
        exact_float_output: float_output.parse::<i32>().is_ok_and(|value| value >= 1),
    };
    let mut unsupported = Vec::new();
    let by_id: BTreeMap<_, _> = reports.iter().map(|report| (&report.id, report)).collect();
    let selected: BTreeSet<_> = plan.tables.iter().map(|table| &table.id).collect();
    if by_id.len() != reports.len()
        || reports.len() != plan.tables.len()
        || by_id.keys().any(|id| !selected.contains(id))
    {
        result
            .differences
            .push("run accounting does not cover each selected table exactly once".into());
    }
    for table in &plan.tables {
        let relation = postgres::qualified(&table.target_schema, &table.target_name);
        let exists = target.client.query_opt(
            "SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind IN ('r','p')",
            &[&table.target_schema,&table.target_name],
        ).await?;
        if exists.is_none() {
            result.differences.push(format!(
                "{}: target table is missing or not a base table",
                table.id
            ));
            continue;
        }
        let report = by_id.get(&table.id);
        if let Some(report) = report {
            if report.source_name != table.source_name
                || report.target_schema != table.target_schema
                || report.target_name != table.target_name
            {
                result.differences.push(format!(
                    "{}: run accounting identity differs from the plan",
                    table.id
                ));
            }
            if report.accounted_rows() != Some(report.rows_read)
                || report.unresolved_rows != 0
                || report.indeterminate_rows != 0
            {
                result
                    .differences
                    .push(format!("{}: source row accounting is incomplete", table.id));
            }
            if plan.mode == MigrationMode::SchemaOnly {
                if report.rows_read != 0 || report.committed_rows != 0 || report.rejected_rows != 0
                {
                    result
                        .differences
                        .push(format!("{}: schema_only recorded data activity", table.id));
                }
            } else {
                let sql = format!(
                    "SELECT COUNT(*) FROM {}.{}",
                    mysql::quote_ident(&plan.source_database),
                    mysql::quote_ident(&table.source_name)
                );
                let source_count: Option<u64> = source
                    .query_first(sql)
                    .await
                    .map_err(|error| VerifyError::Source(error.into()))?;
                let source_count = source_count.ok_or(VerifyError::Count)?;
                if source_count != report.rows_read {
                    result.differences.push(format!(
                        "{}: source count differs from recorded read rows",
                        table.id
                    ));
                }
                let target_count: i64 = target
                    .client
                    .query_one(&format!("SELECT COUNT(*) FROM {relation}"), &[])
                    .await?
                    .try_get(0)
                    .map_err(|_| VerifyError::Count)?;
                let target_count = u64::try_from(target_count).map_err(|_| VerifyError::Count)?;
                let initial = if plan.on_existing == ExistingPolicy::Append {
                    match baseline.get(&table.id) {
                        Some(initial) => Some(*initial),
                        None => {
                            unsupported.push(format!(
                                "{}: append target baseline was not captured",
                                table.id
                            ));
                            None
                        }
                    }
                } else {
                    Some(0)
                };
                if let Some(initial) = initial {
                    match initial.checked_add(report.committed_rows) {
                        Some(expected) if expected == target_count => (),
                        Some(_) => result.differences.push(format!(
                            "{}: target count differs from baseline plus committed rows",
                            table.id
                        )),
                        None => result
                            .differences
                            .push(format!("{}: target count expectation overflow", table.id)),
                    }
                }
            }
        } else {
            result
                .differences
                .push(format!("{}: missing run accounting", table.id));
        }
        verify_columns(
            target,
            table,
            &settings,
            interval_style,
            &mut result.differences,
            &mut unsupported,
        )
        .await?;
        verify_primary_key(target, table, &mut result.differences, &mut unsupported).await?;
        verify_validity(target, table, &mut result.differences).await?;
        verify_structure(target, table, &mut result.differences, &mut unsupported).await?;
        verify_sequences(
            target,
            table,
            plan.reset_sequences,
            &mut result.differences,
            &mut unsupported,
        )
        .await?;
        if !table.structure.complete {
            unsupported.push(format!(
                "{}: complete schema expectations were not provided",
                table.id
            ));
        }
        result.tables_checked += 1;
    }
    result.status = if !result.differences.is_empty() {
        VerificationStatus::Different
    } else if !unsupported.is_empty() {
        VerificationStatus::Unsupported
    } else {
        VerificationStatus::Complete
    };
    result.differences.extend(unsupported);
    Ok(result)
}

async fn verify_columns(
    target: &TargetConnection,
    table: &TablePlan,
    settings: &DeparseSettings,
    interval_style: IntervalStyle,
    differences: &mut Vec<String>,
    unsupported: &mut Vec<String>,
) -> Result<(), VerifyError> {
    let rows = target
        .client
        .query(
            include_str!("columns.sql"),
            &[&table.target_schema, &table.target_name],
        )
        .await?;
    if rows.len() != table.columns.len() {
        differences.push(format!(
            "{}: target column count differs from the plan",
            table.id
        ));
    }
    for (expected, actual) in table.columns.iter().zip(rows.iter()) {
        let name: String = actual.try_get(0)?;
        let kind: String = actual.try_get(1)?;
        let nullable: bool = actual.try_get(2)?;
        let generated: String = actual.try_get(3)?;
        let identity: String = actual.try_get(4)?;
        let default: Option<String> = actual.try_get(5)?;
        let comment: Option<String> = actual.try_get(6)?;
        let column_oid: u32 = actual.try_get(7)?;
        let enum_identity = if expected.kind == ValueKind::Enum {
            let expected_oid: Option<u32> =
                if let Some(expected_type) = defaults::type_identifier(&expected.target_type) {
                    target
                        .client
                        .query_one("SELECT pg_catalog.to_regtype($1)::oid", &[&expected_type])
                        .await?
                        .try_get(0)?
                } else {
                    None
                };
            let cast_oid = if let Some(cast) = default.as_deref().and_then(defaults::enum_cast) {
                target
                    .client
                    .query_one("SELECT pg_catalog.to_regtype($1)::oid", &[&cast])
                    .await?
                    .try_get(0)?
            } else {
                None
            };
            expected_oid.map(|expected_oid| EnumIdentity {
                expected_oid,
                column_oid,
                cast_oid,
            })
        } else {
            None
        };
        let type_matches = if expected.kind == ValueKind::Enum {
            enum_identity
                .as_ref()
                .is_some_and(|identity| identity.expected_oid == column_oid)
        } else {
            crate::types::equivalent_postgres_types(&kind, &expected.target_type)
        };
        if expected.kind == ValueKind::Enum {
            if expected.enum_labels.is_empty() {
                unsupported.push(format!(
                    "{}: complete ordered ENUM label expectations are missing for {}",
                    table.id, expected.target_name
                ));
            } else {
                let labels = target
                    .client
                    .query(include_str!("enum-labels.sql"), &[&column_oid])
                    .await?
                    .iter()
                    .map(|row| row.try_get::<_, String>(0))
                    .collect::<Result<Vec<_>, _>>()?;
                if labels != expected.enum_labels {
                    differences.push(format!(
                        "{}: ordered ENUM labels differ for {}",
                        table.id, expected.target_name
                    ));
                }
            }
        }
        let context = DefaultCatalogContext {
            interval_style,
            target_type_proven: type_matches,
            enum_identity,
        };
        if name != expected.target_name
            || !type_matches
            || nullable != expected.nullable
            || !generated.is_empty() != expected.generated_expression.is_some()
            || !identity.is_empty() != expected.identity
        {
            differences.push(format!(
                "{}: ordered column type/nullability/generation/identity differs for {}",
                table.id, expected.target_name
            ));
        }
        if let Some(expected_expression) = expected.generated_expression.as_deref()
            && !generated_expression_matches(Some(expected_expression), default.as_deref())
        {
            differences.push(format!(
                "{}: generated expression differs for {}",
                table.id, expected.target_name
            ));
        }
        if let Some(expected_comment) = &expected.comment
            && comment.as_deref().unwrap_or("") != expected_comment
        {
            differences.push(format!(
                "{}: column comment differs for {}",
                table.id, expected.target_name
            ));
        }
        if !generated.is_empty() {
            continue;
        }
        match (&expected.default_sql, &default) {
            (None, None) => (),
            (Some(_), Some(actual)) => match if matches!(
                expected.kind,
                ValueKind::Enum | ValueKind::Interval | ValueKind::Datetime | ValueKind::Timestamp
            ) {
                defaults::compare_with_catalog(expected, actual, settings, &context)
            } else {
                defaults::compare(expected, actual, settings)
            } {
                DefaultComparison::Equal => (),
                DefaultComparison::Different => differences.push(format!(
                    "{}: typed default literal differs for {}",
                    table.id, expected.target_name
                )),
                DefaultComparison::Unsupported => unsupported.push(format!(
                    "{}: default expression equivalence cannot yet be proven for {}",
                    table.id, expected.target_name
                )),
            },
            _ => differences.push(format!(
                "{}: default presence differs for {}",
                table.id, expected.target_name
            )),
        }
    }
    Ok(())
}

/// Require an exact catalog deparse match for explicitly planned PG expressions.
fn generated_expression_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    expected.is_some() && expected == actual
}

/// Decode only the complete built-in SET membership predicate; never evaluate SQL.
pub(crate) fn set_membership_matches(
    expression: &str,
    expected: &SetMembershipExpectation,
) -> Option<bool> {
    let expression = expression.trim();
    let expression = expression
        .strip_prefix('(')
        .and_then(|text| text.strip_suffix(')'))
        .unwrap_or(expression)
        .trim();
    let (column, mut tail) = if let Some(quoted) = expression.strip_prefix('"') {
        let mut name = String::new();
        let mut chars = quoted.char_indices().peekable();
        let remainder = loop {
            let (position, ch) = chars.next()?;
            if ch == '"' {
                if chars.peek().is_some_and(|(_, next)| *next == '"') {
                    chars.next();
                    name.push('"');
                } else {
                    break &quoted[position + 1..];
                }
            } else {
                name.push(ch);
            }
        };
        (name, remainder)
    } else {
        let length = expression
            .find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '$')))
            .unwrap_or(expression.len());
        let name = &expression[..length];
        if !name.starts_with(|ch: char| ch.is_alphabetic() || ch == '_') {
            return None;
        }
        (name.to_owned(), &expression[length..])
    };
    tail = tail.trim_start().strip_prefix("<@")?.trim_start();
    tail = tail.strip_prefix("ARRAY[")?.trim_start();
    let mut labels = BTreeSet::new();
    if !tail.starts_with(']') {
        loop {
            let (label, rest) = quoted_literal(tail)?;
            labels.insert(label);
            tail = rest.trim_start();
            if let Some(rest) = tail.strip_prefix("::text") {
                tail = rest.trim_start();
            } else if let Some(rest) = tail.strip_prefix("::pg_catalog.text") {
                tail = rest.trim_start();
            }
            if let Some(rest) = tail.strip_prefix(',') {
                tail = rest.trim_start();
            } else {
                break;
            }
        }
    }
    tail = tail.strip_prefix(']')?.trim();
    tail = tail
        .strip_prefix("::text[]")
        .or_else(|| tail.strip_prefix("::pg_catalog.text[]"))
        .unwrap_or(tail)
        .trim();
    if !tail.is_empty() {
        return None;
    }
    Some(column == expected.column && labels == expected.labels.iter().cloned().collect())
}

async fn verify_structure(
    target: &TargetConnection,
    table: &TablePlan,
    differences: &mut Vec<String>,
    unsupported: &mut Vec<String>,
) -> Result<(), VerifyError> {
    for expected in &table.structure.indexes {
        let rows = target
            .client
            .query(
                include_str!("indexes.sql"),
                &[&table.target_schema, &table.target_name, &expected.name],
            )
            .await?;
        let columns: Vec<String> = rows
            .iter()
            .filter_map(|row| row.get::<_, Option<String>>(0))
            .collect();
        let descending: Vec<bool> = rows.iter().map(|row| row.get(3)).collect();
        let expressions: Vec<Option<String>> = rows.iter().map(|row| row.get(6)).collect();
        let correct = !rows.is_empty()
            && columns == expected.columns
            && expressions == expected.expressions
            && descending == expected.descending
            && rows.iter().all(|row| {
                row.get::<_, bool>(1) == expected.unique
                    && row.get::<_, bool>(2) == expected.primary
                    && row.get::<_, String>(4) == "btree"
                    && row.get::<_, bool>(5)
            });
        if !correct {
            differences.push(format!(
                "{}: expected index definition/validity differs for {}",
                table.id, expected.name
            ));
        }
    }
    for expected in &table.structure.foreign_keys {
        let rows = target
            .client
            .query(
                include_str!("foreign-keys.sql"),
                &[&table.target_schema, &table.target_name, &expected.name],
            )
            .await?;
        let columns: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
        let referenced: Vec<String> = rows.iter().map(|row| row.get(3)).collect();
        if rows.is_empty()
            || columns != expected.columns
            || referenced != expected.referenced_columns
            || rows.iter().any(|row| {
                row.get::<_, String>(1) != expected.referenced_schema
                    || row.get::<_, String>(2) != expected.referenced_table
                    || fk_action(&row.get::<_, String>(4)) != expected.on_update
                    || fk_action(&row.get::<_, String>(5)) != expected.on_delete
                    || !row.get::<_, bool>(6)
            })
        {
            differences.push(format!(
                "{}: expected foreign key differs for {}",
                table.id, expected.name
            ));
        }
    }
    for expected in &table.structure.checks {
        let row=target.client.query_opt("SELECT pg_get_expr(con.conbin,con.conrelid),con.convalidated FROM pg_catalog.pg_constraint con JOIN pg_catalog.pg_class c ON c.oid=con.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 AND con.conname=$3 AND con.contype='c'",&[&table.target_schema,&table.target_name,&expected.name]).await?;
        if let (Some(row), Some(membership)) = (&row, &expected.set_membership)
            && row.get::<_, bool>(1)
        {
            match set_membership_matches(&row.get::<_, String>(0), membership) {
                Some(true) => (),
                Some(false) => differences.push(format!(
                    "{}: SET membership CHECK differs for {}",
                    table.id, expected.name
                )),
                None => unsupported.push(format!(
                    "{}: SET membership CHECK cannot be decoded for {}",
                    table.id, expected.name
                )),
            }
            continue;
        }
        match row {
            Some(row)
                if row.get::<_, String>(0) == expected.expression && row.get::<_, bool>(1) => {}
            Some(row) if row.get::<_, bool>(1) => unsupported.push(format!(
                "{}: CHECK expression equivalence cannot yet be proven for {}",
                table.id, expected.name
            )),
            _ => differences.push(format!(
                "{}: expected CHECK is missing or unvalidated for {}",
                table.id, expected.name
            )),
        }
    }
    if let Some(comment) = &table.structure.comment {
        let actual:Option<String>=target.client.query_one("SELECT obj_description(c.oid,'pg_class') FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2",&[&table.target_schema,&table.target_name]).await?.get(0);
        if actual.as_deref().unwrap_or("") != comment {
            differences.push(format!("{}: table comment differs", table.id));
        }
    }
    Ok(())
}
fn fk_action(value: &str) -> &str {
    match value {
        "a" => "NO ACTION",
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "UNKNOWN",
    }
}

async fn verify_sequences(
    target: &TargetConnection,
    table: &TablePlan,
    reset_sequences: bool,
    differences: &mut Vec<String>,
    unsupported: &mut Vec<String>,
) -> Result<(), VerifyError> {
    for expectation in &table.structure.sequences {
        if !table
            .columns
            .iter()
            .any(|column| column.identity && column.target_name == expectation.column)
        {
            unsupported.push(format!(
                "{}: sequence expectation has no planned identity column {}",
                table.id, expectation.column
            ));
        }
    }
    for column in table.columns.iter().filter(|column| column.identity) {
        let expectations: Vec<_> = table
            .structure
            .sequences
            .iter()
            .filter(|expected| expected.column == column.target_name)
            .collect();
        let expected = match expectations.as_slice() {
            [expected] => Some(*expected),
            _ => {
                unsupported.push(format!(
                    "{}: exactly one explicit sequence generation expectation is required for {}",
                    table.id, column.target_name
                ));
                None
            }
        };
        let relation = postgres::qualified(&table.target_schema, &table.target_name);
        let name: Option<String> = target
            .client
            .query_one(
                "SELECT pg_get_serial_sequence($1,$2)",
                &[&relation, &column.target_name],
            )
            .await?
            .get(0);
        let Some(name) = name else {
            differences.push(format!(
                "{}: identity sequence is missing for {}",
                table.id, column.target_name
            ));
            continue;
        };
        let row=target.client.query_one("SELECT n.nspname,c.relname,s.seqincrement,s.seqmin,s.seqmax,s.seqcycle,s.seqcache FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_sequence s ON s.seqrelid=c.oid WHERE c.oid=$1::text::regclass",&[&name]).await?;
        let schema: String = row.get(0);
        let sequence: String = row.get(1);
        let actual_increment: i64 = row.try_get(2)?;
        let actual_minimum: i64 = row.try_get(3)?;
        let actual_maximum: i64 = row.try_get(4)?;
        let actual_cycle: bool = row.try_get(5)?;
        let cache: i64 = row.try_get(6)?;
        let increment = i128::from(actual_increment);
        let minimum = i128::from(actual_minimum);
        let maximum = i128::from(actual_maximum);
        if let Some(expected) = expected {
            if actual_increment != expected.increment
                || actual_minimum != expected.min_value
                || actual_maximum != expected.max_value
                || actual_cycle != expected.cycle
            {
                differences.push(format!(
                    "{}: identity sequence increment/range/cycle differs for {}",
                    table.id, column.target_name
                ));
            }
            if expected.increment != 1 || expected.cycle || expected.min_value > expected.max_value
            {
                unsupported.push(format!(
                    "{}: nonunit/cyclic/invalid sequence expectations are unsupported for {}",
                    table.id, column.target_name
                ));
            }
        }
        if cache != 1 {
            unsupported.push(format!(
                "{}: cached sequence next-value state cannot be certified for {}",
                table.id, column.target_name
            ));
        }
        let state = target
            .client
            .query_one(
                &format!(
                    "SELECT last_value::text,is_called FROM {}",
                    postgres::qualified(&schema, &sequence)
                ),
                &[],
            )
            .await?;
        let last: i128 = state
            .get::<_, String>(0)
            .parse()
            .map_err(|_| VerifyError::Count)?;
        let called: bool = state.get(1);
        let next = if called {
            last.checked_add(increment).ok_or(VerifyError::Count)?
        } else {
            last
        };
        let loaded: Option<String> = target
            .client
            .query_one(
                &format!(
                    "SELECT MAX({})::text FROM {relation}",
                    postgres::quote_ident(&column.target_name)
                ),
                &[],
            )
            .await?
            .get(0);
        let loaded = loaded
            .map(|value| value.parse::<i128>())
            .transpose()
            .map_err(|_| VerifyError::Count)?;
        if !reset_sequences {
            match expected.and_then(|expected| expected.preserved_state.as_ref()) {
                Some(SequenceStateObservation::Known {
                    last_value,
                    is_called,
                }) => {
                    if last != i128::from(*last_value) || called != *is_called {
                        differences.push(format!(
                            "{}: preserved sequence state changed for {}",
                            table.id, column.target_name
                        ));
                    }
                }
                _ => unsupported.push(format!(
                    "{}: explicit preserved sequence state is unavailable for {}",
                    table.id, column.target_name
                )),
            }
            if next < minimum || next > maximum || increment <= 0 {
                differences.push(format!(
                    "{}: preserved sequence state is outside its generation range for {}",
                    table.id, column.target_name
                ));
            }
            continue;
        }
        if expected.is_some_and(|expected| expected.preserved_state.is_some()) {
            unsupported.push(format!(
                "{}: reset and preserved sequence expectations contradict for {}",
                table.id, column.target_name
            ));
        }
        let lower = i128::from(table.next_auto_increment.unwrap_or(1))
            .max(i128::from(
                expected.map_or(1, |expected| expected.next_minimum),
            ))
            .max(loaded.map_or(1, |value| value + 1));
        if next < lower || next < minimum || next > maximum || lower > maximum || increment <= 0 {
            differences.push(format!(
                "{}: identity next value is outside its required lower bound/range for {}",
                table.id, column.target_name
            ));
        }
    }
    Ok(())
}

async fn verify_primary_key(
    target: &TargetConnection,
    table: &TablePlan,
    differences: &mut Vec<String>,
    unsupported: &mut Vec<String>,
) -> Result<(), VerifyError> {
    if !table.structure.complete {
        unsupported.push(format!(
            "{}: complete target primary-key expectations were not provided",
            table.id
        ));
        return Ok(());
    }
    let primary: Vec<_> = table
        .structure
        .indexes
        .iter()
        .filter(|index| index.primary)
        .collect();
    let expected = match primary.as_slice() {
        [] => Vec::new(),
        [index] => index.columns.clone(),
        _ => {
            unsupported.push(format!(
                "{}: target primary-key expectations are ambiguous",
                table.id
            ));
            return Ok(());
        }
    };
    let rows = target
        .client
        .query(
            include_str!("primary-key.sql"),
            &[&table.target_schema, &table.target_name],
        )
        .await?;
    let columns: Vec<String> = rows.iter().map(|row| row.get(0)).collect();
    if expected != columns {
        differences.push(format!("{}: primary key ordered columns differ", table.id));
    }
    Ok(())
}

async fn verify_validity(
    target: &TargetConnection,
    table: &TablePlan,
    differences: &mut Vec<String>,
) -> Result<(), VerifyError> {
    let rows = target
        .client
        .query(
            include_str!("validity.sql"),
            &[&table.target_schema, &table.target_name],
        )
        .await?;
    if !rows.is_empty() {
        differences.push(format!(
            "{}: target has an invalid index or unvalidated constraint",
            table.id
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_expression_requires_an_exact_observed_match() {
        assert!(generated_expression_matches(
            Some("lower(source_text)"),
            Some("lower(source_text)")
        ));
        assert!(!generated_expression_matches(
            Some("lower(source_text)"),
            Some("lower((source_text)::text)")
        ));
        assert!(!generated_expression_matches(
            Some("lower(source_text)"),
            None
        ));
    }

    #[test]
    fn set_membership_decodes_only_complete_typed_predicates() {
        let expected = SetMembershipExpectation {
            column: "a\"b".into(),
            labels: vec![
                "".into(),
                "a,b".into(),
                "O'Reilly".into(),
                "back\\slash".into(),
                "Ω".into(),
            ],
        };
        let actual = r#"("a""b" <@ ARRAY[''::text, 'a,b'::text, 'O''Reilly'::text, E'back\\slash'::text, 'Ω'::text])"#;
        assert_eq!(set_membership_matches(actual, &expected), Some(true));
        let changed = SetMembershipExpectation {
            column: expected.column.clone(),
            labels: vec!["different".into()],
        };
        assert_eq!(set_membership_matches(actual, &changed), Some(false));
        for expression in [
            r#"("a""b" <@ ARRAY['x'] OR true)"#,
            r#"("a""b" <@ ARRAY[lower('x')])"#,
            r#"("a""b" <@ ARRAY['x'::custom_type])"#,
            r#"("a""b" <@ ARRAY['x']); SELECT 1"#,
            r#"("a""b" <@ ARRAY['x',])"#,
        ] {
            assert_eq!(set_membership_matches(expression, &expected), None);
        }
        assert_eq!(
            set_membership_matches(
                "(membership <@ ARRAY[]::text[])",
                &SetMembershipExpectation {
                    column: "membership".into(),
                    labels: vec![]
                }
            ),
            Some(true)
        );
    }
    #[test]
    fn type_aliases_preserve_modifiers_and_timestamp_timezone() {
        use crate::types::normalize_postgres_type as normalized_type;
        assert_ne!(normalized_type("\"State\""), normalized_type("\"state\""));
        for (expected, actual) in [
            ("varchar(40)", "character varying(40)"),
            ("bool[]", "boolean[]"),
            ("numeric(65,30)", "numeric(65, 30)"),
            ("timestamp(6)", "timestamp(6) without time zone"),
            ("timestamptz(6)", "timestamp(6) with time zone"),
            ("int", "integer"),
        ] {
            assert_eq!(normalized_type(expected), normalized_type(actual));
        }
        assert_ne!(
            normalized_type("timestamp(6)"),
            normalized_type("timestamptz(6)")
        );
        assert_ne!(
            normalized_type("numeric(65,30)"),
            normalized_type("numeric(65,29)")
        );
    }
}
