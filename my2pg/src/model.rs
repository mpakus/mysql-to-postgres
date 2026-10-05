//! Shared migration state. Nothing in these structures contains resolved credentials.
use crate::config::{Consistency, ExistingPolicy, MigrationMode, VerificationMode};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::OwnedSemaphorePermit;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceCatalog {
    pub database: String,
    pub server_version: String,
    pub server_comment: String,
    #[serde(default = "one")]
    pub auto_increment_increment: u64,
    #[serde(default = "one")]
    pub auto_increment_offset: u64,
    pub tables: Vec<SourceTable>,
    #[serde(default)]
    pub unsupported_objects: Vec<UnsupportedObject>,
}

fn one() -> u64 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceTable {
    pub name: String,
    pub engine: String,
    pub is_view: bool,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub comment: String,
    pub estimated_rows: Option<u64>,
    pub next_auto_increment: Option<u64>,
    pub columns: Vec<SourceColumn>,
    pub indexes: Vec<SourceIndex>,
    pub foreign_keys: Vec<SourceForeignKey>,
    pub checks: Vec<SourceCheck>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceColumn {
    pub name: String,
    pub ordinal: u32,
    pub data_type: String,
    pub column_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub default_is_expression: bool,
    pub extra: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub generation_expression: Option<String>,
    pub numeric_precision: Option<u32>,
    pub numeric_scale: Option<u32>,
    pub datetime_precision: Option<u32>,
    pub character_length: Option<u64>,
    pub comment: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceIndex {
    pub name: String,
    pub primary: bool,
    pub unique: bool,
    pub kind: String,
    pub parts: Vec<IndexPart>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexPart {
    pub column: Option<String>,
    pub expression: Option<String>,
    pub prefix_length: Option<u32>,
    pub descending: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceCheck {
    pub name: String,
    pub expression: String,
    pub enforced: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnsupportedObject {
    pub object: String,
    pub kind: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TargetCatalog {
    pub server_version: String,
    pub schema_exists: bool,
    #[serde(default)]
    pub can_create_schema: bool,
    #[serde(default)]
    pub can_use_schema: bool,
    #[serde(default)]
    pub can_create_objects: bool,
    pub tables: Vec<TargetTable>,
    pub enums: BTreeMap<String, Vec<String>>,
    pub extensions: Vec<String>,
    pub external_dependencies: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<TargetDependency>,
    #[serde(default)]
    pub occupied_names: BTreeMap<String, String>,
    #[serde(default)]
    pub occupied_types: BTreeSet<String>,
    /// All database event triggers, including functions outside the target schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_triggers: Option<Vec<TargetTriggerPart>>,
    /// None is uninspected; an empty successful namespace inventory is Some([]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_parts: Option<Vec<TargetTypePart>>,
    /// Inspection roots never imply that a dependent is selected for mutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_graph: Option<TargetDependencyGraph>,
    /// None is uninspected; each requested namespace has its own positive facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespaces: Option<Vec<TargetNamespaceObserved>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deparse: Option<TargetDeparseObserved>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetNamespaceObserved {
    pub schema: String,
    pub oid: Option<u32>,
    pub schema_exists: bool,
    pub can_use: bool,
    pub can_create_objects: bool,
    pub occupied_names: BTreeMap<String, String>,
    pub occupied_types: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetDeparseObserved {
    pub extra_float_digits: i32,
    pub interval_style: String,
}

/// Namespace × type × enum-label × domain-constraint facts, without aggregation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TargetTypePart {
    pub namespace_oid: u32,
    pub namespace_name: String,
    pub namespace_can_use: bool,
    pub namespace_can_create: bool,
    pub type_oid: Option<u32>,
    pub type_owner_role_oid: Option<u32>,
    pub type_relation_oid: Option<u32>,
    pub type_element_oid: Option<u32>,
    pub type_array_oid: Option<u32>,
    pub type_base_oid: Option<u32>,
    pub type_collation_oid: Option<u32>,
    pub extension_oid: Option<u32>,
    pub type_schema: Option<String>,
    pub type_name: Option<String>,
    pub type_owner_role: Option<String>,
    pub type_kind: Option<String>,
    pub type_category: Option<String>,
    pub type_alignment: Option<String>,
    pub type_storage: Option<String>,
    pub type_default_expression: Option<String>,
    pub type_comment: Option<String>,
    pub extension_name: Option<String>,
    pub type_defined: Option<bool>,
    pub type_by_value: Option<bool>,
    pub domain_not_null: Option<bool>,
    pub can_use: Option<bool>,
    pub can_alter: Option<bool>,
    pub type_length: Option<i16>,
    pub type_modifier: Option<i32>,
    pub domain_array_dimensions: Option<i32>,
    pub enum_label_oid: Option<u32>,
    pub enum_label: Option<String>,
    pub enum_sort_order: Option<f32>,
    pub domain_constraint_oid: Option<u32>,
    pub domain_constraint_name: Option<String>,
    pub domain_constraint_kind: Option<String>,
    pub domain_constraint_definition: Option<String>,
    pub domain_constraint_expression: Option<String>,
    pub domain_constraint_validated: Option<bool>,
    pub domain_constraint_deferrable: Option<bool>,
    pub domain_constraint_initially_deferred: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetDependencyGraph {
    pub database_oid: u32,
    pub relation_root_count: u64,
    pub type_root_count: u64,
    pub event_trigger_count: u64,
    /// Flat node × incident-edge facts; header-only empty results have no parts.
    pub parts: Vec<TargetDependencyGraphPart>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetDependencyGraphPart {
    pub node_class_oid: u32,
    pub node_oid: u32,
    pub node_sub_id: i32,
    pub node_catalog: String,
    pub node_type: String,
    pub node_schema: Option<String>,
    pub node_name: Option<String>,
    pub node_identity: String,
    pub relation_root: bool,
    pub type_root: bool,
    pub global_event_root: bool,
    pub node_owner_relation_oid: Option<u32>,
    pub node_owner_role_oid: Option<u32>,
    pub node_extension_oid: Option<u32>,
    pub node_owner_column_ordinal: Option<i16>,
    pub node_owner_relation_schema: Option<String>,
    pub node_owner_relation_name: Option<String>,
    pub node_owner_role: Option<String>,
    pub node_extension_name: Option<String>,
    pub node_relation_kind: Option<String>,
    pub node_type_kind: Option<String>,
    pub node_constraint_kind: Option<String>,
    pub node_function_kind: Option<String>,
    pub node_function_definition: Option<String>,
    pub node_event_enabled: Option<String>,
    pub node_event: Option<String>,
    pub node_function_body_parsed: Option<bool>,
    pub edge: Option<TargetDependencyEdge>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetDependencyEdge {
    pub dependent_class_oid: u32,
    pub referenced_class_oid: u32,
    pub dependency: TargetObjectDependency,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetDependency {
    pub kind: String,
    pub name: String,
    pub dependent_schema: String,
    pub dependent_table: String,
    pub referenced_schema: String,
    pub referenced_table: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetTable {
    pub schema: String,
    pub name: String,
    pub columns: Vec<TargetColumn>,
    pub row_count: Option<u64>,
    /// Positively observed ordinary table with no inheritance/partition hierarchy.
    #[serde(default)]
    pub ordinary_standalone: bool,
    #[serde(default)]
    pub can_insert: bool,
    #[serde(default)]
    pub can_select: bool,
    #[serde(default)]
    pub can_alter: bool,
    #[serde(default)]
    pub can_truncate: bool,
    /// Separate proof for ACCESS EXCLUSIVE LOCK; None means uninspected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_lock: Option<bool>,
    #[serde(default)]
    pub row_security_active: bool,
    /// None means uninspected; Some with empty vectors is a positive inventory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<TargetTableObserved>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetTableObserved {
    pub oid: u32,
    pub owner: String,
    pub relation_kind: String,
    pub access_method: Option<String>,
    pub persistence: String,
    pub comment: Option<String>,
    pub row_security_enabled: bool,
    pub row_security_forced: bool,
    pub replica_identity: String,
    pub columns: Vec<TargetColumnObserved>,
    pub index_parts: Vec<TargetIndexPart>,
    pub constraint_parts: Vec<TargetConstraintPart>,
    pub sequence_parts: Vec<TargetSequencePart>,
    pub trigger_parts: Vec<TargetTriggerPart>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetObjectDependency {
    pub dependency_catalog: Option<String>,
    pub dependency_direction: Option<String>,
    pub dependency_kind: Option<String>,
    pub dependent_catalog: Option<String>,
    pub dependent_oid: Option<u32>,
    pub dependent_sub_id: Option<i32>,
    pub referenced_catalog: Option<String>,
    pub referenced_oid: Option<u32>,
    pub referenced_sub_id: Option<i32>,
    pub dependent_type: Option<String>,
    pub dependent_schema: Option<String>,
    pub dependent_name: Option<String>,
    pub dependent_identity: Option<String>,
    pub referenced_type: Option<String>,
    pub referenced_schema: Option<String>,
    pub referenced_name: Option<String>,
    pub referenced_identity: Option<String>,
}

/// Observed state is not transactional with catalog metadata; no sequence is advanced.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SequenceStateObservation {
    Known { last_value: i64, is_called: bool },
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetSequencePart {
    pub sequence_oid: u32,
    pub sequence_type_oid: u32,
    pub sequence_schema: String,
    pub sequence_name: String,
    pub type_schema: String,
    pub type_name: String,
    pub sequence_owner_role: String,
    pub bound_column_name: String,
    pub bound_identity_mode: String,
    pub bound_column_ordinal: i16,
    pub bound_default_expression: Option<String>,
    pub owner_table_schema: Option<String>,
    pub owner_table_name: Option<String>,
    pub owner_column_name: Option<String>,
    pub ownership_dependency: Option<String>,
    pub extension_name: Option<String>,
    pub default_reference_schema: Option<String>,
    pub default_reference_table: Option<String>,
    pub default_reference_column: Option<String>,
    pub default_reference_expression: Option<String>,
    pub owner_column_ordinal: Option<i16>,
    pub default_reference_ordinal: Option<i16>,
    pub start_value: i64,
    pub increment_by: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cache_size: i64,
    pub cycle: bool,
    pub can_alter: bool,
    pub can_use: bool,
    pub can_select: bool,
    pub can_update: bool,
    pub dependency: TargetObjectDependency,
    pub state: SequenceStateObservation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetTriggerPart {
    pub scope: String,
    pub trigger_name: String,
    pub enabled: String,
    pub function_schema: String,
    pub function_name: String,
    pub function_arguments: String,
    pub function_identity: String,
    pub function_definition: String,
    pub function_language: String,
    pub function_volatility: String,
    pub function_owner_role: String,
    pub trigger_oid: u32,
    pub function_oid: u32,
    pub internal: Option<bool>,
    pub table_oid: Option<u32>,
    pub constraint_oid: Option<u32>,
    pub trigger_parent_oid: Option<u32>,
    pub table_schema: Option<String>,
    pub table_name: Option<String>,
    pub constraint_name: Option<String>,
    pub definition: Option<String>,
    pub event: Option<String>,
    pub dependency_subject: Option<String>,
    pub event_flags: Option<i16>,
    pub event_tags: Option<Vec<String>>,
    pub function_config: Option<Vec<String>>,
    pub function_security_definer: bool,
    pub dependency: TargetObjectDependency,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetColumnObserved {
    pub ordinal: i16,
    pub name: String,
    pub data_type: String,
    pub type_oid: u32,
    pub type_schema: String,
    pub type_name: String,
    pub type_kind: String,
    pub type_base_oid: u32,
    pub type_element_oid: u32,
    pub type_modifier: i32,
    pub array_dimensions: i16,
    pub nullable: bool,
    pub identity_mode: String,
    pub generated_kind: String,
    pub default_expression: Option<String>,
    pub generation_expression: Option<String>,
    pub comment: Option<String>,
    pub collation_oid: Option<u32>,
    pub collation_schema: Option<String>,
    pub collation_name: Option<String>,
    pub collation_provider: Option<String>,
    pub collation_deterministic: Option<bool>,
    pub collation_locale: Option<String>,
    pub collation_version: Option<String>,
    pub collation_actual_version: Option<String>,
}

/// One actual index part; header facts repeat without aggregating identifiers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetIndexPart {
    pub index_oid: u32,
    pub index_schema: String,
    pub index_name: String,
    pub table_oid: u32,
    pub table_schema: String,
    pub table_name: String,
    pub method: String,
    pub is_unique: bool,
    pub is_primary: bool,
    pub is_exclusion: bool,
    pub valid: bool,
    pub ready: bool,
    pub live: bool,
    pub immediate: bool,
    pub nulls_not_distinct: bool,
    pub predicate: Option<String>,
    pub constraint_name: Option<String>,
    pub constraint_deferrable: Option<bool>,
    pub constraint_initially_deferred: Option<bool>,
    pub ordinal: i64,
    pub column_ordinal: i16,
    pub column_name: Option<String>,
    pub expression: Option<String>,
    pub included: bool,
    pub descending: Option<bool>,
    pub nulls_first: Option<bool>,
    pub operator_class_oid: Option<u32>,
    pub operator_class_schema: Option<String>,
    pub operator_class_name: Option<String>,
    pub default_operator_class: Option<bool>,
    pub collation_oid: Option<u32>,
    pub collation_schema: Option<String>,
    pub collation_name: Option<String>,
    pub collation_provider: Option<String>,
    pub collation_deterministic: Option<bool>,
}

/// A zero-key constraint still has one row with ordinal/column None.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetConstraintPart {
    pub constraint_oid: u32,
    pub name: String,
    pub kind: String,
    pub definition: String,
    pub validated: bool,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub no_inherit: bool,
    pub match_type: String,
    pub update_action: String,
    pub delete_action: String,
    pub ordinal: Option<i64>,
    pub column_name: Option<String>,
    pub column_expression: Option<String>,
    pub check_expression: Option<String>,
    pub referenced_table_oid: Option<u32>,
    pub referenced_column: Option<String>,
    pub referenced_schema: Option<String>,
    pub referenced_table: Option<String>,
    pub supporting_index_oid: Option<u32>,
    pub supporting_index_schema: Option<String>,
    pub supporting_index_name: Option<String>,
    pub delete_set_columns: Option<Vec<String>>,
    pub pk_fk_operator_oid: Option<u32>,
    pub pk_fk_operator: Option<String>,
    pub pk_pk_operator_oid: Option<u32>,
    pub pk_pk_operator: Option<String>,
    pub fk_fk_operator_oid: Option<u32>,
    pub fk_fk_operator: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub generated: bool,
    pub identity: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub version: u32,
    pub source_database: String,
    pub source_version: String,
    pub target_schema: String,
    pub target_version: String,
    pub consistency: Consistency,
    pub mode: MigrationMode,
    pub verification: VerificationMode,
    #[serde(default)]
    pub on_existing: ExistingPolicy,
    #[serde(default = "yes")]
    pub reset_sequences: bool,
    pub tables: Vec<TablePlan>,
    pub ddl: Vec<DdlStep>,
    pub diagnostics: Vec<Diagnostic>,
    pub exclusions: Vec<String>,
    pub transformations: Vec<String>,
    pub hooks: Vec<HookPlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TablePlan {
    pub id: String,
    pub source_name: String,
    pub target_schema: String,
    pub target_name: String,
    pub engine: String,
    pub is_view: bool,
    pub estimated_rows: Option<u64>,
    pub next_auto_increment: Option<u64>,
    pub columns: Vec<ColumnPlan>,
    /// Original source column names used for reader identity; target keys are in `structure`.
    pub primary_key: Vec<String>,
    #[serde(default)]
    pub structure: SchemaExpectations,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaExpectations {
    pub complete: bool,
    pub indexes: Vec<IndexExpectation>,
    pub foreign_keys: Vec<ForeignKeyExpectation>,
    pub checks: Vec<CheckExpectation>,
    #[serde(default)]
    pub sequences: Vec<SequenceExpectation>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SequenceExpectation {
    pub column: String,
    pub increment: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cycle: bool,
    pub next_minimum: u64,
    /// Explicit reset=false contract; None is not proof of preserved state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_state: Option<SequenceStateObservation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexExpectation {
    pub name: String,
    pub columns: Vec<String>,
    #[serde(default)]
    pub expressions: Vec<Option<String>>,
    pub unique: bool,
    pub primary: bool,
    pub descending: Vec<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForeignKeyExpectation {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckExpectation {
    pub name: String,
    pub expression: String,
    #[serde(default)]
    pub set_membership: Option<SetMembershipExpectation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetMembershipExpectation {
    pub column: String,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnPlan {
    pub source_name: String,
    pub target_name: String,
    pub source_type: String,
    pub target_type: String,
    pub kind: ValueKind,
    pub nullable: bool,
    pub default_sql: Option<String>,
    pub identity: bool,
    pub generated_expression: Option<String>,
    pub copy: bool,
    pub transform: Option<String>,
    pub charset: Option<String>,
    pub enum_labels: Vec<String>,
    pub set_labels: Vec<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    SignedInteger,
    UnsignedInteger,
    Decimal,
    Float,
    Text,
    Binary,
    Date,
    Datetime,
    Timestamp,
    Interval,
    Bit(u16),
    Boolean,
    Json,
    Enum,
    Set,
    Geometry,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DdlPhase {
    Prepare,
    Finalize,
    Sequence,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DdlStep {
    pub phase: DdlPhase,
    pub table_id: Option<String>,
    pub object: String,
    pub sql: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookPlan {
    pub before: bool,
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub stage: String,
    pub object: Option<String>,
    pub severity: Severity,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RawValue {
    Null,
    Bytes(Vec<u8>),
    Int(i64),
    UInt(u64),
    Float(f32),
    Double(f64),
    Date {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        micros: u32,
    },
    Time {
        negative: bool,
        days: u32,
        hour: u8,
        minute: u8,
        second: u8,
        micros: u32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RowLocator {
    pub ordinal: u64,
    pub key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RowPosition {
    pub start: usize,
    pub end: usize,
    pub locator: RowLocator,
}

#[derive(Debug)]
pub struct BatchStorage {
    pub bytes: Bytes,
    pub permit: OwnedSemaphorePermit,
}

#[derive(Clone, Debug)]
pub struct EncodedBatch {
    pub storage: Arc<BatchStorage>,
    pub rows: Vec<RowPosition>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Complete,
    Partial,
    Failed,
    Cancelled,
    Indeterminate,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunEvent {
    pub version: u32,
    pub kind: String,
    pub run_id: String,
    pub table: Option<String>,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub progress: Option<ProgressEstimate>,
    pub phase: String,
    pub elapsed_millis: u64,
    pub committed_rows: u64,
    #[serde(default)]
    pub committed_bytes: u64,
    pub rejected_rows: u64,
    pub diagnostic: Option<Diagnostic>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EstimateQuality {
    Unknown,
    Metadata,
    Measured,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgressEstimate {
    pub estimated_rows: Option<u64>,
    pub quality: EstimateQuality,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    #[default]
    NotRun,
    Complete,
    Different,
    Unsupported,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationReport {
    pub mode: VerificationMode,
    pub status: VerificationStatus,
    pub tables_checked: usize,
    pub differences: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableReport {
    pub id: String,
    pub source_name: String,
    pub target_schema: String,
    pub target_name: String,
    pub status: RunStatus,
    pub rows_read: u64,
    pub committed_rows: u64,
    #[serde(default)]
    pub committed_bytes: u64,
    #[serde(default)]
    pub copy_elapsed_millis: u64,
    pub rejected_rows: u64,
    pub unresolved_rows: u64,
    pub indeterminate_rows: u64,
    pub transformations: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunReport {
    pub version: u32,
    pub run_id: String,
    #[serde(default)]
    pub plan_sha256: String,
    #[serde(default)]
    pub source_endpoint_sha256: String,
    #[serde(default)]
    pub target_endpoint_sha256: String,
    pub status: RunStatus,
    pub consistency: Consistency,
    pub mode: MigrationMode,
    pub tables: Vec<TableReport>,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub failed_steps: Vec<String>,
    pub verification: VerificationReport,
    pub elapsed_millis: u64,
    pub artifact_dir: String,
}

impl TableReport {
    pub fn accounted_rows(&self) -> Option<u64> {
        self.committed_rows
            .checked_add(self.rejected_rows)?
            .checked_add(self.unresolved_rows)?
            .checked_add(self.indeterminate_rows)
    }
}

impl RunReport {
    pub fn exit_code(&self) -> u8 {
        if matches!(
            self.status,
            RunStatus::Failed | RunStatus::Indeterminate | RunStatus::Running
        ) || self
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
            || self.tables.iter().any(|t| {
                matches!(
                    t.status,
                    RunStatus::Failed | RunStatus::Indeterminate | RunStatus::Running
                ) || t.indeterminate_rows > 0
                    || (matches!(t.status, RunStatus::Complete | RunStatus::Partial)
                        && (t.unresolved_rows > 0 || t.accounted_rows() != Some(t.rows_read)))
            })
            || !self.failed_steps.is_empty()
            || self.verification.status == VerificationStatus::Error
        {
            1
        } else if self.status == RunStatus::Cancelled
            || self.tables.iter().any(|t| t.status == RunStatus::Cancelled)
        {
            130
        } else if self.verification.mode != VerificationMode::None
            && (self.verification.status != VerificationStatus::Complete
                || !self.verification.differences.is_empty()
                || self.verification.tables_checked != self.tables.len())
        {
            4
        } else if self
            .tables
            .iter()
            .any(|t| t.rejected_rows > 0 || t.status == RunStatus::Partial)
            || self.status == RunStatus::Partial
        {
            3
        } else {
            0
        }
    }
}
