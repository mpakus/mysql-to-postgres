use super::*;
use regex::Regex;
use serde::Serialize;
use std::{collections::BTreeSet, fmt, fs, path::Path};
use url::Url;

/// Diagnostics deliberately exclude parser messages, URLs and input excerpts.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ConfigError {
    pub code: String,
    pub field: String,
    pub message: String,
    pub span: Option<SourceSpan>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.code, self.field, self.message)?;
        if let Some(span) = &self.span {
            write!(f, " (line {}, column {})", span.line, span.column)?;
        }
        Ok(())
    }
}
impl std::error::Error for ConfigError {}

fn error(field: &str, message: &str) -> ConfigError {
    ConfigError {
        code: "CFG_INVALID".into(),
        field: field.into(),
        message: message.into(),
        span: None,
    }
}

/// Never serialize credentials or expose them through ordinary debug formatting.
pub struct SecretUrl(String);
impl SecretUrl {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SecretUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretUrl([redacted])")
    }
}
impl fmt::Display for SecretUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}
#[derive(Debug)]
pub struct ResolvedCredentials {
    pub source: SecretUrl,
    pub target: SecretUrl,
}

pub fn load(path: impl AsRef<Path>) -> Result<MigrationConfig, ConfigError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|_| {
        error(
            "config",
            "cannot read a UTF-8 configuration file; check path and permissions",
        )
    })?;
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|_| error("config", "cannot determine working directory"))?
            .join(path)
    };
    parse(&text, absolute.parent().unwrap_or(Path::new(".")))
}

pub fn check(path: impl AsRef<Path>) -> Result<MigrationConfig, ConfigError> {
    let config = load(path)?;
    resolve_credentials(&config)?;
    Ok(config)
}

pub fn parse(text: &str, directory: &Path) -> Result<MigrationConfig, ConfigError> {
    let mut config: MigrationConfig = toml::from_str(text).map_err(|e: toml::de::Error| {
        let mut diagnostic = error("config", "invalid TOML, missing required field, unknown field, or unsupported value/type; check the indicated location against the configuration schema");
        diagnostic.code = "CFG_PARSE".into();
        diagnostic.span = e.span().map(|range| span(text, range.start, range.end));
        diagnostic
    })?;
    // Deserialization defaults cannot distinguish an explicit choice from omission.
    if config.migration.mode == MigrationMode::DataOnly {
        let value: toml::Value = toml::from_str(text).expect("already parsed valid TOML");
        if value
            .get("migration")
            .and_then(|m| m.get("reset_sequences"))
            .is_none()
        {
            return Err(with_span(
                error(
                    "migration.reset_sequences",
                    "data_only requires an explicit true or false sequence-reset choice",
                ),
                text,
            ));
        }
    }
    validate(&config).map_err(|e| with_span(e, text))?;
    resolve_paths(&mut config, directory);
    validate_files(&config).map_err(|e| with_span(e, text))?;
    Ok(config)
}

fn span(text: &str, start: usize, end: usize) -> SourceSpan {
    let start = start.min(text.len());
    let prefix = &text[..text.floor_char_boundary(start)];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, rest)| rest)
        .chars()
        .count()
        + 1;
    SourceSpan {
        start,
        end: end.min(text.len()),
        line,
        column,
    }
}

fn with_span(mut diagnostic: ConfigError, text: &str) -> ConfigError {
    let (wanted_section, wanted_key) = diagnostic
        .field
        .rsplit_once('.')
        .unwrap_or(("", &diagnostic.field));
    let mut section = "";
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.trim_matches(['[', ']']);
        } else if section == wanted_section
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == wanted_key)
        {
            diagnostic.span = Some(span(text, offset, offset + line.trim_end().len()));
            break;
        }
        offset += line.len();
    }
    diagnostic
}

pub fn validate(config: &MigrationConfig) -> Result<(), ConfigError> {
    if config.version != 1 {
        return Err(error("version", "supported configuration version is 1"));
    }
    reference(&config.source.url_env, &config.source.url_file, "source")?;
    reference(&config.target.url_env, &config.target.url_file, "target")?;
    destination_schema(&config.target.schema, "target.schema")?;
    tls(
        config.source.tls_mode,
        &config.source.ca_file,
        &config.source.client_cert,
        &config.source.client_key,
        "source",
    )?;
    tls(
        config.target.tls_mode,
        &config.target.ca_file,
        &config.target.client_cert,
        &config.target.client_key,
        "target",
    )?;
    sessions(&config.source.session, true)?;
    sessions(&config.target.session, false)?;
    let m = &config.migration;
    for (field, value) in [
        ("table_workers", m.table_workers),
        ("readers_per_table", m.readers_per_table),
        ("index_workers", m.index_workers),
        ("batch_rows", m.batch_rows),
        ("batch_bytes", m.batch_bytes),
        ("queue_batches", m.queue_batches),
        ("memory_bytes", m.memory_bytes),
        ("max_row_bytes", m.max_row_bytes),
    ] {
        if value == 0 {
            return Err(error(&format!("migration.{field}"), "must be positive"));
        }
    }
    if m.rows_per_range == 0 {
        return Err(error("migration.rows_per_range", "must be positive"));
    }
    if m.max_key_span == Some(0) {
        return Err(error("migration.max_key_span", "must be positive when set"));
    }
    if m.readers_per_table > 1 {
        if m.max_key_span.is_none() {
            return Err(error(
                "migration.max_key_span",
                "multiple readers per table require an explicit positive key span",
            ));
        }
        if config.source.consistency != Consistency::Frozen {
            return Err(error(
                "source.consistency",
                "multiple range readers require a frozen source",
            ));
        }
    }
    if config.source.consistency == Consistency::SingleSnapshot
        && (m.table_workers != 1 || m.readers_per_table != 1)
    {
        return Err(error(
            "migration.table_workers",
            "single_snapshot requires exactly one table worker and one reader per table",
        ));
    }
    if m.max_row_bytes > m.batch_bytes {
        return Err(error(
            "migration.max_row_bytes",
            "must not exceed batch_bytes; increase batch_bytes for larger rows",
        ));
    }
    let layout = crate::pipeline::scheduler::MemoryLayout::compute(m).map_err(|_| {
            error(
                "migration.memory_bytes",
                "resource-limit arithmetic overflow or reservation exceeds u32; reduce row/batch/queue limits",
            )
        })?;
    if m.mode != MigrationMode::SchemaOnly && m.memory_bytes < layout.pipeline_reservation {
        return Err(error(
            "migration.memory_bytes",
            "too small for queued, active and filling batches, row workspace and locators; increase memory_bytes or reduce row/batch/queue limits",
        ));
    }
    if m.memory_bytes > tokio::sync::Semaphore::MAX_PERMITS {
        return Err(error(
            "migration.memory_bytes",
            "exceeds the supported retained-data budget",
        ));
    }
    match (m.on_row_error, m.max_rejected_rows) {
        (RowErrorPolicy::Reject, 0) => {
            return Err(error(
                "migration.max_rejected_rows",
                "reject mode requires a positive explicit reject limit",
            ));
        }
        (RowErrorPolicy::Stop, n) if n != 0 => {
            return Err(error(
                "migration.max_rejected_rows",
                "stop mode requires a zero reject limit",
            ));
        }
        _ => {}
    }
    match (config.target.on_existing, m.mode) {
        (ExistingPolicy::Error, MigrationMode::DataOnly) => {
            return Err(error(
                "target.on_existing",
                "data_only requires an explicit append or truncate policy",
            ));
        }
        (ExistingPolicy::Append, mode) if mode != MigrationMode::DataOnly => {
            return Err(error(
                "target.on_existing",
                "append requires data_only mode",
            ));
        }
        (ExistingPolicy::Recreate, MigrationMode::DataOnly) => {
            return Err(error(
                "target.on_existing",
                "recreate contradicts data_only mode",
            ));
        }
        (ExistingPolicy::Truncate, MigrationMode::SchemaOnly) => {
            return Err(error(
                "target.on_existing",
                "truncate contradicts schema_only mode",
            ));
        }
        _ => {}
    }
    for (field, values) in [
        ("include", &config.tables.include),
        ("exclude", &config.tables.exclude),
        ("views", &config.tables.views),
    ] {
        let mut unique = BTreeSet::new();
        for name in values {
            identifier(name, &format!("tables.{field}"), false)?;
            if !unique.insert(name) {
                return Err(error(
                    &format!("tables.{field}"),
                    "duplicate exact source name",
                ));
            }
        }
    }
    for (field, patterns) in [
        ("include_regex", &config.tables.include_regex),
        ("exclude_regex", &config.tables.exclude_regex),
    ] {
        for pattern in patterns {
            Regex::new(pattern).map_err(|_| error(&format!("tables.{field}"), "invalid or unsupported regular expression; use Rust regex syntax without backreferences/lookaround"))?;
        }
    }
    let mut renamed = BTreeSet::new();
    for mapping in &config.tables.rename {
        identifier(&mapping.source, "tables.rename.source", false)?;
        identifier(&mapping.target, "tables.rename.target", true)?;
        if let Some(schema) = &mapping.schema {
            destination_schema(schema, "tables.rename.schema")?;
        }
        if !renamed.insert(&mapping.source) {
            return Err(error(
                "tables.rename.source",
                "each source table may be renamed only once",
            ));
        }
    }
    for rule in &config.cast {
        cast(rule)?;
    }
    let mut overridden = BTreeSet::new();
    for rule in &config.overrides {
        if rule.object.trim().is_empty() || rule.object.contains('\0') {
            return Err(error(
                "overrides.object",
                "requires a nonempty exact object ID without NUL",
            ));
        }
        if !overridden.insert(&rule.object) {
            return Err(error(
                "overrides.object",
                "each object may have only one override",
            ));
        }
        let actions = usize::from(rule.omit)
            + usize::from(rule.target_expression.is_some())
            + usize::from(rule.target_sql.is_some())
            + usize::from(rule.materialize.is_some());
        if actions != 1 {
            return Err(error(
                "overrides",
                "choose exactly one action: omit, target_expression, target_sql, or materialize",
            ));
        }
        if let Some(expression) = &rule.target_expression {
            fragment(expression, "overrides.target_expression")?;
        }
        if let Some(sql) = &rule.target_sql
            && (sql.trim().is_empty() || sql.contains('\0'))
        {
            return Err(error(
                "overrides.target_sql",
                "explicit reviewed target SQL must be nonempty and contain no NUL",
            ));
        }
    }
    if config.report.directory.as_os_str().is_empty() {
        return Err(error(
            "report.directory",
            "requires a nonempty directory path",
        ));
    }
    Ok(())
}

fn reference(env: &Option<String>, file: &Option<PathBuf>, side: &str) -> Result<(), ConfigError> {
    if env.is_some() == file.is_some() {
        return Err(error(
            &format!("{side}.url_env"),
            "set exactly one of url_env or url_file",
        ));
    }
    if let Some(name) = env
        && (name.is_empty()
            || !name.bytes().enumerate().all(|(i, b)| {
                b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
            }))
    {
        return Err(error(
            &format!("{side}.url_env"),
            "use a valid environment-variable name",
        ));
    }
    if file
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err(error(
            &format!("{side}.url_file"),
            "requires a nonempty credential-file path",
        ));
    }
    Ok(())
}
fn identifier(name: &str, field: &str, target: bool) -> Result<(), ConfigError> {
    if name.is_empty() || name.contains('\0') {
        return Err(error(field, "requires a nonempty identifier without NUL"));
    }
    if target && name.len() > 63 {
        return Err(error(
            field,
            "PostgreSQL identifiers must fit within 63 UTF-8 bytes; use an explicit shorter mapping",
        ));
    }
    Ok(())
}
fn destination_schema(name: &str, field: &str) -> Result<(), ConfigError> {
    identifier(name, field, true)?;
    if name.trim().is_empty() || name.starts_with("pg_") || name == "information_schema" {
        return Err(error(
            field,
            "choose a nonblank destination schema outside PostgreSQL system schemas",
        ));
    }
    Ok(())
}
fn tls(
    mode: TlsMode,
    ca: &Option<PathBuf>,
    cert: &Option<PathBuf>,
    key: &Option<PathBuf>,
    side: &str,
) -> Result<(), ConfigError> {
    if cert.is_some() != key.is_some() {
        return Err(error(
            &format!("{side}.client_cert"),
            "client_cert and client_key must be supplied together",
        ));
    }
    if mode == TlsMode::Disable && (ca.is_some() || cert.is_some()) {
        return Err(error(
            &format!("{side}.tls_mode"),
            "TLS certificate fields contradict tls_mode=disable",
        ));
    }
    Ok(())
}
fn sessions(settings: &BTreeMap<String, String>, source: bool) -> Result<(), ConfigError> {
    let allowed: &[&str] = if source {
        &[
            "time_zone",
            "sql_mode",
            "net_read_timeout",
            "net_write_timeout",
            "wait_timeout",
            "interactive_timeout",
            "group_concat_max_len",
        ]
    } else {
        &[
            "application_name",
            "timezone",
            "statement_timeout",
            "lock_timeout",
            "idle_in_transaction_session_timeout",
            "work_mem",
            "maintenance_work_mem",
            "client_min_messages",
            "standard_conforming_strings",
            "datestyle",
        ]
    };
    for (key, value) in settings {
        if !allowed.contains(&key.as_str()) {
            return Err(error(
                if source {
                    "source.session"
                } else {
                    "target.session"
                },
                "unsupported session setting; connection policy and consistency settings cannot be overridden",
            ));
        }
        let field = format!("{}.session.{key}", if source { "source" } else { "target" });
        if value.is_empty() || value.len() > 1024 || value.contains('\0') {
            return Err(error(
                &field,
                "session value must be nonempty, without NUL, and at most 1024 bytes",
            ));
        }
        if (source && key == "time_zone" && value != "+00:00")
            || (!source && key == "timezone" && value != "UTC")
        {
            return Err(error(
                &field,
                "UTC session time is required for timestamp fidelity",
            ));
        }
        if source && key != "time_zone" && key != "sql_mode" && value.parse::<u64>().is_err() {
            return Err(error(
                &field,
                "numeric source session setting requires an unsigned integer",
            ));
        }
        if key == "standard_conforming_strings" && value != "on" {
            return Err(error(&field, "standard_conforming_strings must remain on"));
        }
        if key == "datestyle" && value != "ISO, YMD" {
            return Err(error(&field, "DateStyle must remain ISO, YMD"));
        }
    }
    Ok(())
}

pub const TRANSFORMS: &[&str] = &[
    "zero-dates-to-null",
    "tinyint-to-boolean",
    "bits-to-boolean",
    "empty-string-to-null",
    "remove-null-characters",
    "right-trim",
    "byte-vector-to-bytea",
    "bytes-to-pg-bytea",
    "hex-to-bytea",
    "set-to-array",
    "year-to-integer",
    "tinyint-to-integer",
    "int-to-ip",
];
fn cast(rule: &CastRule) -> Result<(), ConfigError> {
    if rule.source_type.is_none() && rule.source_column.is_none() {
        return Err(error(
            "cast",
            "each cast requires source_type or source_column",
        ));
    }
    if rule.source_table.is_some() != rule.source_column.is_some() {
        return Err(error(
            "cast.source_column",
            "column selectors require both source_table and source_column",
        ));
    }
    for (field, name) in [
        ("source_type", &rule.source_type),
        ("source_table", &rule.source_table),
        ("source_column", &rule.source_column),
    ] {
        if let Some(name) = name {
            identifier(name, &format!("cast.{field}"), false)?;
        }
    }
    if let Some(source_type) = &rule.source_type
        && ![
            "tinyint",
            "smallint",
            "mediumint",
            "int",
            "integer",
            "bigint",
            "decimal",
            "numeric",
            "float",
            "double",
            "real",
            "bit",
            "boolean",
            "bool",
            "char",
            "varchar",
            "tinytext",
            "text",
            "mediumtext",
            "longtext",
            "binary",
            "varbinary",
            "tinyblob",
            "blob",
            "mediumblob",
            "longblob",
            "date",
            "datetime",
            "timestamp",
            "time",
            "year",
            "enum",
            "set",
            "json",
            "geometry",
            "point",
            "linestring",
            "polygon",
            "multipoint",
            "multilinestring",
            "multipolygon",
            "geometrycollection",
        ]
        .contains(&source_type.to_ascii_lowercase().as_str())
    {
        return Err(error("cast.source_type", "unknown MySQL type selector"));
    }
    if rule.target_type.is_none()
        && rule.transform.is_none()
        && rule.charset.is_none()
        && !rule.drop_default
        && !rule.drop_not_null
        && !rule.drop_typemod
    {
        return Err(error(
            "cast",
            "cast rule must select a target type, named transform, charset or explicit drop policy",
        ));
    }
    if let Some(transform) = &rule.transform
        && !TRANSFORMS.contains(&transform.as_str())
    {
        return Err(error(
            "cast.transform",
            "unsupported named transform; arbitrary code and plugins are not accepted",
        ));
    }
    if let Some(kind) = &rule.target_type {
        validate_target_type(kind)?;
    }
    if rule
        .precision
        .as_ref()
        .is_some_and(|guard| guard.value > 65)
    {
        return Err(error(
            "cast.precision",
            "MySQL numeric precision guards must be within 0..=65",
        ));
    }
    if rule.scale.as_ref().is_some_and(|guard| guard.value > 30) {
        return Err(error(
            "cast.scale",
            "MySQL numeric scale guards must be within 0..=30",
        ));
    }
    if let Some(charset) = &rule.charset
        && ![
            "utf8",
            "utf8mb3",
            "utf8mb4",
            "latin1",
            "ascii",
            "windows-1252",
            "iso-8859-1",
            "binary",
        ]
        .contains(&charset.as_str())
    {
        return Err(error(
            "cast.charset",
            "unsupported explicit charset; use an implemented strict decoder",
        ));
    }
    Ok(())
}

/// Finite built-in type grammar; custom types require an explicit reviewed override.
pub fn validate_target_type(value: &str) -> Result<(), ConfigError> {
    let pattern = r"(?i)^(?:(smallint|integer|int|bigint|real|double precision|boolean|bool|bytea|text|jsonb|uuid|inet|date|money|numeric|decimal|char|character|varchar|character varying|bit|bit varying|varbit|timestamptz|timetz|interval)(\([0-9]+(,[0-9]+)?\))?|(?:timestamp|time)(?:\([0-9]+\))?(?: (?:with|without) time zone)?)(\[\])?$";
    if !Regex::new(pattern)
        .expect("static type grammar")
        .is_match(value.trim())
    {
        return Err(error(
            "cast.target_type",
            "unsupported target type grammar; use a supported PostgreSQL type or reviewed object override",
        ));
    }
    let lower = value.trim().to_ascii_lowercase();
    if let Some((base, rest)) = lower.split_once('(') {
        let params: Vec<u32> = rest
            .split(')')
            .next()
            .unwrap()
            .split(',')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| {
                error(
                    "cast.target_type",
                    "type modifier is outside its numeric range",
                )
            })?;
        let valid = match base {
            "numeric" | "decimal" => {
                params[0] >= 1 && params[0] <= 1000 && (params.len() == 1 || params[1] <= params[0])
            }
            "char" | "character" | "varchar" | "character varying" | "bit" | "bit varying"
            | "varbit" => params.len() == 1 && params[0] > 0 && params[0] <= 10_485_760,
            "timestamp"
            | "timestamp without time zone"
            | "timestamp with time zone"
            | "timestamptz"
            | "time"
            | "time without time zone"
            | "time with time zone"
            | "timetz"
            | "interval" => params.len() == 1 && params[0] <= 6,
            _ => false,
        };
        if !valid {
            return Err(error(
                "cast.target_type",
                "unsupported or out-of-range PostgreSQL type modifier",
            ));
        }
    }
    Ok(())
}

fn fragment(value: &str, field: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(error(field, "requires a nonempty expression without NUL"));
    }
    let mut quote = None;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(current) = quote {
            if ch == current {
                if chars.peek() == Some(&current) {
                    chars.next();
                } else {
                    quote = None;
                }
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch == ';'
            || (ch == '-' && chars.peek() == Some(&'-'))
            || (ch == '/' && chars.peek() == Some(&'*'))
            || ch == '$'
        {
            return Err(error(
                field,
                "expression must be one SQL fragment without statements, comments or dollar quoting; use a reviewed SQL hook for scripts",
            ));
        }
    }
    if quote.is_some() {
        return Err(error(field, "expression has an unterminated quoted value"));
    }
    Ok(())
}

fn resolve_paths(config: &mut MigrationConfig, directory: &Path) {
    fn relative(path: &mut PathBuf, directory: &Path) {
        if path.is_relative() {
            *path = directory.join(&*path);
        }
    }
    for path in [
        &mut config.source.url_file,
        &mut config.source.ca_file,
        &mut config.source.client_cert,
        &mut config.source.client_key,
        &mut config.target.url_file,
        &mut config.target.ca_file,
        &mut config.target.client_cert,
        &mut config.target.client_key,
        &mut config.target.passfile,
    ]
    .into_iter()
    .flatten()
    {
        relative(path, directory);
    }
    for path in config
        .hooks
        .before
        .iter_mut()
        .chain(&mut config.hooks.after)
    {
        relative(path, directory);
    }
    relative(&mut config.report.directory, directory);
}
fn validate_files(config: &MigrationConfig) -> Result<(), ConfigError> {
    for (field, path) in [
        ("source.ca_file", &config.source.ca_file),
        ("source.client_cert", &config.source.client_cert),
        ("source.client_key", &config.source.client_key),
        ("target.ca_file", &config.target.ca_file),
        ("target.client_cert", &config.target.client_cert),
        ("target.client_key", &config.target.client_key),
        ("target.passfile", &config.target.passfile),
    ] {
        if let Some(path) = path {
            readable_file(path, field)?;
        }
    }
    for (field, paths) in [
        ("hooks.before", &config.hooks.before),
        ("hooks.after", &config.hooks.after),
    ] {
        for path in paths {
            readable_file(path, field)?;
            if fs::metadata(path)
                .map(|metadata| metadata.len() > super::MAX_SQL_HOOK_BYTES as u64)
                .unwrap_or(true)
            {
                return Err(error(field, "SQL hook exceeds the 1 MiB size limit"));
            }
        }
    }
    Ok(())
}
fn readable_file(path: &Path, field: &str) -> Result<(), ConfigError> {
    let metadata = fs::metadata(path).map_err(|_| {
        error(
            field,
            "cannot access the referenced file; check path and permissions",
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(error(
            field,
            "referenced path must be a nonempty regular file",
        ));
    }
    fs::File::open(path)
        .map_err(|_| error(field, "cannot read the referenced file; check permissions"))?;
    Ok(())
}

pub fn resolve_credentials(config: &MigrationConfig) -> Result<ResolvedCredentials, ConfigError> {
    resolve_credentials_with(config, |name| std::env::var(name).ok())
}
/// Inject environment lookup for tests, avoiding global environment mutation.
pub fn resolve_credentials_with(
    config: &MigrationConfig,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Result<ResolvedCredentials, ConfigError> {
    validate(config)?;
    fn get(
        env: &Option<String>,
        file: &Option<PathBuf>,
        side: &str,
        lookup: &mut impl FnMut(&str) -> Option<String>,
    ) -> Result<SecretUrl, ConfigError> {
        let value = if let Some(name) = env {
            lookup(name).ok_or_else(|| {
                error(
                    &format!("{side}.url_env"),
                    &format!(
                        "environment variable {name} is missing or non-UTF-8; set it explicitly"
                    ),
                )
            })?
        } else {
            let path = file.as_ref().expect("reference validated");
            readable_file(path, &format!("{side}.url_file"))?;
            if fs::metadata(path)
                .map_err(|_| {
                    error(
                        &format!("{side}.url_file"),
                        "cannot inspect credential file",
                    )
                })?
                .len()
                > 16_386
            {
                return Err(error(
                    &format!("{side}.url_file"),
                    "credential file exceeds the 16384-byte URL limit",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if fs::metadata(path)
                    .map_err(|_| {
                        error(
                            &format!("{side}.url_file"),
                            "cannot inspect credential-file permissions",
                        )
                    })?
                    .permissions()
                    .mode()
                    & 0o077
                    != 0
                {
                    return Err(error(
                        &format!("{side}.url_file"),
                        "credential file must be private to its owner (chmod 600)",
                    ));
                }
            }
            fs::read_to_string(path)
                .map_err(|_| {
                    error(
                        &format!("{side}.url_file"),
                        "credential file must be readable UTF-8",
                    )
                })?
                .trim_end_matches(['\r', '\n'])
                .to_owned()
        };
        validate_url(
            &value,
            side,
            if env.is_some() { "url_env" } else { "url_file" },
        )?;
        Ok(SecretUrl(value))
    }
    // Source is resolved first; an excluded source never accesses target credentials.
    let source = get(
        &config.source.url_env,
        &config.source.url_file,
        "source",
        &mut lookup,
    )?;
    let target = get(
        &config.target.url_env,
        &config.target.url_file,
        "target",
        &mut lookup,
    )?;
    Ok(ResolvedCredentials { source, target })
}

fn validate_url(value: &str, side: &str, reference_field: &str) -> Result<(), ConfigError> {
    let field = format!("{side}.{reference_field}");
    if value.is_empty()
        || value.len() > 16_384
        || value.bytes().any(|b| b.is_ascii_whitespace() || b == 0)
    {
        return Err(error(
            &field,
            "connection URL must be nonempty, at most 16384 bytes and without unescaped whitespace/NUL",
        ));
    }
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
                || &bytes[index + 1..index + 3] == b"00")
        {
            return Err(error(
                &field,
                "invalid percent escape or encoded NUL in connection URL",
            ));
        }
    }
    let uri = Url::parse(value).map_err(|_| error(&field, "invalid connection URL; use a percent-encoded network URI with explicit user, host and database"))?;
    let expected = if side == "source" {
        "mysql"
    } else {
        "postgresql"
    };
    if uri.scheme() != expected && !(side == "target" && uri.scheme() == "postgres") {
        return Err(error(
            &field,
            if side == "source" {
                "only mysql:// Oracle MySQL source URLs are supported"
            } else {
                "target requires postgresql:// or postgres://"
            },
        ));
    }
    if uri.host_str().is_none_or(str::is_empty)
        || uri.username().is_empty()
        || uri.path().strip_prefix('/').is_none_or(str::is_empty)
        || uri.path()[1..].contains('/')
        || uri.fragment().is_some()
        || uri.port() == Some(0)
    {
        return Err(error(
            &field,
            "URL requires explicit user, host, one database name and a nonzero port; fragments are not supported",
        ));
    }
    // Session/TLS configuration has one authoritative TOML location. URI options
    // can otherwise override verification or trigger undocumented driver behavior.
    if uri.query().is_some() {
        return Err(error(
            &field,
            "URL query options are unsupported; configure TLS and session settings in TOML",
        ));
    }
    if side == "target" {
        let parsed: tokio_postgres::Config = value
            .parse()
            .map_err(|_| error(&field, "invalid PostgreSQL connection URL"))?;
        if !matches!(
            parsed.get_hosts(),
            [tokio_postgres::config::Host::Tcp(host)] if !host.is_empty() && !host.starts_with('/')
        ) {
            return Err(error(
                &field,
                "target requires exactly one TCP host; Unix sockets and multi-host failover are unsupported",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const MINIMAL: &str = "version = 1\n[source]\nurl_env = 'MY2PG_SOURCE'\nconsistency = 'frozen'\n[target]\nurl_env = 'MY2PG_TARGET'\nschema = 'legacy'\n";
    fn config(extra: &str) -> Result<MigrationConfig, ConfigError> {
        parse(&format!("{MINIMAL}{extra}"), Path::new("/tmp/config-home"))
    }
    fn credentials(source: &str, target: &str) -> Result<ResolvedCredentials, ConfigError> {
        resolve_credentials_with(&config("").unwrap(), |name| match name {
            "MY2PG_SOURCE" => Some(source.into()),
            "MY2PG_TARGET" => Some(target.into()),
            _ => None,
        })
    }

    #[test]
    fn planning_example_and_nested_schema_are_executable() {
        let text = include_str!("../../docs/examples/mysql-to-postgres.toml");
        let parsed = parse(text, Path::new("/tmp/example")).unwrap();
        assert_eq!(parsed.migration.table_workers, 4);
        assert_eq!(
            parsed.cast[0].transform.as_deref(),
            Some("zero-dates-to-null")
        );
        assert_eq!(parsed.report.directory, Path::new("/tmp/example/./runs"));
        assert!(config("[[cast]]\nsource_type='decimal'\ntarget_type='numeric(65,30)'\nprecision={op='ge', value=40}\nscale={op='le', value=30}\n[[overrides]]\nobject='index.orders.search'\nomit=true\n").is_ok());
    }

    #[test]
    fn unknown_fields_duplicate_settings_types_and_versions_fail_with_spans() {
        for text in [
            MINIMAL.replace("version = 1", "version = 9"),
            MINIMAL.replace("version = 1", "version = 1\nversion = 1"),
            MINIMAL.replace(
                "schema = 'legacy'",
                "schema = 'legacy'\npassword='super_secret'",
            ),
            format!("{MINIMAL}[migration]\nbatch_rows = 'super_secret'\n"),
            format!("{MINIMAL}[[cast]]\nsource_type='int'\nunsigned='super_secret'\n"),
            format!(
                "{MINIMAL}[[cast]]\nsource_type='decimal'\ntarget_type='numeric'\nprecision={{op='lisp-eval',value=3}}\n"
            ),
        ] {
            let diagnostic = parse(&text, Path::new("/tmp")).unwrap_err();
            assert!(diagnostic.span.is_some(), "{diagnostic:?}");
            assert!(!format!("{diagnostic:?}").contains("super_secret"));
        }
    }

    #[test]
    fn required_identity_and_excluded_sources_fail_before_target_lookup() {
        assert!(
            config("[[tables.rename]]\nsource='orders'\ntarget='orders'\nschema='pg_catalog'\n")
                .is_err()
        );
        for source in [
            "sqlite:///tmp/db",
            "postgresql://u:p@host/db",
            "mssql://u:p@host/db",
            "csv:///tmp/file",
            "http://host/data",
            "s3://bucket/key",
            "jdbc:mysql://host/db",
            "mariadb://u:p@host/db",
            "mysql://host/db",
            "mysql://u:p@host",
            "mysql://u:p@host:0/db",
            "mysql://u:p@host/db#fragment",
            "mysql://u:p@host/db?ssl-mode=DISABLED",
            "mysql://u:p@host/db extra",
            "mysql://u:p%ZZ@host/db",
            "mysql://u:p@host/%00",
        ] {
            let mut target_looked_up = false;
            let error = resolve_credentials_with(&config("").unwrap(), |name| {
                if name == "MY2PG_SOURCE" {
                    Some(source.into())
                } else {
                    target_looked_up = true;
                    None
                }
            })
            .unwrap_err();
            assert!(!target_looked_up, "{source}");
            assert!(!error.message.contains(source));
        }
        assert!(
            credentials(
                "mysql://u:secret%40word@host:3306/db",
                "postgresql://u:secret@host:5432/db"
            )
            .is_ok()
        );
        assert!(credentials("mysql://u:p@host/db", "mysql://u:p@host/db").is_err());
        assert!(parse(&MINIMAL.replace("schema = 'legacy'", ""), Path::new("/tmp")).is_err());
        assert!(
            parse(
                &MINIMAL.replace("schema = 'legacy'", "schema=''"),
                Path::new("/tmp")
            )
            .is_err()
        );
        assert!(
            parse(
                &MINIMAL.replace("schema = 'legacy'", "schema='pg_catalog'"),
                Path::new("/tmp")
            )
            .is_err()
        );
        assert!(
            parse(
                &MINIMAL.replace("schema = 'legacy'", "schema='A\"B; name'"),
                Path::new("/tmp")
            )
            .is_ok()
        );
    }

    #[test]
    fn numeric_sessions_and_canonical_target_settings_validate_offline() {
        for name in [
            "net_read_timeout",
            "net_write_timeout",
            "wait_timeout",
            "interactive_timeout",
            "group_concat_max_len",
        ] {
            for value in ["t14-session-secret", "-1", "18446744073709551616"] {
                let diagnostic =
                    config(&format!("[source.session]\n{name}='{value}'\n")).unwrap_err();
                assert_eq!(diagnostic.field, format!("source.session.{name}"));
                assert!(!format!("{diagnostic} {diagnostic:?}").contains(value));
            }
            assert!(config(&format!("[source.session]\n{name}='17'\n")).is_ok());
        }
        for extra in [
            "[target.session]\ntimezone='Etc/UTC'\n",
            "[target.session]\ntimezone='+00:00'\n",
            "[target.session]\ndatestyle='iso, ymd'\n",
        ] {
            assert!(config(extra).is_err());
        }
        assert!(config("[target.session]\ntimezone='UTC'\ndatestyle='ISO, YMD'\n").is_ok());
    }

    #[test]
    fn target_urls_require_one_tcp_host_and_preserve_ipv6_offline() {
        for target in [
            "postgresql://u:t14-offline-secret@[::1]:1/db",
            "postgres://u:t14-offline-secret@[2001:db8::1]/db",
            "postgresql://u:t14-offline-secret@%6cocalhost:1/db",
        ] {
            let resolved = credentials("mysql://u:p@host:1/db", target).unwrap();
            assert_eq!(resolved.target.expose(), target);
            assert!(!format!("{resolved:?}").contains("t14-offline-secret"));
        }
        let parsed: tokio_postgres::Config = "postgresql://u:p@[::1]:1/db".parse().unwrap();
        assert_eq!(
            parsed.get_hosts(),
            [tokio_postgres::config::Host::Tcp("::1".into())]
        );
        for target in [
            "postgresql://u:t14-offline-secret@%2Ftmp/db",
            "postgres://u:t14-offline-secret@%2fvar%2frun%2fpostgresql/db",
            "postgresql://u:t14-offline-secret@localhost,127.0.0.1/db",
            "postgresql://u:t14-offline-secret@localhost,/db",
            "postgresql://u:t14-offline-secret@[::1],localhost/db",
            "postgresql://u:t14-offline-secret@%FF/db",
        ] {
            let diagnostic = credentials("mysql://u:p@host:1/db", target).unwrap_err();
            assert_eq!(diagnostic.field, "target.url_env");
            assert_eq!(diagnostic.code, "CFG_INVALID");
            assert!(!format!("{diagnostic} {diagnostic:?}").contains("t14-offline-secret"));
            assert!(!diagnostic.message.contains(target));
        }
    }

    #[test]
    fn credentials_are_redacted_even_in_debug_and_error_outputs() {
        let resolved = credentials(
            "mysql://u:secret%40word@host/db",
            "postgresql://u:target-secret@host/db",
        )
        .unwrap();
        assert_eq!(resolved.source.expose(), "mysql://u:secret%40word@host/db");
        assert!(!format!("{resolved:?} {}", resolved.source).contains("secret"));
        let missing = resolve_credentials_with(&config("").unwrap(), |_| None).unwrap_err();
        assert!(missing.message.contains("MY2PG_SOURCE"));
        assert!(
            !credentials(
                "mysql://u:secret@host/db?evil=secret",
                "postgresql://u:p@host/db"
            )
            .unwrap_err()
            .to_string()
            .contains("secret")
        );
    }

    #[test]
    fn limits_consistency_and_existing_policies_are_validated_together() {
        for field in [
            "table_workers",
            "readers_per_table",
            "index_workers",
            "batch_rows",
            "batch_bytes",
            "queue_batches",
            "memory_bytes",
            "max_row_bytes",
            "rows_per_range",
        ] {
            assert!(
                config(&format!("[migration]\n{field}=0\n")).is_err(),
                "{field}"
            );
        }
        assert!(config("[migration]\nbatch_bytes=1024\nmax_row_bytes=2048\n").is_err());
        assert!(config("[migration]\nmemory_bytes=1024\n").is_err());
        assert!(config("[migration]\non_row_error='reject'\n").is_err());
        assert!(config("[migration]\nmax_rejected_rows=1\n").is_err());
        assert!(config("[migration]\non_row_error='reject'\nmax_rejected_rows=3\n").is_ok());
        let snapshot = MINIMAL.replace("'frozen'", "'single_snapshot'");
        assert!(
            parse(
                &format!("{snapshot}[migration]\ntable_workers=2\n"),
                Path::new("/tmp")
            )
            .is_err()
        );
        assert!(
            parse(
                &format!("{snapshot}[migration]\nreaders_per_table=2\n"),
                Path::new("/tmp")
            )
            .is_err()
        );
        assert!(parse(&snapshot, Path::new("/tmp")).is_ok());
        assert!(config("[migration]\nmode='data_only'\n").is_err());
        assert!(config("[migration]\nmode='data_only'\nreset_sequences=false\n").is_err());
        let append = MINIMAL.replace(
            "schema = 'legacy'",
            "schema = 'legacy'\non_existing='append'",
        );
        assert!(parse(&append, Path::new("/tmp")).is_err());
        assert!(
            parse(
                &format!("{append}[migration]\nmode='data_only'\nreset_sequences=false\n"),
                Path::new("/tmp")
            )
            .is_ok()
        );
    }

    #[test]
    fn range_readers_require_an_explicit_positive_span_and_frozen_source() {
        assert_eq!(config("").unwrap().migration.max_key_span, None);
        assert_eq!(
            config("[migration]\nmax_key_span=128\n")
                .unwrap()
                .migration
                .max_key_span,
            Some(128)
        );
        assert!(config("[migration]\nmax_key_span=0\n").is_err());
        assert!(config("[migration]\nreaders_per_table=2\n").is_err());
        assert!(config("[migration]\nreaders_per_table=2\nmax_key_span=64\n").is_ok());

        let snapshot = MINIMAL.replace("'frozen'", "'single_snapshot'");
        assert!(
            parse(
                &format!("{snapshot}[migration]\nreaders_per_table=2\nmax_key_span=64\n"),
                Path::new("/tmp")
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn queue_depth_changes_the_offline_minimum_and_schema_only_needs_no_copy_bytes() {
        // W=65,992; B=1,024+2*48; four retained batches => P=70,472.
        let limits = "[migration]\nbatch_rows=2\nbatch_bytes=1024\nmax_row_bytes=64\n";
        let diagnostic = config(&format!("{limits}memory_bytes=70471\n")).unwrap_err();
        assert_eq!(diagnostic.field, "migration.memory_bytes");
        assert!(config(&format!("{limits}memory_bytes=70472\n")).is_ok());
        assert!(config(&format!("{limits}memory_bytes=70472\nqueue_batches=3\n")).is_err());
        assert!(config(&format!("{limits}memory_bytes=1\nmode='schema_only'\n")).is_ok());
        for extra in [
            "batch_bytes=4294967296\nmax_row_bytes=64\n",
            "queue_batches=9223372036854775807\n",
        ] {
            let diagnostic = config(&format!("[migration]\n{extra}")).unwrap_err();
            assert_eq!(diagnostic.field, "migration.memory_bytes");
        }
    }

    #[test]
    fn casts_regex_and_operator_fragments_never_execute_input() {
        for extra in [
            "[tables]\ninclude_regex=['(?=danger)']\n",
            "[[cast]]\nsource_type='datetime'\ntransform='shell-eval'\n",
            "[[cast]]\nsource_type='int'\ntarget_type='integer; DROP SCHEMA public'\n",
            "[[cast]]\nsource_column='a'\ntarget_type='text'\n",
            "[[cast]]\nsource_type='decimal'\ntarget_type='numeric(0,0)'\n",
            "[[cast]]\nsource_type='date'\ntarget_type='timestamp(7)'\n",
            "[[cast]]\nsource_type='decimal'\ntarget_type='numeric'\nprecision={op='ge',value=66}\n",
            "[[overrides]]\nobject='x'\nomit=true\ntarget_sql='SELECT 1'\n",
            "[[overrides]]\nobject='x'\ntarget_expression='a; SELECT 1'\n",
        ] {
            assert!(config(extra).is_err(), "{extra}");
        }
        assert!(
            config("[tables]\ninclude=['strange; \"name']\ninclude_regex=['^orders_[0-9]+$']\n")
                .is_ok()
        );
        assert!(config("[[overrides]]\nobject='generated.orders.label'\ntarget_expression=\"concat('hello; ', id)\"\n").is_ok());
        assert!(validate_target_type("timestamp(6) without time zone").is_ok());
        assert!(validate_target_type("timestamp without time zone(6)").is_err());
    }

    #[test]
    fn tls_and_session_policy_cannot_be_downgraded_indirectly() {
        assert!(
            parse(
                &MINIMAL.replace(
                    "consistency = 'frozen'",
                    "consistency = 'frozen'\ntls_mode='disable'\nca_file='ca.pem'"
                ),
                Path::new("/tmp")
            )
            .is_err()
        );
        assert!(
            parse(
                &MINIMAL.replace(
                    "consistency = 'frozen'",
                    "consistency = 'frozen'\nclient_cert='cert.pem'"
                ),
                Path::new("/tmp")
            )
            .is_err()
        );
        assert!(config("[source.session]\ntime_zone='+03:00'\n").is_err());
        assert!(config("[target.session]\nstandard_conforming_strings='off'\n").is_err());
        assert!(config("[source.session]\ntransaction_isolation='READ-UNCOMMITTED'\n").is_err());
        assert!(
            config("[source.session]\ntime_zone='+00:00'\n[target.session]\ntimezone='UTC'\n")
                .is_ok()
        );
    }

    #[test]
    fn files_resolve_beside_config_and_private_credentials_are_required() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "my2pg-t04-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source.url");
        fs::write(&source, "mysql://u:synthetic@host/db\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        }
        fs::write(directory.join("before.sql"), "SELECT 1;").unwrap();
        let text = MINIMAL.replace("url_env = 'MY2PG_SOURCE'", "url_file = 'source.url'")
            + "[hooks]\nbefore=['before.sql']\n";
        fs::write(directory.join("migration.toml"), &text).unwrap();
        let parsed = load(directory.join("migration.toml")).unwrap();
        assert_eq!(parsed.source.url_file.as_ref().unwrap(), &source);
        assert_eq!(parsed.hooks.before[0], directory.join("before.sql"));
        let resolved =
            resolve_credentials_with(&parsed, |_| Some("postgresql://u:p@host/db".into())).unwrap();
        assert_eq!(resolved.source.expose(), "mysql://u:synthetic@host/db");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(
                resolve_credentials_with(&parsed, |_| Some("postgresql://u:p@host/db".into()))
                    .is_err()
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
