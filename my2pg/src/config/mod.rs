//! Versioned executable configuration; credentials remain references.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

pub const MAX_SQL_HOOK_BYTES: usize = 1024 * 1024;

pub mod import;
mod validation;
pub use validation::{
    ConfigError, ResolvedCredentials, SecretUrl, SourceSpan, TRANSFORMS, check, load, parse,
    resolve_credentials, resolve_credentials_with, validate, validate_target_type,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationConfig {
    pub version: u32,
    pub source: SourceConfig,
    pub target: TargetConfig,
    #[serde(default)]
    pub migration: MigrationOptions,
    #[serde(default)]
    pub tables: TableSelection,
    #[serde(default)]
    pub cast: Vec<CastRule>,
    #[serde(default)]
    pub overrides: Vec<ObjectOverride>,
    #[serde(default)]
    pub hooks: Hooks,
    #[serde(default)]
    pub verification: VerificationConfig,
    #[serde(default)]
    pub report: ReportConfig,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    #[default]
    VerifyFull,
    Disable,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Consistency {
    Frozen,
    SingleSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub url_env: Option<String>,
    pub url_file: Option<PathBuf>,
    pub consistency: Consistency,
    #[serde(default)]
    pub tls_mode: TlsMode,
    pub ca_file: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    #[serde(default)]
    pub session: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub url_env: Option<String>,
    pub url_file: Option<PathBuf>,
    pub schema: String,
    #[serde(default)]
    pub tls_mode: TlsMode,
    pub ca_file: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub passfile: Option<PathBuf>,
    #[serde(default)]
    pub on_existing: ExistingPolicy,
    #[serde(default)]
    pub session: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExistingPolicy {
    #[default]
    Error,
    Recreate,
    Truncate,
    Append,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationMode {
    #[default]
    Full,
    SchemaOnly,
    DataOnly,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierPolicy {
    #[default]
    Preserve,
    Downcase,
    SnakeCase,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RowErrorPolicy {
    #[default]
    Stop,
    Reject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MigrationOptions {
    pub mode: MigrationMode,
    pub identifiers: IdentifierPolicy,
    pub table_workers: usize,
    pub readers_per_table: usize,
    pub index_workers: usize,
    pub batch_rows: usize,
    pub batch_bytes: usize,
    pub queue_batches: usize,
    pub memory_bytes: usize,
    pub max_row_bytes: usize,
    pub on_row_error: RowErrorPolicy,
    pub max_rejected_rows: u64,
    pub reset_sequences: bool,
    pub rows_per_range: u64,
    pub max_key_span: Option<u64>,
}

impl Default for MigrationOptions {
    fn default() -> Self {
        Self {
            mode: MigrationMode::Full,
            identifiers: IdentifierPolicy::Preserve,
            table_workers: 1,
            readers_per_table: 1,
            index_workers: 1,
            batch_rows: 25_000,
            batch_bytes: 16 * 1024 * 1024,
            queue_batches: 2,
            memory_bytes: 512 * 1024 * 1024,
            max_row_bytes: 16 * 1024 * 1024,
            on_row_error: RowErrorPolicy::Stop,
            max_rejected_rows: 0,
            reset_sequences: true,
            rows_per_range: 100_000,
            max_key_span: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TableSelection {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub include_regex: Vec<String>,
    pub exclude_regex: Vec<String>,
    pub rename: Vec<TableRename>,
    pub views: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableRename {
    pub source: String,
    pub target: String,
    pub schema: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CastRule {
    pub source_type: Option<String>,
    pub source_table: Option<String>,
    pub source_column: Option<String>,
    pub target_type: Option<String>,
    pub transform: Option<String>,
    pub drop_default: bool,
    pub drop_not_null: bool,
    pub drop_typemod: bool,
    pub unsigned: Option<bool>,
    pub not_null: Option<bool>,
    pub auto_increment: Option<bool>,
    pub default: Option<String>,
    pub precision: Option<NumericGuard>,
    pub scale: Option<NumericGuard>,
    pub charset: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumericGuard {
    pub op: Comparison,
    pub value: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectOverride {
    pub object: String,
    #[serde(default)]
    pub omit: bool,
    pub target_expression: Option<String>,
    pub target_sql: Option<String>,
    pub materialize: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hooks {
    pub before: Vec<PathBuf>,
    pub after: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    #[default]
    CountsAndSchema,
    Content,
    None,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VerificationConfig {
    pub mode: VerificationMode,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPolicy {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReportConfig {
    pub directory: PathBuf,
    pub console: OutputFormat,
    pub progress: ProgressPolicy,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("runs"),
            console: OutputFormat::Text,
            progress: ProgressPolicy::Auto,
        }
    }
}
