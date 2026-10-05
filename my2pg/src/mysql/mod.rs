//! Oracle MySQL transport. Metadata query errors never become an empty catalog.
use crate::pipeline::ranges::{IntegerKey, IntegerRange};
use crate::{
    config::{Consistency, SourceConfig, TlsMode},
    model::*,
};
use futures_util::FutureExt;
use mysql_async::{
    BinaryProtocol, ClientIdentity, Conn, Opts, OptsBuilder, ResultSetStream, Row, SslOpts, Value,
    prelude::*,
};
use std::collections::BTreeMap;

/// Owns one unpooled reader, including its snapshot. All exits close promptly.
pub struct SourceConnection(Option<Conn>);

impl std::fmt::Debug for SourceConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceConnection")
            .field("id", &self.0.as_ref().map(Conn::id))
            .finish()
    }
}

impl std::ops::Deref for SourceConnection {
    type Target = Conn;
    fn deref(&self) -> &Conn {
        self.0.as_ref().expect("live source connection")
    }
}
impl std::ops::DerefMut for SourceConnection {
    fn deref_mut(&mut self) -> &mut Conn {
        self.0.as_mut().expect("live source connection")
    }
}
impl Drop for SourceConnection {
    fn drop(&mut self) {
        if let Some(conn) = self.0.take() {
            if std::thread::panicking() {
                // Pinned mysql_async Conn::drop returns immediately while
                // unwinding, directly dropping this unpooled socket. It does
                // not schedule the normal cleanup/drain task in this branch.
                drop(conn);
            } else {
                let _ = close_now(conn);
            }
        }
    }
}
impl SourceConnection {
    pub async fn disconnect(mut self) -> Result<(), SourceError> {
        close_now(self.0.take().expect("live source connection"))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("MySQL connection/session initialization exceeded the 10-second deadline")]
    InitializationDeadline,
    #[error("MySQL operation failed (code {code:?}, SQLSTATE {state:?})")]
    Driver {
        code: Option<u16>,
        state: Option<String>,
    },
    #[error("unsupported source server: {0}")]
    Unsupported(String),
    #[error("invalid MySQL configuration: {0}")]
    Configuration(String),
    #[error("invalid source metadata at {0}")]
    Metadata(String),
    #[error("raw source row exceeds configured maximum {limit} bytes (observed {observed})")]
    OversizedRow { limit: usize, observed: usize },
}

impl From<mysql_async::Error> for SourceError {
    fn from(error: mysql_async::Error) -> Self {
        // Server messages may contain row contents or credentials. Keep only
        // protocol classification fields in ordinary diagnostics.
        match error {
            mysql_async::Error::Server(error) => Self::Driver {
                code: Some(error.code),
                state: Some(error.state),
            },
            _ => Self::Driver {
                code: None,
                state: None,
            },
        }
    }
}

pub fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

pub fn validate_server(version: &str, comment: &str) -> Result<(u32, u32, u32), SourceError> {
    let identity = format!("{version} {comment}").to_ascii_lowercase();
    if [
        "mariadb",
        "tidb",
        "vitess",
        "percona",
        "aurora",
        "oceanbase",
    ]
    .iter()
    .any(|variant| identity.contains(variant))
        || !comment.contains("MySQL")
        || !(comment.contains("Community") || comment.contains("Enterprise"))
    {
        return Err(SourceError::Unsupported(
            "only identifiable Oracle MySQL Community/Enterprise is supported".into(),
        ));
    }
    let numbers: Vec<_> = version
        .split('.')
        .take(3)
        .map(|p| p.split('-').next().unwrap_or("").parse::<u32>())
        .collect();
    let [Ok(major), Ok(minor), Ok(patch)] = numbers.as_slice() else {
        return Err(SourceError::Unsupported(
            "unrecognized MySQL version".into(),
        ));
    };
    if !matches!((*major, *minor), (5, 7) | (8, 0) | (8, 4)) {
        return Err(SourceError::Unsupported(
            "supported MySQL lines are 5.7, 8.0 and 8.4".into(),
        ));
    }
    Ok((*major, *minor, *patch))
}

pub async fn connect(
    config: &SourceConfig,
    resolved_url: &str,
    max_row_bytes: usize,
) -> Result<SourceConnection, SourceError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        connect_initialized(config, resolved_url, max_row_bytes),
    )
    .await
    .map_err(|_| SourceError::InitializationDeadline)?
}

async fn connect_initialized(
    config: &SourceConfig,
    resolved_url: &str,
    max_row_bytes: usize,
) -> Result<SourceConnection, SourceError> {
    let parsed = url::Url::parse(resolved_url)
        .map_err(|_| SourceError::Configuration("invalid source URL".into()))?;
    if parsed.scheme() != "mysql"
        || parsed.host_str().is_none()
        || parsed.username().is_empty()
        || parsed.path().trim_matches('/').is_empty()
    {
        return Err(SourceError::Configuration(
            "mysql URL requires explicit username, host and database".into(),
        ));
    }
    // Driver URL options can override validation, enable local file handlers or
    // select a socket. Product transport/session options live in typed config.
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(SourceError::Configuration(
            "source URL parameters/fragments are unsupported; use typed TLS/session settings"
                .into(),
        ));
    }
    let opts = Opts::from_url(resolved_url)
        .map_err(|_| SourceError::Configuration("invalid source URL".into()))?;
    let ssl = match config.tls_mode {
        TlsMode::Disable => {
            if config.ca_file.is_some()
                || config.client_cert.is_some()
                || config.client_key.is_some()
            {
                return Err(SourceError::Configuration(
                    "TLS files require verify_full".into(),
                ));
            }
            None
        }
        TlsMode::VerifyFull => {
            let mut ssl = SslOpts::default();
            if let Some(ca) = &config.ca_file {
                ssl = ssl
                    .with_root_certs(vec![ca.clone().into()])
                    .with_disable_built_in_roots(true);
            }
            match (&config.client_cert, &config.client_key) {
                (Some(cert), Some(key)) => {
                    ssl = ssl.with_client_identity(Some(ClientIdentity::new(
                        cert.clone().into(),
                        key.clone().into(),
                    )))
                }
                (None, None) => (),
                _ => {
                    return Err(SourceError::Configuration(
                        "client certificate and key must be supplied together".into(),
                    ));
                }
            }
            Some(ssl)
        }
    };
    let packet_limit = max_row_bytes
        .checked_add(64 * 1024)
        .ok_or_else(|| SourceError::Configuration("row limit overflows packet bound".into()))?
        .clamp(1024, 1024 * 1024 * 1024);
    let mut session = config.session.clone();
    if session
        .get("time_zone")
        .is_some_and(|value| value != "+00:00")
    {
        return Err(SourceError::Configuration(
            "source time_zone must be +00:00".into(),
        ));
    }
    session.insert("time_zone".into(), "+00:00".into());
    let mut prepared = BTreeMap::new();
    for (name, value) in session {
        if !matches!(
            name.as_str(),
            "time_zone"
                | "sql_mode"
                | "net_read_timeout"
                | "net_write_timeout"
                | "wait_timeout"
                | "interactive_timeout"
                | "group_concat_max_len"
        ) {
            return Err(SourceError::Configuration(
                "unsupported source session variable".into(),
            ));
        }
        if value.is_empty() || value.len() > 1024 || value.contains('\0') {
            return Err(SourceError::Configuration(
                "source session value must be nonempty, without NUL, and at most 1024 bytes".into(),
            ));
        }
        let value = if matches!(
            name.as_str(),
            "net_read_timeout"
                | "net_write_timeout"
                | "wait_timeout"
                | "interactive_timeout"
                | "group_concat_max_len"
        ) {
            Value::UInt(value.parse::<u64>().map_err(|_| {
                SourceError::Configuration(
                    "numeric source session setting requires an unsigned integer".into(),
                )
            })?)
        } else {
            Value::Bytes(value.into_bytes())
        };
        prepared.insert(name, value);
    }
    let conn = Conn::new(
        OptsBuilder::from_opts(opts)
            .prefer_socket(false)
            .ssl_opts(ssl)
            .max_allowed_packet(Some(packet_limit)),
    )
    .await?;
    let mut conn = SourceConnection(Some(conn));
    for (name, value) in prepared {
        let expected_numeric = if let Value::UInt(value) = &value {
            Some(*value)
        } else {
            None
        };
        conn.exec_drop(format!("SET SESSION {} = ?", quote_ident(&name)), (value,))
            .await?;
        if let Some(expected) = expected_numeric {
            let actual: Option<u64> = conn
                .query_first(format!("SELECT @@session.{}", quote_ident(&name)))
                .await?;
            if actual != Some(expected) {
                return Err(SourceError::Configuration(
                    "numeric source session setting was not accepted exactly".into(),
                ));
            }
        }
    }
    Ok(conn)
}

fn field<T: FromValue>(row: &Row, index: usize, context: &str) -> Result<T, SourceError> {
    row.get_opt::<T, _>(index)
        .ok_or_else(|| SourceError::Metadata(context.into()))?
        .map_err(|_| SourceError::Metadata(context.into()))
}

pub async fn inspect(conn: &mut Conn) -> Result<SourceCatalog, SourceError> {
    let identity: Option<(String, String, Option<String>,u64,u64)> = conn
        .query_first("SELECT VERSION(), @@version_comment, DATABASE(), @@auto_increment_increment, @@auto_increment_offset")
        .await?;
    let (server_version, server_comment, database, auto_increment_increment, auto_increment_offset) =
        identity.ok_or_else(|| SourceError::Metadata("server identity".into()))?;
    let version = validate_server(&server_version, &server_comment)?;
    let database =
        database.ok_or_else(|| SourceError::Configuration("source database is required".into()))?;
    let rows: Vec<Row> = conn.exec(include_str!("tables.sql"), (&database,)).await?;
    let mut catalog = SourceCatalog {
        database: database.clone(),
        server_version,
        server_comment,
        auto_increment_increment,
        auto_increment_offset,
        tables: Vec::new(),
        unsupported_objects: Vec::new(),
    };
    for row in rows {
        let name: String = field(&row, 0, "table name")?;
        let kind: String = field(&row, 1, "table kind")?;
        let collation: Option<String> = field(&row, 3, "table collation")?;
        let charset = if let Some(collation) = &collation {
            conn.exec_first("SELECT CHARACTER_SET_NAME FROM information_schema.COLLATION_CHARACTER_SET_APPLICABILITY WHERE COLLATION_NAME=?",(collation,)).await?
        } else {
            None
        };
        let mut table = SourceTable {
            name: name.clone(),
            engine: field(&row, 2, &name)?,
            is_view: kind == "VIEW",
            charset,
            collation,
            comment: field(&row, 4, &name)?,
            estimated_rows: field(&row, 5, &name)?,
            next_auto_increment: field(&row, 6, &name)?,
            columns: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            checks: Vec::new(),
        };
        let options: String = field(&row, 7, &name)?;
        if !options.is_empty() {
            catalog.unsupported_objects.push(UnsupportedObject {object:name.clone(),kind:"storage_options".into(),reason:format!("source CREATE_OPTIONS={options}; target storage semantics require explicit omission/replacement")});
        }
        let columns: Vec<Row> = conn
            .exec(include_str!("columns.sql"), (&database, &name))
            .await?;
        for column in columns {
            let column_name: String = field(&column, 0, &name)?;
            let extra: String = field(&column, 6, &name)?;
            let generation: Option<String> = field(&column, 9, &name)?;
            let data_type: String = field(&column, 2, &name)?;
            let mut default: Option<String> = field(&column, 5, &name)?;
            let default_is_expression = extra.contains("DEFAULT_GENERATED")
                || (matches!(data_type.as_str(), "timestamp" | "datetime")
                    && default.as_deref().is_some_and(|value| {
                        value.to_ascii_lowercase().starts_with("current_timestamp")
                    }));
            let binary_default = matches!(data_type.as_str(), "binary" | "varbinary");
            if !table.is_view
                && default.is_some()
                && !default_is_expression
                && generation.as_deref().is_none_or(str::is_empty)
                && (binary_default
                    || matches!(data_type.as_str(), "char" | "varchar" | "enum" | "set"))
            {
                // MySQL's cached COLUMN_DEFAULT truncates binary at NUL and replaces
                // supplementary Unicode. DEFAULT reads the literal field buffer instead.
                // Avoid outer-join NULL extension of the default field buffer.
                let native_default = format!(
                    "DEFAULT(my2pg_default_source.{})",
                    quote_ident(&column_name)
                );
                let projection = if binary_default {
                    format!("HEX({native_default})")
                } else {
                    format!("CONVERT({native_default} USING utf8mb4)")
                };
                let query = format!(
                    "SELECT {projection} FROM {}.{} AS my2pg_default_source LIMIT 1",
                    quote_ident(&database),
                    quote_ident(&name)
                );
                let context = format!("literal default {database}.{name}.{column_name}");
                let row: Row = match conn.query_first(query).await? {
                    Some(row) => row,
                    None => {
                        // An aggregate produces one row from an empty table without
                        // NULL-extending DEFAULT. A concurrent insert cannot alter
                        // the schema default, though COUNT may then scan its rows.
                        let query = format!(
                            "SELECT {projection},COUNT(*) FROM {}.{} AS my2pg_default_source",
                            quote_ident(&database),
                            quote_ident(&name)
                        );
                        conn.query_first(query)
                            .await?
                            .ok_or_else(|| SourceError::Metadata(context.clone()))?
                    }
                };
                let value = field::<Option<String>>(&row, 0, &context)?
                    .ok_or(SourceError::Metadata(context))?;
                default = Some(if binary_default {
                    format!("0x{value}")
                } else {
                    value
                });
            }
            table.columns.push(SourceColumn {
                name: column_name,
                ordinal: field(&column, 1, &name)?,
                data_type,
                column_type: field(&column, 3, &name)?,
                nullable: field::<String>(&column, 4, &name)? == "YES",
                default,
                default_is_expression,
                extra,
                charset: field(&column, 7, &name)?,
                collation: field(&column, 8, &name)?,
                generation_expression: generation.filter(|s| !s.is_empty()),
                numeric_precision: field(&column, 10, &name)?,
                numeric_scale: field(&column, 11, &name)?,
                datetime_precision: field(&column, 12, &name)?,
                character_length: field(&column, 13, &name)?,
                comment: field(&column, 14, &name)?,
            });
        }
        let index_sql = include_str!("indexes.sql")
            .replace(
                "/*expression*/",
                if version >= (8, 0, 13) {
                    "EXPRESSION"
                } else {
                    "NULL"
                },
            )
            .replace(
                "/*visible*/",
                if version >= (8, 0, 0) {
                    "IS_VISIBLE"
                } else {
                    "'YES'"
                },
            );
        let parts: Vec<Row> = conn.exec(index_sql, (&database, &name)).await?;
        let mut indexes: BTreeMap<String, SourceIndex> = BTreeMap::new();
        for part in parts {
            let index_name: String = field(&part, 0, &name)?;
            let index = indexes.entry(index_name.clone()).or_insert(SourceIndex {
                name: index_name.clone(),
                primary: index_name == "PRIMARY",
                unique: field::<u8>(&part, 1, &name)? == 0,
                kind: field(&part, 2, &name)?,
                parts: Vec::new(),
            });
            let ordinal: u32 = field(&part, 3, &name)?;
            if ordinal as usize != index.parts.len() + 1 {
                return Err(SourceError::Metadata(format!(
                    "{name}.{index_name} part order"
                )));
            }
            index.parts.push(IndexPart {
                column: field(&part, 4, &name)?,
                expression: field(&part, 7, &name)?,
                prefix_length: field(&part, 5, &name)?,
                descending: field::<Option<String>>(&part, 6, &name)?.as_deref() == Some("D"),
            });
            if field::<String>(&part, 8, &name)? == "NO" && ordinal == 1 {
                catalog.unsupported_objects.push(UnsupportedObject {object:format!("{name}.{index_name}"),kind:"invisible_index".into(),reason:"PostgreSQL has no equivalent optimizer-invisible index; require explicit decision".into()});
            }
        }
        table.indexes = indexes.into_values().collect();
        let keys: Vec<Row> = conn
            .exec(include_str!("foreign_keys.sql"), (&database, &name))
            .await?;
        let mut foreign_keys: BTreeMap<String, SourceForeignKey> = BTreeMap::new();
        for key in keys {
            let key_name: String = field(&key, 0, &name)?;
            let fk = foreign_keys
                .entry(key_name.clone())
                .or_insert(SourceForeignKey {
                    name: key_name,
                    columns: Vec::new(),
                    referenced_schema: field(&key, 2, &name)?,
                    referenced_table: field(&key, 3, &name)?,
                    referenced_columns: Vec::new(),
                    on_update: field(&key, 5, &name)?,
                    on_delete: field(&key, 6, &name)?,
                });
            fk.columns.push(field(&key, 1, &name)?);
            fk.referenced_columns.push(field(&key, 4, &name)?);
        }
        table.foreign_keys = foreign_keys.into_values().collect();
        if version >= (8, 0, 16) {
            let checks: Vec<Row> = conn
                .exec(include_str!("checks.sql"), (&database, &name))
                .await?;
            for check in checks {
                table.checks.push(SourceCheck {
                    name: field(&check, 0, &name)?,
                    expression: field(&check, 1, &name)?,
                    enforced: field::<String>(&check, 2, &name)? == "YES",
                });
            }
        }
        catalog.tables.push(table);
    }
    let objects: Vec<Row> = conn
        .exec(
            include_str!("unsupported.sql"),
            (&database, &database, &database, &database),
        )
        .await?;
    for object in objects {
        let kind: String = field(&object, 1, "unsupported object kind")?;
        catalog.unsupported_objects.push(UnsupportedObject {
            object: field(&object, 0, "unsupported object name")?,
            reason: format!("source {kind} is inventoried for manual migration"),
            kind,
        });
    }
    catalog
        .unsupported_objects
        .sort_by(|a, b| (&a.kind, &a.object).cmp(&(&b.kind, &b.object)));
    Ok(catalog)
}

pub fn enforce_consistency(
    catalog: &SourceCatalog,
    consistency: Consistency,
) -> Result<(), SourceError> {
    if consistency == Consistency::SingleSnapshot
        && catalog
            .tables
            .iter()
            .any(|table| !table.is_view && !table.engine.eq_ignore_ascii_case("innodb"))
    {
        return Err(SourceError::Configuration(
            "single_snapshot requires InnoDB; nontransactional tables require frozen source".into(),
        ));
    }
    Ok(())
}

pub async fn start_snapshot(conn: &mut Conn) -> Result<(), SourceError> {
    conn.query_drop("SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .await?;
    conn.query_drop("START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY")
        .await?;
    Ok(())
}

pub fn select_sql(database: &str, table: &TablePlan) -> Result<String, SourceError> {
    let columns: Vec<_> = table
        .columns
        .iter()
        .filter(|column| column.copy)
        .map(|column| {
            let name = quote_ident(&column.source_name);
            if !column.enum_labels.is_empty() {
                // Ordinal zero is MySQL's invalid sentinel, distinct from a
                // declared empty label even when the target override is text.
                return format!("CAST({name} AS UNSIGNED)");
            }
            match column.kind {
                ValueKind::Set => format!("CAST({name} AS UNSIGNED)"),
                ValueKind::Geometry => {
                    format!("CONCAT('SRID=',ST_SRID({name}),';',ST_AsText({name}))")
                }
                ValueKind::Text | ValueKind::Enum if column.charset.is_some() => {
                    format!("CAST({name} AS BINARY)")
                }
                _ => name,
            }
        })
        .collect();
    if columns.is_empty() {
        return Err(SourceError::Configuration(
            "selected table has no readable COPY columns".into(),
        ));
    }
    Ok(format!(
        "SELECT {} FROM {}.{}",
        columns.join(","),
        quote_ident(database),
        quote_ident(&table.source_name)
    ))
}

pub async fn table_stream<'a>(
    conn: &'a mut Conn,
    database: &str,
    table: &TablePlan,
) -> Result<ResultSetStream<'a, 'a, 'static, Row, BinaryProtocol>, SourceError> {
    Ok(conn.exec_stream(select_sql(database, table)?, ()).await?)
}

/// Return exact integer primary-key endpoints without narrowing unsigned values.
/// The aggregate is rendered as decimal text so MySQL driver's signed integer
/// conversion cannot truncate BIGINT UNSIGNED values above i64::MAX.
pub async fn integer_key_bounds(
    conn: &mut Conn,
    database: &str,
    table: &str,
    key: &str,
    unsigned: bool,
) -> Result<Option<(IntegerKey, IntegerKey)>, SourceError> {
    let sql = format!(
        "SELECT CAST(MIN({key}) AS CHAR), CAST(MAX({key}) AS CHAR) FROM {database}.{table}",
        key = quote_ident(key),
        database = quote_ident(database),
        table = quote_ident(table),
    );
    let bounds: Option<(Option<String>, Option<String>)> = conn.exec_first(sql, ()).await?;
    let Some((minimum, maximum)) = bounds else {
        return Err(SourceError::Metadata(
            "integer key bounds returned no row".into(),
        ));
    };
    let (minimum, maximum) = match (minimum, maximum) {
        (None, None) => return Ok(None),
        (Some(minimum), Some(maximum)) => (minimum, maximum),
        _ => {
            return Err(SourceError::Metadata(
                "integer key bounds are incomplete".into(),
            ));
        }
    };
    if unsigned {
        let minimum = minimum
            .parse::<u64>()
            .map_err(|_| SourceError::Metadata("unsigned integer key minimum is invalid".into()))?;
        let maximum = maximum
            .parse::<u64>()
            .map_err(|_| SourceError::Metadata("unsigned integer key maximum is invalid".into()))?;
        Ok(Some((
            IntegerKey::Unsigned(minimum),
            IntegerKey::Unsigned(maximum),
        )))
    } else {
        let minimum = minimum
            .parse::<i64>()
            .map_err(|_| SourceError::Metadata("signed integer key minimum is invalid".into()))?;
        let maximum = maximum
            .parse::<i64>()
            .map_err(|_| SourceError::Metadata("signed integer key maximum is invalid".into()))?;
        Ok(Some((
            IntegerKey::Signed(minimum),
            IntegerKey::Signed(maximum),
        )))
    }
}

/// Stream one closed key interval through the same planned projection and
/// binary protocol as the whole-table reader.
pub async fn table_range_stream<'a>(
    conn: &'a mut Conn,
    database: &str,
    table: &TablePlan,
    key: &str,
    range: IntegerRange,
) -> Result<ResultSetStream<'a, 'a, 'static, Row, BinaryProtocol>, SourceError> {
    let (lower, upper) = match (range.lower, range.upper) {
        (IntegerKey::Signed(lower), IntegerKey::Signed(upper)) => {
            (Value::Int(lower), Value::Int(upper))
        }
        (IntegerKey::Unsigned(lower), IntegerKey::Unsigned(upper)) => {
            (Value::UInt(lower), Value::UInt(upper))
        }
        _ => {
            return Err(SourceError::Metadata(
                "range endpoint signedness does not match".into(),
            ));
        }
    };
    let sql = format!(
        "{} WHERE {} >= ? AND {} <= ?",
        select_sql(database, table)?,
        quote_ident(key),
        quote_ident(key),
    );
    Ok(conn.exec_stream(sql, (lower, upper)).await?)
}

pub fn raw_values(row: Row, max_row_bytes: usize) -> Result<Vec<RawValue>, SourceError> {
    let observed = row
        .columns_ref()
        .len()
        .saturating_mul(std::mem::size_of::<Value>())
        .saturating_add(
            (0..row.len())
                .filter_map(|i| row.as_ref(i))
                .map(|value| {
                    if let Value::Bytes(bytes) = value {
                        bytes.len()
                    } else {
                        0
                    }
                })
                .sum::<usize>(),
        );
    if observed > max_row_bytes {
        return Err(SourceError::OversizedRow {
            limit: max_row_bytes,
            observed,
        });
    }
    Ok(row
        .unwrap()
        .into_iter()
        .map(|value| match value {
            Value::NULL => RawValue::Null,
            Value::Bytes(bytes) => RawValue::Bytes(bytes),
            Value::Int(v) => RawValue::Int(v),
            Value::UInt(v) => RawValue::UInt(v),
            Value::Float(v) => RawValue::Float(v),
            Value::Double(v) => RawValue::Double(v),
            Value::Date(year, month, day, hour, minute, second, micros) => RawValue::Date {
                year,
                month,
                day,
                hour,
                minute,
                second,
                micros,
            },
            Value::Time(negative, days, hour, minute, second, micros) => RawValue::Time {
                negative,
                days,
                hour,
                minute,
                second,
                micros,
            },
        })
        .collect())
}

pub async fn cancel(conn: SourceConnection) -> Result<(), SourceError> {
    conn.disconnect().await
}

fn close_now(conn: Conn) -> Result<(), SourceError> {
    // Pinned mysql_async marks Conn disconnected before its first await; normal
    // disconnect then drains clean_dirty. Poll once to set that ownership flag,
    // then drop the future at its first pending I/O. This closes the socket and
    // prevents Conn::drop from scheduling an unbounded cleanup/drain task.
    // A real slow-result cancellation test guards this version-specific detail.
    match conn.disconnect().now_or_never() {
        Some(result) => result.map_err(Into::into),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifiers_and_server_scope_are_exact() {
        assert_eq!(quote_ident("t` x"), "`t`` x`");
        assert_eq!(
            validate_server("8.4.9", "MySQL Community Server - GPL").unwrap(),
            (8, 4, 9)
        );
        for (version, comment) in [
            ("10.6.0-MariaDB", "mariadb.org"),
            ("8.0.1", "TiDB Server"),
            ("8.0.1", "Percona Server"),
            ("9.0.0", "MySQL Community Server - GPL"),
            ("8.4", "MySQL Community Server - GPL"),
        ] {
            assert!(validate_server(version, comment).is_err());
        }
    }
}
