//! Deterministic, read-only planning. Catalogs are data; this module never opens a database.
use crate::{config::*, model::*, pipeline::hooks};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
mod existing;
mod labels;

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("invalid planning configuration: {0}")]
    Config(String),
    #[error("migration blocked by required planning diagnostics")]
    Blocked(Vec<Diagnostic>),
}

pub fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
pub fn qualified_name(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(name))
}
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}
fn digest(identity: &str) -> String {
    format!("{:x}", Sha256::digest(identity.as_bytes()))
}

/// PostgreSQL identifiers have a 63-byte ceiling, not a 63-character ceiling.
pub fn stable_name(name: &str, identity: &str) -> String {
    if name.len() <= 63 {
        return name.to_owned();
    }
    let mut end = 54;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}_{}", &name[..end], &digest(identity)[..8])
}

fn mapped_name(name: &str, policy: IdentifierPolicy, identity: &str) -> String {
    let mapped = match policy {
        IdentifierPolicy::Preserve => name.to_owned(),
        IdentifierPolicy::Downcase => name.to_lowercase(),
        IdentifierPolicy::SnakeCase => {
            let mut result = String::new();
            let chars: Vec<_> = name.chars().collect();
            for (index, ch) in chars.iter().enumerate() {
                if ch.is_uppercase()
                    && index > 0
                    && (chars[index - 1].is_lowercase()
                        || chars[index - 1].is_numeric()
                        || chars.get(index + 1).is_some_and(|next| next.is_lowercase()))
                {
                    result.push('_');
                }
                result.extend(ch.to_lowercase());
            }
            result
        }
    };
    stable_name(&mapped, identity)
}

fn diagnostic(code: &str, object: &str, message: &str, severity: Severity) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        stage: "plan".into(),
        object: Some(object.into()),
        severity,
        message: message.into(),
    }
}

fn override_for<'a>(config: &'a MigrationConfig, object: &str) -> Option<&'a ObjectOverride> {
    config.overrides.iter().find(|rule| rule.object == object)
}
fn omitted(config: &MigrationConfig, object: &str) -> bool {
    override_for(config, object).is_some_and(|rule| rule.omit)
}
fn unsigned(column: &SourceColumn) -> bool {
    column.column_type.to_ascii_lowercase().contains("unsigned")
}
fn auto(column: &SourceColumn) -> bool {
    column.extra.to_ascii_lowercase().contains("auto_increment")
}
fn sequence_max(target_type: &str) -> Option<i64> {
    match target_type {
        "smallint" => Some(i16::MAX as i64),
        "integer" | "int" => Some(i32::MAX as i64),
        "bigint" => Some(i64::MAX),
        _ => None,
    }
}
fn compare(guard: &NumericGuard, value: Option<u32>) -> bool {
    value.is_some_and(|value| match guard.op {
        Comparison::Eq => value == guard.value,
        Comparison::Lt => value < guard.value,
        Comparison::Le => value <= guard.value,
        Comparison::Gt => value > guard.value,
        Comparison::Ge => value >= guard.value,
    })
}
fn matches(rule: &CastRule, table: &str, column: &SourceColumn) -> bool {
    rule.source_type
        .as_ref()
        .is_none_or(|kind| source_type_name(kind) == source_type_name(&column.data_type))
        && rule.source_table.as_ref().is_none_or(|name| name == table)
        && rule
            .source_column
            .as_ref()
            .is_none_or(|name| name == &column.name)
        && rule.unsigned.is_none_or(|value| value == unsigned(column))
        && rule.not_null.is_none_or(|value| value != column.nullable)
        && rule
            .auto_increment
            .is_none_or(|value| value == auto(column))
        && rule
            .default
            .as_ref()
            .is_none_or(|value| Some(value) == column.default.as_ref())
        && rule
            .precision
            .as_ref()
            .is_none_or(|guard| compare(guard, column.numeric_precision))
        && rule
            .scale
            .as_ref()
            .is_none_or(|guard| compare(guard, column.numeric_scale))
}

fn basic_type(column: &SourceColumn) -> Option<(String, ValueKind)> {
    let is_unsigned = unsigned(column);
    let integer = if is_unsigned {
        ValueKind::UnsignedInteger
    } else {
        ValueKind::SignedInteger
    };
    let temporal = column.datetime_precision.unwrap_or(0);
    let length = column.character_length.unwrap_or(0);
    let result = match column.data_type.to_ascii_lowercase().as_str() {
        "tinyint" => ("smallint".into(), integer),
        "smallint" => (
            if is_unsigned { "integer" } else { "smallint" }.into(),
            integer,
        ),
        "mediumint" => ("integer".into(), integer),
        "int" | "integer" => (
            if is_unsigned { "bigint" } else { "integer" }.into(),
            integer,
        ),
        "bigint" => (
            if is_unsigned {
                "numeric(20,0)"
            } else {
                "bigint"
            }
            .into(),
            integer,
        ),
        "decimal" | "numeric" => {
            let precision = column.numeric_precision?;
            let scale = column.numeric_scale?;
            if !(1..=65).contains(&precision) || scale > 30 || scale > precision {
                return None;
            }
            (format!("numeric({precision},{scale})"), ValueKind::Decimal)
        }
        "float" => ("real".into(), ValueKind::Float),
        "double" | "real" => ("double precision".into(), ValueKind::Float),
        "char" | "varchar" if length > 0 => (format!("varchar({length})"), ValueKind::Text),
        "tinytext" | "text" | "mediumtext" | "longtext" => ("text".into(), ValueKind::Text),
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => {
            ("bytea".into(), ValueKind::Binary)
        }
        "date" => ("date".into(), ValueKind::Date),
        "datetime" if temporal <= 6 => (
            format!("timestamp({temporal}) without time zone"),
            ValueKind::Datetime,
        ),
        "timestamp" if temporal <= 6 => (format!("timestamptz({temporal})"), ValueKind::Timestamp),
        "time" if temporal <= 6 => (format!("interval({temporal})"), ValueKind::Interval),
        "year" => ("smallint".into(), ValueKind::UnsignedInteger),
        "bit" => {
            let width: u16 = column
                .column_type
                .split_once('(')?
                .1
                .split_once(')')?
                .0
                .parse()
                .ok()?;
            if !(1..=64).contains(&width) {
                return None;
            }
            (format!("bit({width})"), ValueKind::Bit(width))
        }
        "json" => ("jsonb".into(), ValueKind::Json),
        "enum" => ("enum".into(), ValueKind::Enum),
        "set" => ("text[]".into(), ValueKind::Set),
        "geometry" | "point" | "linestring" | "polygon" | "multipoint" | "multilinestring"
        | "multipolygon" | "geometrycollection" => ("geometry".into(), ValueKind::Geometry),
        _ => return None,
    };
    Some(result)
}

fn source_type_name(kind: &str) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "integer" => "int".into(),
        "numeric" | "dec" => "decimal".into(),
        "double precision" => "double".into(),
        other => other.into(),
    }
}

fn override_type(kind: &str) -> Option<ValueKind> {
    let kind = kind.trim().to_ascii_lowercase();
    let base = kind.split('(').next().unwrap_or(&kind).trim();
    Some(match base {
        "smallint" | "integer" | "bigint" => ValueKind::SignedInteger,
        "numeric" | "decimal" => ValueKind::Decimal,
        "real" | "double precision" => ValueKind::Float,
        "text" | "varchar" | "character varying" | "char" | "character" | "inet" => ValueKind::Text,
        "text[]" => ValueKind::Set,
        "bytea" => ValueKind::Binary,
        "date" => ValueKind::Date,
        "timestamp" | "timestamp without time zone" => {
            if kind.contains("with time zone") && !kind.contains("without") {
                ValueKind::Timestamp
            } else {
                ValueKind::Datetime
            }
        }
        "timestamptz" | "timestamp with time zone" => ValueKind::Timestamp,
        "interval" | "time" | "time without time zone" => ValueKind::Interval,
        "boolean" | "bool" => ValueKind::Boolean,
        "json" | "jsonb" => ValueKind::Json,
        "geometry" => ValueKind::Geometry,
        "bit" => ValueKind::Bit(kind.split_once('(')?.1.split_once(')')?.0.parse().ok()?),
        _ => return None,
    })
}

fn default_sql(column: &SourceColumn, plan: &ColumnPlan) -> Result<Option<String>, String> {
    let Some(value) = column.default.as_ref() else {
        return Ok(None);
    };
    if column.default_is_expression || value.to_ascii_uppercase().starts_with("CURRENT_TIMESTAMP") {
        let normalized = value.to_ascii_uppercase();
        if (normalized == "CURRENT_TIMESTAMP"
            || Regex::new(r"^CURRENT_TIMESTAMP\([0-6]\)$")
                .unwrap()
                .is_match(&normalized))
            && matches!(plan.kind, ValueKind::Datetime | ValueKind::Timestamp)
        {
            return Ok(Some(normalized));
        }
        return Err("default expression requires a reviewed target override".into());
    }
    if !plan.enum_labels.is_empty() {
        let ordinal = plan
            .enum_labels
            .iter()
            .position(|label| label == value)
            .ok_or("ENUM default is not a declared label")? as u64
            + 1;
        return crate::convert::encode_value(plan, &RawValue::UInt(ordinal))
            .map(|value| value.map(|text| format!("E{}", literal(&text))))
            .map_err(|error| error.reason);
    }
    match plan.kind {
        ValueKind::SignedInteger
        | ValueKind::UnsignedInteger
        | ValueKind::Decimal
        | ValueKind::Float => {
            let grammar = if plan.kind == ValueKind::Float {
                r"^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$"
            } else {
                r"^[+-]?[0-9]+(?:\.[0-9]+)?$"
            };
            if !Regex::new(grammar).unwrap().is_match(value) {
                return Err("invalid exact numeric default".into());
            }
            if plan.kind == ValueKind::UnsignedInteger && value.starts_with('-') {
                return Err("unsigned default cannot be negative".into());
            }
            let raw = match plan.kind {
                ValueKind::SignedInteger => RawValue::Int(
                    value
                        .parse()
                        .map_err(|_| "invalid signed integer default")?,
                ),
                ValueKind::UnsignedInteger => RawValue::UInt(
                    value
                        .parse()
                        .map_err(|_| "invalid unsigned integer default")?,
                ),
                ValueKind::Decimal => RawValue::Bytes(value.as_bytes().to_vec()),
                ValueKind::Float => {
                    if column.data_type.eq_ignore_ascii_case("float") {
                        RawValue::Float(value.parse().map_err(|_| "invalid float default")?)
                    } else {
                        RawValue::Double(value.parse().map_err(|_| "invalid float default")?)
                    }
                }
                _ => unreachable!(),
            };
            crate::convert::encode_value(plan, &raw).map_err(|error| error.reason)
        }
        ValueKind::Text | ValueKind::Json => {
            // information_schema strings are already decoded by the metadata session.
            let mut metadata_plan = plan.clone();
            metadata_plan.charset = Some("utf8mb4".into());
            crate::convert::encode_value(
                &metadata_plan,
                &RawValue::Bytes(value.as_bytes().to_vec()),
            )
            .map(|text| text.map(|text| format!("E{}", literal(&text))))
            .map_err(|error| error.reason)
        }
        ValueKind::Date | ValueKind::Datetime | ValueKind::Timestamp | ValueKind::Interval => {
            let raw =
                crate::convert::temporal_default(plan, value).map_err(|error| error.reason)?;
            crate::convert::encode_value(plan, &raw)
                .map(|text| text.map(|text| format!("E{}", literal(&text))))
                .map_err(|error| error.reason)
        }
        ValueKind::Boolean | ValueKind::Bit(_) => {
            if plan.kind == ValueKind::Boolean && !value.starts_with("b'") {
                let raw = RawValue::Int(value.parse().map_err(|_| "invalid boolean default")?);
                return crate::convert::encode_value(plan, &raw).map_err(|error| error.reason);
            }
            let number = if let Some(bits) = value
                .strip_prefix("b'")
                .and_then(|text| text.strip_suffix('\''))
            {
                if bits.is_empty()
                    || bits.len() > 64
                    || !bits.bytes().all(|byte| matches!(byte, b'0' | b'1'))
                {
                    return Err("invalid BIT default".into());
                }
                u64::from_str_radix(bits, 2).map_err(|_| "invalid BIT default")?
            } else {
                value.parse().map_err(|_| "invalid BIT/boolean default")?
            };
            crate::convert::encode_value(plan, &RawValue::UInt(number))
                .map(|text| {
                    text.map(|text| {
                        if matches!(plan.kind, ValueKind::Bit(_)) {
                            format!("B{}", literal(&text))
                        } else {
                            text
                        }
                    })
                })
                .map_err(|error| error.reason)
        }
        ValueKind::Binary => {
            let source_binary = matches!(
                column.data_type.to_ascii_lowercase().as_str(),
                "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob"
            );
            let mut bytes = if source_binary {
                let hex = value
                    .strip_prefix("0x")
                    .ok_or("binary catalog default requires hex metadata")?;
                if !hex.len().is_multiple_of(2) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err("invalid binary catalog default".into());
                }
                hex.as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|chunk| {
                        u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap()
                    })
                    .collect::<Vec<_>>()
            } else {
                value.as_bytes().to_vec()
            };
            if column.data_type.eq_ignore_ascii_case("binary") {
                let length = column
                    .character_length
                    .ok_or("BINARY default requires declared length")?
                    as usize;
                if length > 255 || bytes.len() > length {
                    return Err("binary default exceeds declared length".into());
                }
                bytes.resize(length, 0);
            }
            crate::convert::encode_value(plan, &RawValue::Bytes(bytes))
                .map(|text| text.map(|text| format!("E{}", literal(&text))))
                .map_err(|error| error.reason)
        }
        ValueKind::Set => {
            let mut mask = 0u64;
            if !value.is_empty() {
                for label in value.split(',') {
                    let index = plan
                        .set_labels
                        .iter()
                        .position(|known| known == label)
                        .ok_or("SET default is not a declared label")?;
                    mask |= 1u64 << index;
                }
            }
            crate::convert::encode_value(plan, &RawValue::UInt(mask))
                .map(|text| text.map(|text| format!("E{}", literal(&text))))
                .map_err(|error| error.reason)
        }
        _ => Err("default literal requires a typed target override".into()),
    }
}

fn transform_compatible(column: &ColumnPlan) -> bool {
    match column.transform.as_deref() {
        None => true,
        Some("tinyint-to-boolean" | "bits-to-boolean") => column.kind == ValueKind::Boolean,
        Some("zero-dates-to-null") => matches!(
            column.kind,
            ValueKind::Date | ValueKind::Datetime | ValueKind::Timestamp
        ),
        Some("empty-string-to-null") => {
            matches!(column.kind, ValueKind::Text | ValueKind::Enum)
        }
        Some("remove-null-characters" | "right-trim") => column.kind == ValueKind::Text,
        Some("byte-vector-to-bytea" | "bytes-to-pg-bytea" | "hex-to-bytea") => {
            column.kind == ValueKind::Binary
        }
        Some("set-to-array") => column.kind == ValueKind::Set && !column.set_labels.is_empty(),
        Some("year-to-integer" | "tinyint-to-integer") => matches!(
            column.kind,
            ValueKind::SignedInteger | ValueKind::UnsignedInteger
        ),
        Some("int-to-ip") => column.kind == ValueKind::Text && column.target_type == "inet",
        Some(_) => false,
    }
}

pub fn build(
    config: &MigrationConfig,
    source: &SourceCatalog,
    target: &TargetCatalog,
) -> Result<MigrationPlan, PlanError> {
    crate::config::validate(config).map_err(|error| PlanError::Config(error.to_string()))?;
    let mut plan = MigrationPlan {
        version: 1,
        source_database: source.database.clone(),
        source_version: source.server_version.clone(),
        target_schema: config.target.schema.clone(),
        target_version: target.server_version.clone(),
        consistency: config.source.consistency,
        mode: config.migration.mode,
        verification: config.verification.mode,
        on_existing: config.target.on_existing,
        reset_sequences: config.migration.reset_sequences,
        tables: vec![],
        ddl: vec![],
        diagnostics: vec![],
        exclusions: vec![],
        transformations: vec![],
        hooks: vec![],
    };
    let include: Vec<_> = config
        .tables
        .include_regex
        .iter()
        .map(|value| Regex::new(value).unwrap())
        .collect();
    let exclude: Vec<_> = config
        .tables
        .exclude_regex
        .iter()
        .map(|value| Regex::new(value).unwrap())
        .collect();
    let selected: Vec<_> = source
        .tables
        .iter()
        .filter(|table| {
            let included = (config.tables.include.is_empty() && include.is_empty())
                || config.tables.include.contains(&table.name)
                || include.iter().any(|pattern| pattern.is_match(&table.name));
            let excluded = config.tables.exclude.contains(&table.name)
                || exclude.iter().any(|pattern| pattern.is_match(&table.name));
            let chosen = included
                && !excluded
                && (!table.is_view || config.tables.views.contains(&table.name));
            if !chosen {
                plan.exclusions.push(table.name.clone());
            }
            chosen
        })
        .collect();
    for name in config
        .tables
        .include
        .iter()
        .chain(config.tables.views.iter())
        .chain(config.tables.rename.iter().map(|rule| &rule.source))
    {
        if !source.tables.iter().any(|table| &table.name == name) {
            plan.diagnostics.push(diagnostic(
                "SOURCE_TABLE_MISSING",
                name,
                "configured source object does not exist",
                Severity::Error,
            ));
        }
    }
    for object in &source.unsupported_objects {
        if source
            .tables
            .iter()
            .filter(|table| {
                object.object == table.name
                    || object.object.starts_with(&(table.name.clone() + "."))
            })
            .max_by_key(|table| table.name.len())
            .is_some_and(|owner| !selected.iter().any(|selected| selected.name == owner.name))
        {
            continue;
        }
        if omitted(config, &object.object) {
            plan.exclusions.push(object.object.clone());
        } else {
            plan.diagnostics.push(diagnostic(
                "SOURCE_UNSUPPORTED_OBJECT",
                &object.object,
                &object.reason,
                if matches!(object.kind.as_str(), "routine" | "event" | "trigger") {
                    Severity::Warning
                } else {
                    Severity::Error
                },
            ));
        }
    }
    for (before, paths) in [(true, &config.hooks.before), (false, &config.hooks.after)] {
        for path in paths {
            let object = path.to_string_lossy().into_owned();
            match hooks::read_sql(path).ok().and_then(|bytes| {
                std::str::from_utf8(&bytes).ok()?;
                Some(hooks::digest(&bytes))
            }) {
                Some(sha256) => plan.hooks.push(HookPlan {
                    before,
                    path: object.clone(),
                    sha256,
                }),
                None => plan.diagnostics.push(diagnostic(
                    "HOOK_UNREADABLE",
                    &object,
                    "hook must remain readable UTF-8 SQL while planning",
                    Severity::Error,
                )),
            }
        }
    }
    let mut destinations = BTreeSet::new();
    let mut table_ids = BTreeSet::new();
    let typemod = Regex::new(r"\([^)]*\)").unwrap();
    for table in &selected {
        let identity = format!("{}.{}", source.database, table.name);
        let rename = config
            .tables
            .rename
            .iter()
            .find(|rule| rule.source == table.name);
        let schema = rename
            .and_then(|rule| rule.schema.clone())
            .unwrap_or_else(|| config.target.schema.clone());
        let name = rename
            .map(|rule| stable_name(&rule.target, &identity))
            .unwrap_or_else(|| mapped_name(&table.name, config.migration.identifiers, &identity));
        if !destinations.insert((schema.clone(), name.clone())) {
            plan.diagnostics.push(diagnostic(
                "TABLE_NAME_COLLISION",
                &identity,
                "multiple sources resolve to one destination table",
                Severity::Error,
            ));
        }
        if config.source.consistency == Consistency::SingleSnapshot
            && !table.is_view
            && table.engine != "InnoDB"
        {
            plan.diagnostics.push(diagnostic(
                "SNAPSHOT_ENGINE",
                &identity,
                "single_snapshot requires selected InnoDB tables",
                Severity::Error,
            ));
        }
        if table.columns.iter().any(auto)
            && (source.auto_increment_increment != 1 || source.auto_increment_offset != 1)
        {
            plan.diagnostics.push(diagnostic("IDENTITY_INCREMENT_OFFSET", &identity, "non-default source auto-increment increment/offset needs an explicit generation policy", Severity::Error));
        }
        let mut resolved = TablePlan {
            id: format!("t_{}", &digest(&identity)[..16]),
            source_name: table.name.clone(),
            target_schema: schema,
            target_name: name,
            engine: table.engine.clone(),
            is_view: table.is_view,
            estimated_rows: table.estimated_rows,
            next_auto_increment: table.next_auto_increment,
            columns: vec![],
            primary_key: vec![],
            structure: SchemaExpectations::default(),
        };
        if !table_ids.insert(resolved.id.clone()) {
            plan.diagnostics.push(diagnostic(
                "TABLE_ID_COLLISION",
                &identity,
                "generated artifact identity collides with another selected source",
                Severity::Error,
            ));
        }
        let mut column_names = BTreeSet::new();
        let mut columns: Vec<_> = table.columns.iter().collect();
        columns.sort_by_key(|column| column.ordinal);
        for column in columns {
            let object = format!("{identity}.{}", column.name);
            let source_kind = column.data_type.to_ascii_lowercase();
            let source_labels = if matches!(source_kind.as_str(), "enum" | "set") {
                let Ok(labels) = labels::parse(&column.column_type) else {
                    plan.diagnostics.push(diagnostic(
                        "CATALOG_LABELS_INVALID",
                        &object,
                        "invalid ordered ENUM/SET catalog labels",
                        Severity::Error,
                    ));
                    continue;
                };
                let charset = column
                    .charset
                    .as_deref()
                    .or(table.charset.as_deref())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                // Only positively known BMP-only charsets prove '?' was not
                // substituted for supplementary Unicode by system metadata.
                if !matches!(charset.as_str(), "utf8" | "utf8mb3" | "ascii" | "latin1")
                    && labels.iter().any(|label| label.contains('?'))
                {
                    plan.diagnostics.push(diagnostic(
                        "LOSSY_LABEL_METADATA", &object,
                        "ordered ENUM/SET labels containing '?' are ambiguous: MySQL system metadata can replace supplementary Unicode with '?'; lossless label recovery is unavailable", Severity::Error,
                    ));
                    continue;
                }
                if source_kind == "set" && labels.len() > 64 {
                    plan.diagnostics.push(diagnostic(
                        "CATALOG_LABELS_INVALID",
                        &object,
                        "invalid ordered ENUM/SET catalog labels",
                        Severity::Error,
                    ));
                    continue;
                }
                Some(labels)
            } else {
                None
            };
            let rule = config
                .cast
                .iter()
                .find(|rule| matches(rule, &table.name, column));
            let mapped_type = if let Some(kind) = rule.and_then(|rule| rule.target_type.as_ref()) {
                override_type(kind).map(|value| (kind.clone(), value))
            } else {
                basic_type(column)
            };
            let Some((mut target_type, kind)) = mapped_type else {
                plan.diagnostics.push(diagnostic(
                    "COLUMN_TYPE_UNSUPPORTED",
                    &object,
                    "source type needs an explicit supported cast or the later fidelity capability",
                    Severity::Error,
                ));
                continue;
            };
            if rule.is_some_and(|rule| rule.drop_typemod) {
                target_type = typemod.replace(&target_type, "").into_owned();
            }
            target_type = target_type.trim().to_ascii_lowercase();
            let target_name = mapped_name(&column.name, config.migration.identifiers, &object);
            if !column_names.insert(target_name.clone()) {
                plan.diagnostics.push(diagnostic(
                    "COLUMN_NAME_COLLISION",
                    &object,
                    "multiple columns resolve to one destination identifier",
                    Severity::Error,
                ));
            }
            let mut mapped = ColumnPlan {
                source_name: column.name.clone(),
                target_name,
                source_type: column.column_type.clone(),
                target_type,
                kind,
                nullable: column.nullable || rule.is_some_and(|rule| rule.drop_not_null),
                default_sql: None,
                identity: auto(column),
                generated_expression: None,
                copy: true,
                transform: rule.and_then(|rule| rule.transform.clone()),
                charset: rule
                    .and_then(|rule| rule.charset.clone())
                    .or_else(|| column.charset.clone()),
                enum_labels: vec![],
                set_labels: vec![],
                comment: (!column.comment.is_empty()).then(|| column.comment.clone()),
            };
            if mapped.kind == ValueKind::Geometry
                && !target
                    .extensions
                    .iter()
                    .any(|extension| extension.eq_ignore_ascii_case("postgis"))
            {
                plan.diagnostics.push(diagnostic(
                    "POSTGIS_REQUIRED",
                    &object,
                    "MySQL spatial values require PostGIS to be installed in the target database; my2pg will not install extensions",
                    Severity::Error,
                ));
            }
            if let Some(labels) = source_labels {
                if source_kind == "enum" {
                    mapped.enum_labels = labels;
                } else {
                    mapped.set_labels = labels;
                }
            }
            if mapped.kind == ValueKind::Enum {
                let name = stable_name(
                    &format!(
                        "enum_{}_{}_{}",
                        resolved.target_name,
                        mapped.target_name,
                        &digest(&object)[..8]
                    ),
                    &object,
                );
                mapped.target_type = qualified_name(&resolved.target_schema, &name);
                if mapped.enum_labels.iter().any(|label| label.len() > 63) {
                    plan.diagnostics.push(diagnostic(
                        "ENUM_LABEL_TOO_LONG",
                        &object,
                        "PostgreSQL enum labels exceed63bytes; choose explicit text override",
                        Severity::Error,
                    ));
                }
                let observed_enum = target
                    .tables
                    .iter()
                    .find(|t| t.schema == resolved.target_schema && t.name == resolved.target_name)
                    .and_then(|t| t.observed.as_ref())
                    .and_then(|t| t.columns.iter().find(|c| c.name == mapped.target_name))
                    .filter(|c| c.type_kind == "e")
                    .and_then(|c| {
                        existing::enum_type(
                            target,
                            &c.type_schema,
                            &c.type_name,
                            &mapped.enum_labels,
                        )
                        .map(|_| qualified_name(&c.type_schema, &c.type_name))
                    });
                if let Some(actual) = observed_enum {
                    mapped.target_type = actual;
                } else if existing::namespace(
                    target,
                    &resolved.target_schema,
                    &config.target.schema,
                )
                .is_some_and(|n| n.names.contains_key(&name) || n.types.contains(&name))
                    && existing::enum_type(
                        target,
                        &resolved.target_schema,
                        &name,
                        &mapped.enum_labels,
                    )
                    .is_none()
                {
                    plan.diagnostics.push(diagnostic("ENUM_NAME_OCCUPIED",&object,
                            "existing type requires positively observed exact enum labels/order, identity and USAGE",Severity::Error));
                }
            }
            if source_kind == "enum" && !matches!(mapped.kind, ValueKind::Enum | ValueKind::Text) {
                plan.diagnostics.push(diagnostic(
                    "ENUM_TARGET_UNSUPPORTED",
                    &object,
                    "ENUM ordinal mapping requires enum or explicit text destination",
                    Severity::Error,
                ));
            }
            if source_kind == "set" && mapped.kind != ValueKind::Set {
                plan.diagnostics.push(diagnostic(
                    "SET_TARGET_UNSUPPORTED",
                    &object,
                    "SET mask mapping requires declared text-array membership policy",
                    Severity::Error,
                ));
            }
            if mapped.kind == ValueKind::Text
                && mapped.enum_labels.is_empty()
                && mapped.charset.as_deref().is_some_and(|charset| {
                    !matches!(
                        charset,
                        "utf8"
                            | "utf8mb3"
                            | "utf8mb4"
                            | "latin1"
                            | "ascii"
                            | "windows-1252"
                            | "iso-8859-1"
                            | "binary"
                    )
                })
            {
                plan.diagnostics.push(diagnostic(
                    "CHARSET_UNSUPPORTED",
                    &object,
                    "source charset requires an explicit supported strict decoding override",
                    Severity::Error,
                ));
            }
            if !transform_compatible(&mapped) {
                plan.diagnostics.push(diagnostic(
                    "TRANSFORM_TARGET_MISMATCH",
                    &object,
                    "named transform contradicts the resolved source/destination type",
                    Severity::Error,
                ));
            }
            if mapped.transform.as_deref() == Some("zero-dates-to-null")
                && (!mapped.nullable
                    || (column.default.is_some() && !rule.is_some_and(|rule| rule.drop_default)))
            {
                plan.diagnostics.push(diagnostic("ZERO_DATE_POLICY", &object, "zero-dates-to-null requires nullable target and explicit removal of a source default", Severity::Error));
            }
            if mapped.identity
                && (sequence_max(&mapped.target_type).is_none()
                    || table.next_auto_increment.is_some_and(|value| {
                        value > sequence_max(&mapped.target_type).unwrap_or(0) as u64
                    }))
            {
                plan.diagnostics.push(diagnostic(
                    "IDENTITY_RANGE",
                    &object,
                    "automatic identity generation needs a reviewed representable integer policy",
                    Severity::Error,
                ));
                mapped.identity = false;
            }
            if column.generation_expression.is_some() {
                match override_for(config, &object) {
                    Some(policy) if policy.target_expression.is_some() => {
                        mapped.generated_expression = policy.target_expression.clone();
                        mapped.copy = false;
                    }
                    Some(policy) if policy.materialize == Some(true) => {}
                    _ => plan.diagnostics.push(diagnostic(
                        "GENERATED_POLICY",
                        &object,
                        "generated columns require a PostgreSQL target_expression or materialize=true",
                        Severity::Error,
                    )),
                }
            }
            if column.extra.to_ascii_lowercase().contains("on update")
                && !omitted(config, &(object.clone() + ".on_update"))
            {
                plan.diagnostics.push(diagnostic(
                    "ON_UPDATE_POLICY",
                    &object,
                    "ON UPDATE requires tested emulation or explicit omission",
                    Severity::Error,
                ));
            }
            if column
                .collation
                .as_ref()
                .is_some_and(|collation| collation != "binary")
            {
                let key = table.indexes.iter().any(|index| {
                    (index.primary || index.unique)
                        && index
                            .parts
                            .iter()
                            .any(|part| part.column.as_ref() == Some(&column.name))
                });
                let acknowledged = omitted(config, &(object.clone() + ".collation"));
                plan.diagnostics.push(diagnostic("COLLATION_SEMANTICS", &object, "MySQL collation comparison/uniqueness and PAD SPACE behavior do not automatically match PostgreSQL; review explicit collation policy", if key && !acknowledged { Severity::Error } else { Severity::Warning }));
            }
            if !mapped.identity
                && column.generation_expression.is_none()
                && !rule.is_some_and(|rule| rule.drop_default)
            {
                match default_sql(column, &mapped) {
                    Ok(default) => mapped.default_sql = default,
                    Err(message) => plan.diagnostics.push(diagnostic(
                        "DEFAULT_UNSUPPORTED",
                        &object,
                        &message,
                        Severity::Error,
                    )),
                }
            }
            if let Some(transform) = &mapped.transform {
                plan.transformations.push(format!("{object}: {transform}"));
            }
            resolved.columns.push(mapped);
        }
        for index in table.indexes.iter().filter(|index| index.primary) {
            resolved.primary_key = index
                .parts
                .iter()
                .filter_map(|part| part.column.as_ref())
                .filter_map(|name| {
                    resolved
                        .columns
                        .iter()
                        .find(|column| &column.source_name == name)
                        .map(|column| column.source_name.clone())
                })
                .collect();
        }
        if resolved
            .columns
            .iter()
            .any(|column| resolved.primary_key.contains(&column.source_name) && column.nullable)
        {
            plan.diagnostics.push(diagnostic(
                "PRIMARY_KEY_NULLABLE",
                &identity,
                "resolved primary-key columns must remain NOT NULL",
                Severity::Error,
            ));
        }
        plan.tables.push(resolved);
    }
    for table in &plan.tables {
        let existing = target.tables.iter().find(|existing| {
            existing.schema == table.target_schema && existing.name == table.target_name
        });
        if existing.is_none()
            && existing::namespace(target, &table.target_schema, &config.target.schema)
                .is_some_and(|n| n.names.contains_key(&table.target_name))
        {
            plan.diagnostics.push(diagnostic(
                "TARGET_RELATION_COLLISION",
                &table.source_name,
                "destination identifier is occupied by a view, sequence, index or other relation",
                Severity::Error,
            ));
        }
        let creates = config.migration.mode != MigrationMode::DataOnly
            && (existing.is_none() || config.target.on_existing == ExistingPolicy::Recreate);
        existing::namespace_policy(config, target, table, creates, &mut plan.diagnostics);
        if config.migration.mode == MigrationMode::DataOnly && existing.is_none() {
            plan.diagnostics.push(diagnostic(
                "TARGET_TABLE_MISSING",
                &table.source_name,
                "data_only requires an existing compatible target table",
                Severity::Error,
            ));
        }
        if let Some(existing) = existing {
            if !existing.ordinary_standalone {
                plan.diagnostics.push(diagnostic(
                    "TARGET_TABLE_TOPOLOGY",
                    &table.source_name,
                    "existing destination requires positive ordinary standalone table proof; partition or inheritance handling is unsupported",
                    Severity::Error,
                ));
            }
            let recreate = config.target.on_existing == ExistingPolicy::Recreate;
            let copies = config.migration.mode != MigrationMode::SchemaOnly;
            let resets = config.migration.reset_sequences
                && table.columns.iter().any(|column| column.identity);
            let required = [
                (
                    "TARGET_INSERT_DENIED",
                    !recreate && copies && !existing.can_insert,
                ),
                (
                    "TARGET_SELECT_DENIED",
                    !recreate
                        && (resets || config.verification.mode != VerificationMode::None)
                        && !existing.can_select,
                ),
                (
                    "TARGET_OWNERSHIP_DENIED",
                    (recreate || resets) && !existing.can_alter,
                ),
                (
                    "TARGET_TRUNCATE_DENIED",
                    config.target.on_existing == ExistingPolicy::Truncate && !existing.can_truncate,
                ),
                (
                    "TARGET_ROW_SECURITY",
                    !recreate
                        && (copies || config.verification.mode != VerificationMode::None)
                        && existing.row_security_active,
                ),
            ];
            for (code, denied) in required {
                if denied {
                    plan.diagnostics.push(diagnostic(code, &table.source_name,
                        "existing destination privileges or row security contradict the requested operation", Severity::Error));
                }
            }
            if config.migration.mode != MigrationMode::DataOnly
                && config.target.on_existing == ExistingPolicy::Error
            {
                plan.diagnostics.push(diagnostic(
                    "TARGET_EXISTS",
                    &table.source_name,
                    "destination table already exists; choose an explicit target policy",
                    Severity::Error,
                ));
            }
        }
    }
    let mut type_names: BTreeSet<_> = plan
        .tables
        .iter()
        .map(|table| qualified_name(&table.target_schema, &table.target_name))
        .collect();
    for table in &plan.tables {
        for column in table
            .columns
            .iter()
            .filter(|column| column.kind == ValueKind::Enum)
        {
            if !type_names.insert(column.target_type.clone())
                && !existing::reuses_enum(target, column)
            {
                plan.diagnostics.push(diagnostic(
                    "ENUM_NAME_COLLISION",
                    &format!("{}.{}", table.source_name, column.source_name),
                    "generated enum conflicts with another enum or selected table composite type",
                    Severity::Error,
                ));
            }
        }
    }
    generate_ddl(config, source, target, &mut plan);
    existing::apply(config, target, &mut plan);
    let errors: Vec<_> = plan
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .cloned()
        .collect();
    if errors.is_empty() {
        Ok(plan)
    } else {
        Err(PlanError::Blocked(errors))
    }
}

fn compatible_type(actual: &str, expected: &str) -> bool {
    crate::types::equivalent_postgres_types(actual, expected)
}

// Finite built-in default-btree FK semantics; assignment casts are insufficient.
fn fk_type_compatible(child: &ColumnPlan, parent: &ColumnPlan) -> bool {
    let (Some(child_type), Some(parent_type)) = (
        crate::types::normalize_postgres_type(&child.target_type),
        crate::types::normalize_postgres_type(&parent.target_type),
    ) else {
        return false;
    };
    if child.kind == ValueKind::Enum || parent.kind == ValueKind::Enum {
        return child.kind == ValueKind::Enum
            && parent.kind == ValueKind::Enum
            && child_type == parent_type
            && !child.enum_labels.is_empty()
            && child.enum_labels == parent.enum_labels;
    }
    if child_type.ends_with("[]") || parent_type.ends_with("[]") {
        return child_type == "text[]" && parent_type == "text[]";
    }
    let child_base = child_type.split('(').next().unwrap();
    let parent_base = parent_type.split('(').next().unwrap();
    let integer = |kind| matches!(kind, "smallint" | "integer" | "bigint");
    if integer(child_base) && (integer(parent_base) || parent_base == "numeric") {
        return true;
    }
    child_base == parent_base
        && matches!(
            child_base,
            "numeric"
                | "real"
                | "double precision"
                | "boolean"
                | "bytea"
                | "text"
                | "varchar"
                | "character"
                | "date"
                | "timestamp"
                | "timestamptz"
                | "time"
                | "timetz"
                | "interval"
                | "uuid"
                | "inet"
                | "money"
                | "bit"
                | "varbit"
                | "jsonb"
        )
}

fn generate_ddl(
    config: &MigrationConfig,
    source: &SourceCatalog,
    target: &TargetCatalog,
    plan: &mut MigrationPlan,
) {
    let tables = plan.tables.clone();
    let mut relation_names: BTreeSet<_> = tables
        .iter()
        .map(|table| (table.target_schema.clone(), table.target_name.clone()))
        .collect();
    let mappings: BTreeMap<_, _> = tables
        .iter()
        .map(|table| (table.source_name.as_str(), table))
        .collect();
    if config.migration.mode != MigrationMode::DataOnly {
        for schema in tables
            .iter()
            .map(|table| &table.target_schema)
            .collect::<BTreeSet<_>>()
        {
            if existing::namespace(target, schema, &config.target.schema).is_some_and(|n| !n.exists)
            {
                plan.ddl.push(DdlStep {
                    phase: DdlPhase::Prepare,
                    table_id: None,
                    object: schema.clone(),
                    sql: format!("CREATE SCHEMA IF NOT EXISTS {};", quote_identifier(schema)),
                });
            }
        }
    }
    if config.target.on_existing == ExistingPolicy::Recreate {
        let dropped: Vec<_> = tables
            .iter()
            .filter(|table| {
                target.tables.iter().any(|existing| {
                    existing.schema == table.target_schema && existing.name == table.target_name
                })
            })
            .map(|table| qualified_name(&table.target_schema, &table.target_name))
            .collect();
        if !dropped.is_empty() {
            plan.ddl.push(DdlStep {
                phase: DdlPhase::Prepare,
                table_id: None,
                object: "selected destination tables".into(),
                sql: format!("DROP TABLE {} RESTRICT;", dropped.join(", ")),
            });
        }
    }
    if config.target.on_existing == ExistingPolicy::Truncate {
        let truncated: Vec<_> = tables
            .iter()
            .filter(|table| {
                target.tables.iter().any(|existing| {
                    existing.schema == table.target_schema && existing.name == table.target_name
                })
            })
            .map(|table| {
                format!(
                    "ONLY {}",
                    qualified_name(&table.target_schema, &table.target_name)
                )
            })
            .collect();
        if !truncated.is_empty() {
            plan.ddl.push(DdlStep {
                phase: DdlPhase::Prepare,
                table_id: None,
                object: "selected destination tables".into(),
                sql: format!(
                    "TRUNCATE TABLE {} CONTINUE IDENTITY RESTRICT;",
                    truncated.join(", ")
                ),
            });
        }
    }
    for table in &tables {
        let name = qualified_name(&table.target_schema, &table.target_name);
        let original = source
            .tables
            .iter()
            .find(|source| source.name == table.source_name)
            .unwrap();
        let existing = target.tables.iter().any(|existing| {
            existing.schema == table.target_schema && existing.name == table.target_name
        });
        let creates = config.migration.mode != MigrationMode::DataOnly
            && (!existing || config.target.on_existing == ExistingPolicy::Recreate);
        if creates {
            for column in table
                .columns
                .iter()
                .filter(|column| column.kind == ValueKind::Enum)
            {
                if existing::reuses_enum(target, column) {
                    continue;
                }
                plan.ddl.push(DdlStep {
                    phase: DdlPhase::Prepare,
                    table_id: Some(table.id.clone()),
                    object: format!("{}.{}", table.source_name, column.source_name),
                    sql: format!(
                        "CREATE TYPE {} AS ENUM ({});",
                        column.target_type,
                        column
                            .enum_labels
                            .iter()
                            .map(|label| format!("E{}", literal(label)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
            }
            let definitions: Vec<_> = table
                .columns
                .iter()
                .map(|column| {
                    format!(
                        "{} {}{}{}{}{}",
                        quote_identifier(&column.target_name),
                        column.target_type,
                        if column.identity {
                            " GENERATED BY DEFAULT AS IDENTITY"
                        } else {
                            ""
                        },
                        column
                            .generated_expression
                            .as_ref()
                            .map(|expression| {
                                format!(" GENERATED ALWAYS AS ({expression}) STORED")
                            })
                            .unwrap_or_default(),
                        column
                            .default_sql
                            .as_ref()
                            .map(|default| format!(" DEFAULT {default}"))
                            .unwrap_or_default(),
                        if column.nullable { "" } else { " NOT NULL" }
                    )
                })
                .collect();
            plan.ddl.push(DdlStep {
                phase: DdlPhase::Prepare,
                table_id: Some(table.id.clone()),
                object: table.source_name.clone(),
                sql: format!("CREATE TABLE {name} ({});", definitions.join(", ")),
            });
        }
        let mut expectations = SchemaExpectations {
            complete: true,
            comment: (!original.comment.is_empty()).then(|| original.comment.clone()),
            ..Default::default()
        };
        if creates {
            expectations.sequences = table
                .columns
                .iter()
                .filter(|column| column.identity)
                .map(|column| SequenceExpectation {
                    column: column.target_name.clone(),
                    increment: 1,
                    min_value: 1,
                    max_value: sequence_max(&column.target_type).expect("validated identity type"),
                    cycle: false,
                    next_minimum: table.next_auto_increment.unwrap_or(1).max(1),
                    preserved_state: (!config.migration.reset_sequences).then_some(
                        SequenceStateObservation::Known {
                            last_value: 1,
                            is_called: false,
                        },
                    ),
                })
                .collect();
        }
        for column in table
            .columns
            .iter()
            .filter(|column| column.kind == ValueKind::Set)
        {
            let object = format!(
                "{}.{}.{}.set_membership",
                source.database, table.source_name, column.source_name
            );
            let constraint = stable_name(
                &format!(
                    "{}_{}_members_{}",
                    table.target_name,
                    column.target_name,
                    &digest(&object)[..8]
                ),
                &object,
            );
            let expression = format!(
                "{} <@ ARRAY[{}]::text[]",
                quote_identifier(&column.target_name),
                column
                    .set_labels
                    .iter()
                    .map(|label| format!("E{}", literal(label)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            expectations.checks.push(CheckExpectation {
                name: constraint.clone(),
                expression: expression.clone(),
                set_membership: Some(SetMembershipExpectation {
                    column: column.target_name.clone(),
                    labels: column.set_labels.clone(),
                }),
            });
            if creates {
                plan.ddl.push(DdlStep {
                    phase: DdlPhase::Finalize,
                    table_id: Some(table.id.clone()),
                    object,
                    sql: format!(
                        "ALTER TABLE {name} ADD CONSTRAINT {} CHECK ({expression});",
                        quote_identifier(&constraint)
                    ),
                });
            }
        }
        {
            for index in &original.indexes {
                let object = format!("{}.{}.{}", source.database, original.name, index.name);
                if omitted(config, &object) {
                    plan.exclusions.push(object);
                    continue;
                }
                let expression_count = index
                    .parts
                    .iter()
                    .filter(|part| part.expression.is_some())
                    .count();
                let target_expression = (expression_count == 1)
                    .then(|| {
                        override_for(config, &object)
                            .and_then(|policy| policy.target_expression.as_deref())
                    })
                    .flatten();
                if expression_count == 1 && target_expression.is_none() {
                    plan.diagnostics.push(diagnostic(
                        "INDEX_EXPRESSION_POLICY",
                        &object,
                        "functional index requires a PostgreSQL target_expression or explicit omission",
                        Severity::Error,
                    ));
                    continue;
                }
                if index.kind != "BTREE"
                    || expression_count > 1
                    || (expression_count > 0 && index.primary)
                    || index.parts.iter().any(|part| {
                        part.prefix_length.is_some()
                            || (part.expression.is_none() && part.column.is_none())
                            || (part.expression.is_some() && part.column.is_some())
                            || (index.primary && part.descending)
                    })
                {
                    plan.diagnostics.push(diagnostic(
                        "INDEX_UNSUPPORTED",
                        &object,
                        "index semantics require explicit omission or reviewed replacement",
                        Severity::Error,
                    ));
                    continue;
                }
                let parts: Option<Vec<_>> = index
                    .parts
                    .iter()
                    .map(|part| {
                        let key = if part.expression.is_some() {
                            target_expression.map(|expression| format!("({expression})"))
                        } else {
                            table
                                .columns
                                .iter()
                                .find(|column| Some(&column.source_name) == part.column.as_ref())
                                .map(|column| quote_identifier(&column.target_name))
                        }?;
                        Some(format!(
                            "{key}{}",
                            if part.descending { " DESC" } else { "" }
                        ))
                    })
                    .collect();
                let Some(parts) = parts else {
                    plan.diagnostics.push(diagnostic(
                        "INDEX_COLUMN_MISSING",
                        &object,
                        "index column is absent from resolved mapping",
                        Severity::Error,
                    ));
                    continue;
                };
                let index_name = stable_name(
                    &format!(
                        "idx_{}_{}_{}",
                        table.target_name,
                        index.name,
                        &digest(&object)[..8]
                    ),
                    &object,
                );
                if creates
                    && (!relation_names.insert((table.target_schema.clone(), index_name.clone()))
                        || (existing::namespace(
                            target,
                            &table.target_schema,
                            &config.target.schema,
                        )
                        .is_some_and(|n| n.names.contains_key(&index_name))
                            && !(config.target.on_existing == ExistingPolicy::Recreate
                                && existing::relation_is_replaced(
                                    target,
                                    &table.target_schema,
                                    &index_name,
                                    &tables,
                                ))))
                {
                    plan.diagnostics.push(diagnostic(
                        "INDEX_NAME_COLLISION",
                        &object,
                        "generated index name collides with an existing or selected relation",
                        Severity::Error,
                    ));
                }
                let sql = if index.primary {
                    format!(
                        "ALTER TABLE {name} ADD CONSTRAINT {} PRIMARY KEY ({});",
                        quote_identifier(&index_name),
                        parts.join(", ")
                    )
                } else {
                    format!(
                        "CREATE {}INDEX {} ON {name} ({});",
                        if index.unique { "UNIQUE " } else { "" },
                        quote_identifier(&index_name),
                        parts.join(", ")
                    )
                };
                expectations.indexes.push(IndexExpectation {
                    name: index_name,
                    columns: index
                        .parts
                        .iter()
                        .filter_map(|part| {
                            part.column.as_ref().and_then(|source_name| {
                                table
                                    .columns
                                    .iter()
                                    .find(|column| &column.source_name == source_name)
                                    .map(|column| column.target_name.clone())
                            })
                        })
                        .collect(),
                    expressions: index
                        .parts
                        .iter()
                        .map(|part| {
                            part.expression
                                .as_ref()
                                .map(|_| target_expression.unwrap().to_owned())
                        })
                        .collect(),
                    unique: index.unique || index.primary,
                    primary: index.primary,
                    descending: index.parts.iter().map(|part| part.descending).collect(),
                });
                if creates {
                    plan.ddl.push(DdlStep {
                        phase: DdlPhase::Finalize,
                        table_id: Some(table.id.clone()),
                        object,
                        sql,
                    });
                }
            }
            for fk in &original.foreign_keys {
                let object = format!("{}.{}.{}", source.database, original.name, fk.name);
                if omitted(config, &object) {
                    plan.exclusions.push(object);
                    continue;
                }
                let Some(parent) = mappings
                    .get(fk.referenced_table.as_str())
                    .filter(|_| fk.referenced_schema == source.database)
                else {
                    plan.diagnostics.push(diagnostic(
                        "FK_PARENT_MISSING",
                        &object,
                        "referenced parent is not selected from this source database",
                        Severity::Error,
                    ));
                    continue;
                };
                let local: Option<Vec<_>> = fk
                    .columns
                    .iter()
                    .map(|name| {
                        table
                            .columns
                            .iter()
                            .find(|column| &column.source_name == name)
                            .map(|column| quote_identifier(&column.target_name))
                    })
                    .collect();
                let remote: Option<Vec<_>> = fk
                    .referenced_columns
                    .iter()
                    .map(|name| {
                        parent
                            .columns
                            .iter()
                            .find(|column| &column.source_name == name)
                            .map(|column| quote_identifier(&column.target_name))
                    })
                    .collect();
                let actions = ["RESTRICT", "CASCADE", "SET NULL", "NO ACTION"];
                if local.is_none()
                    || remote.is_none()
                    || fk.columns.len() != fk.referenced_columns.len()
                    || !actions.contains(&fk.on_delete.as_str())
                    || !actions.contains(&fk.on_update.as_str())
                {
                    plan.diagnostics.push(diagnostic(
                        "FK_UNSUPPORTED",
                        &object,
                        "foreign-key columns/action are unsupported or unresolved",
                        Severity::Error,
                    ));
                    continue;
                }
                let mut incompatible = false;
                for (local_name, remote_name) in fk.columns.iter().zip(&fk.referenced_columns) {
                    let child_column = table
                        .columns
                        .iter()
                        .find(|c| &c.source_name == local_name)
                        .unwrap();
                    let parent_column = parent
                        .columns
                        .iter()
                        .find(|c| &c.source_name == remote_name)
                        .unwrap();
                    if !fk_type_compatible(child_column, parent_column) {
                        incompatible = true;
                        plan.diagnostics.push(diagnostic(
                            "FK_TYPE_INCOMPATIBLE",
                            &object,
                            &format!(
                                "source {}.{}.{} -> {}.{}.{} resolves to {}.{} ({}) -> {}.{} ({}); foreign-key equality is unsupported or incompatible",
                                source.database, original.name, local_name,
                                fk.referenced_schema, fk.referenced_table, remote_name,
                                qualified_name(&table.target_schema, &table.target_name), quote_identifier(&child_column.target_name), child_column.target_type,
                                qualified_name(&parent.target_schema, &parent.target_name), quote_identifier(&parent_column.target_name), parent_column.target_type,
                            ),
                            Severity::Error,
                        ));
                    }
                }
                if incompatible {
                    continue;
                }
                expectations.foreign_keys.push(ForeignKeyExpectation {
                    name: stable_name(&fk.name, &object),
                    columns: fk
                        .columns
                        .iter()
                        .map(|name| {
                            table
                                .columns
                                .iter()
                                .find(|column| &column.source_name == name)
                                .unwrap()
                                .target_name
                                .clone()
                        })
                        .collect(),
                    referenced_schema: parent.target_schema.clone(),
                    referenced_table: parent.target_name.clone(),
                    referenced_columns: fk
                        .referenced_columns
                        .iter()
                        .map(|name| {
                            parent
                                .columns
                                .iter()
                                .find(|column| &column.source_name == name)
                                .unwrap()
                                .target_name
                                .clone()
                        })
                        .collect(),
                    on_update: fk.on_update.clone(),
                    on_delete: fk.on_delete.clone(),
                });
                if creates {
                    plan.ddl.push(DdlStep { phase: DdlPhase::Finalize, table_id: Some(table.id.clone()), object: object.clone(), sql: format!("ALTER TABLE {name} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}) ON UPDATE {} ON DELETE {};", quote_identifier(&stable_name(&fk.name, &object)), local.unwrap().join(", "), qualified_name(&parent.target_schema, &parent.target_name), remote.unwrap().join(", "), fk.on_update, fk.on_delete) });
                }
            }
            for check in &original.checks {
                let object = format!("{}.{}.{}", source.database, original.name, check.name);
                if omitted(config, &object) {
                    plan.exclusions.push(object);
                    continue;
                }
                let Some(expression) = override_for(config, &object)
                    .and_then(|policy| policy.target_expression.as_deref())
                else {
                    plan.diagnostics.push(diagnostic(
                        "CHECK_NOT_IMPLEMENTED",
                        &object,
                        "CHECK requires an explicit PostgreSQL target_expression or explicit omission",
                        Severity::Error,
                    ));
                    continue;
                };
                if !check.enforced {
                    plan.diagnostics.push(diagnostic(
                        "CHECK_ENFORCEMENT_MISMATCH",
                        &object,
                        "an unenforced MySQL CHECK cannot be recreated as an enforced PostgreSQL CHECK; explicitly omit it",
                        Severity::Error,
                    ));
                    continue;
                }
                let constraint = stable_name(&check.name, &object);
                expectations.checks.push(CheckExpectation {
                    name: constraint.clone(),
                    expression: expression.to_owned(),
                    set_membership: None,
                });
                if creates {
                    plan.ddl.push(DdlStep {
                        phase: DdlPhase::Finalize,
                        table_id: Some(table.id.clone()),
                        object,
                        sql: format!(
                            "ALTER TABLE {name} ADD CONSTRAINT {} CHECK ({expression});",
                            quote_identifier(&constraint)
                        ),
                    });
                }
            }
        }
        if creates && let Some(comment) = &expectations.comment {
            plan.ddl.push(DdlStep {
                phase: DdlPhase::Finalize,
                table_id: Some(table.id.clone()),
                object: table.source_name.clone(),
                sql: format!("COMMENT ON TABLE {name} IS E{};", literal(comment)),
            });
        }
        if creates {
            for column in &table.columns {
                if let Some(comment) = &column.comment {
                    plan.ddl.push(DdlStep {
                        phase: DdlPhase::Finalize,
                        table_id: Some(table.id.clone()),
                        object: format!("{}.{}", table.source_name, column.source_name),
                        sql: format!(
                            "COMMENT ON COLUMN {name}.{} IS E{};",
                            quote_identifier(&column.target_name),
                            literal(comment)
                        ),
                    });
                }
            }
        }
        plan.tables
            .iter_mut()
            .find(|resolved| resolved.id == table.id)
            .unwrap()
            .structure = expectations;
        if config.migration.reset_sequences {
            for column in table.columns.iter().filter(|column| column.identity) {
                let sql = crate::postgres::sequences::adjustment_sql(table, column)
                    .expect("validated identity type");
                plan.ddl.push(DdlStep {
                    phase: DdlPhase::Sequence,
                    table_id: Some(table.id.clone()),
                    object: format!("{}.{}", table.source_name, column.source_name),
                    sql,
                });
                plan.diagnostics.push(diagnostic(
                    "SEQUENCE_NONTRANSACTIONAL", &format!("{}.{}", table.source_name, column.source_name),
                    "monotonic sequence adjustment uses setval; its acknowledged or uncertain effects may survive a later transaction or migration failure", Severity::Warning,
                ));
            }
        } else if table.columns.iter().any(|column| column.identity) {
            plan.diagnostics.push(diagnostic("SEQUENCE_STATE_PRESERVED", &table.source_name,
                "sequence adjustment is disabled; verify the preserved state, without claiming generation is ahead of imported rows or source AUTO_INCREMENT", Severity::Warning));
        }
    }
    // All keys/indexes precede FKs, including circular relational graphs.
    plan.ddl.sort_by_key(|step| match step.phase {
        DdlPhase::Prepare => 0,
        DdlPhase::Finalize if step.sql.contains(" FOREIGN KEY ") => 2,
        DdlPhase::Finalize => 1,
        DdlPhase::Sequence => 3,
    });
}

#[cfg(test)]
mod tests;
