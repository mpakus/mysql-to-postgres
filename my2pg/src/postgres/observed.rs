//! Typed catalog rows; all missing/incorrect binary fields propagate as errors.
use super::{CopyStage, TargetError};
use crate::model::{
    SequenceStateObservation, TargetColumnObserved, TargetConstraintPart, TargetIndexPart,
    TargetObjectDependency, TargetSequencePart, TargetTableObserved, TargetTriggerPart,
};
use std::collections::BTreeMap;
use tokio_postgres::{GenericClient, Row};

fn header(row: &Row) -> Result<TargetTableObserved, tokio_postgres::Error> {
    Ok(TargetTableObserved {
        oid: row.try_get("oid")?,
        owner: row.try_get("owner")?,
        relation_kind: row.try_get("relation_kind")?,
        access_method: row.try_get("access_method")?,
        persistence: row.try_get("persistence")?,
        comment: row.try_get("comment")?,
        row_security_enabled: row.try_get("row_security_enabled")?,
        row_security_forced: row.try_get("row_security_forced")?,
        replica_identity: row.try_get("replica_identity")?,
        columns: Vec::new(),
        index_parts: Vec::new(),
        constraint_parts: Vec::new(),
        sequence_parts: Vec::new(),
        trigger_parts: Vec::new(),
    })
}

pub fn decode_column(row: &Row) -> Result<TargetColumnObserved, tokio_postgres::Error> {
    Ok(TargetColumnObserved {
        ordinal: row.try_get("ordinal")?,
        name: row.try_get("name")?,
        data_type: row.try_get("data_type")?,
        type_oid: row.try_get("type_oid")?,
        type_schema: row.try_get("type_schema")?,
        type_name: row.try_get("type_name")?,
        type_kind: row.try_get("type_kind")?,
        type_base_oid: row.try_get("type_base_oid")?,
        type_element_oid: row.try_get("type_element_oid")?,
        type_modifier: row.try_get("type_modifier")?,
        array_dimensions: row.try_get("array_dimensions")?,
        nullable: row.try_get("nullable")?,
        identity_mode: row.try_get("identity_mode")?,
        generated_kind: row.try_get("generated_kind")?,
        default_expression: row.try_get("default_expression")?,
        generation_expression: row.try_get("generation_expression")?,
        comment: row.try_get("comment")?,
        collation_oid: row.try_get("collation_oid")?,
        collation_schema: row.try_get("collation_schema")?,
        collation_name: row.try_get("collation_name")?,
        collation_provider: row.try_get("collation_provider")?,
        collation_deterministic: row.try_get("collation_deterministic")?,
        collation_locale: row.try_get("collation_locale")?,
        collation_version: row.try_get("collation_version")?,
        collation_actual_version: row.try_get("collation_actual_version")?,
    })
}

pub fn decode_index_part(row: &Row) -> Result<TargetIndexPart, tokio_postgres::Error> {
    Ok(TargetIndexPart {
        index_oid: row.try_get("index_oid")?,
        index_schema: row.try_get("index_schema")?,
        index_name: row.try_get("index_name")?,
        table_oid: row.try_get("table_oid")?,
        table_schema: row.try_get("table_schema")?,
        table_name: row.try_get("table_name")?,
        method: row.try_get("method")?,
        is_unique: row.try_get("is_unique")?,
        is_primary: row.try_get("is_primary")?,
        is_exclusion: row.try_get("is_exclusion")?,
        valid: row.try_get("valid")?,
        ready: row.try_get("ready")?,
        live: row.try_get("live")?,
        immediate: row.try_get("immediate")?,
        nulls_not_distinct: row.try_get("nulls_not_distinct")?,
        predicate: row.try_get("predicate")?,
        constraint_name: row.try_get("constraint_name")?,
        constraint_deferrable: row.try_get("constraint_deferrable")?,
        constraint_initially_deferred: row.try_get("constraint_initially_deferred")?,
        ordinal: row.try_get("ordinal")?,
        column_ordinal: row.try_get("column_ordinal")?,
        column_name: row.try_get("column_name")?,
        expression: row.try_get("expression")?,
        included: row.try_get("included")?,
        descending: row.try_get("descending")?,
        nulls_first: row.try_get("nulls_first")?,
        operator_class_oid: row.try_get("operator_class_oid")?,
        operator_class_schema: row.try_get("operator_class_schema")?,
        operator_class_name: row.try_get("operator_class_name")?,
        default_operator_class: row.try_get("default_operator_class")?,
        collation_oid: row.try_get("collation_oid")?,
        collation_schema: row.try_get("collation_schema")?,
        collation_name: row.try_get("collation_name")?,
        collation_provider: row.try_get("collation_provider")?,
        collation_deterministic: row.try_get("collation_deterministic")?,
    })
}

pub fn decode_constraint_part(row: &Row) -> Result<TargetConstraintPart, tokio_postgres::Error> {
    Ok(TargetConstraintPart {
        constraint_oid: row.try_get("constraint_oid")?,
        name: row.try_get("name")?,
        kind: row.try_get("kind")?,
        definition: row.try_get("definition")?,
        validated: row.try_get("validated")?,
        deferrable: row.try_get("deferrable")?,
        initially_deferred: row.try_get("initially_deferred")?,
        no_inherit: row.try_get("no_inherit")?,
        match_type: row.try_get("match_type")?,
        update_action: row.try_get("update_action")?,
        delete_action: row.try_get("delete_action")?,
        ordinal: row.try_get("ordinal")?,
        column_name: row.try_get("column_name")?,
        column_expression: row.try_get("column_expression")?,
        check_expression: row.try_get("check_expression")?,
        referenced_table_oid: row.try_get("referenced_table_oid")?,
        referenced_column: row.try_get("referenced_column")?,
        referenced_schema: row.try_get("referenced_schema")?,
        referenced_table: row.try_get("referenced_table")?,
        supporting_index_oid: row.try_get("supporting_index_oid")?,
        supporting_index_schema: row.try_get("supporting_index_schema")?,
        supporting_index_name: row.try_get("supporting_index_name")?,
        delete_set_columns: row.try_get("delete_set_columns")?,
        pk_fk_operator_oid: row.try_get("pk_fk_operator_oid")?,
        pk_fk_operator: row.try_get("pk_fk_operator")?,
        pk_pk_operator_oid: row.try_get("pk_pk_operator_oid")?,
        pk_pk_operator: row.try_get("pk_pk_operator")?,
        fk_fk_operator_oid: row.try_get("fk_fk_operator_oid")?,
        fk_fk_operator: row.try_get("fk_fk_operator")?,
    })
}

pub fn decode_dependency(row: &Row) -> Result<TargetObjectDependency, tokio_postgres::Error> {
    Ok(TargetObjectDependency {
        dependency_catalog: row.try_get("dependency_catalog")?,
        dependency_direction: row.try_get("dependency_direction")?,
        dependency_kind: row.try_get("dependency_kind")?,
        dependent_catalog: row.try_get("dependent_catalog")?,
        dependent_oid: row.try_get("dependent_oid")?,
        dependent_sub_id: row.try_get("dependent_sub_id")?,
        referenced_catalog: row.try_get("referenced_catalog")?,
        referenced_oid: row.try_get("referenced_oid")?,
        referenced_sub_id: row.try_get("referenced_sub_id")?,
        dependent_type: row.try_get("dependent_type")?,
        dependent_schema: row.try_get("dependent_schema")?,
        dependent_name: row.try_get("dependent_name")?,
        dependent_identity: row.try_get("dependent_identity")?,
        referenced_type: row.try_get("referenced_type")?,
        referenced_schema: row.try_get("referenced_schema")?,
        referenced_name: row.try_get("referenced_name")?,
        referenced_identity: row.try_get("referenced_identity")?,
    })
}

pub fn decode_sequence_part(
    row: &Row,
    state: SequenceStateObservation,
) -> Result<TargetSequencePart, tokio_postgres::Error> {
    Ok(TargetSequencePart {
        sequence_oid: row.try_get("sequence_oid")?,
        sequence_type_oid: row.try_get("sequence_type_oid")?,
        sequence_schema: row.try_get("sequence_schema")?,
        sequence_name: row.try_get("sequence_name")?,
        type_schema: row.try_get("type_schema")?,
        type_name: row.try_get("type_name")?,
        sequence_owner_role: row.try_get("sequence_owner_role")?,
        bound_column_name: row.try_get("bound_column_name")?,
        bound_identity_mode: row.try_get("bound_identity_mode")?,
        bound_column_ordinal: row.try_get("bound_column_ordinal")?,
        bound_default_expression: row.try_get("bound_default_expression")?,
        owner_table_schema: row.try_get("owner_table_schema")?,
        owner_table_name: row.try_get("owner_table_name")?,
        owner_column_name: row.try_get("owner_column_name")?,
        ownership_dependency: row.try_get("ownership_dependency")?,
        extension_name: row.try_get("extension_name")?,
        default_reference_schema: row.try_get("default_reference_schema")?,
        default_reference_table: row.try_get("default_reference_table")?,
        default_reference_column: row.try_get("default_reference_column")?,
        default_reference_expression: row.try_get("default_reference_expression")?,
        owner_column_ordinal: row.try_get("owner_column_ordinal")?,
        default_reference_ordinal: row.try_get("default_reference_ordinal")?,
        start_value: row.try_get("start_value")?,
        increment_by: row.try_get("increment_by")?,
        min_value: row.try_get("min_value")?,
        max_value: row.try_get("max_value")?,
        cache_size: row.try_get("cache_size")?,
        cycle: row.try_get("cycle")?,
        can_alter: row.try_get("can_alter")?,
        can_use: row.try_get("can_use")?,
        can_select: row.try_get("can_select")?,
        can_update: row.try_get("can_update")?,
        dependency: decode_dependency(row)?,
        state,
    })
}

pub fn decode_trigger_part(row: &Row) -> Result<TargetTriggerPart, tokio_postgres::Error> {
    Ok(TargetTriggerPart {
        scope: row.try_get("scope")?,
        trigger_name: row.try_get("trigger_name")?,
        enabled: row.try_get("enabled")?,
        function_schema: row.try_get("function_schema")?,
        function_name: row.try_get("function_name")?,
        function_arguments: row.try_get("function_arguments")?,
        function_identity: row.try_get("function_identity")?,
        function_definition: row.try_get("function_definition")?,
        function_language: row.try_get("function_language")?,
        function_volatility: row.try_get("function_volatility")?,
        function_owner_role: row.try_get("function_owner_role")?,
        trigger_oid: row.try_get("trigger_oid")?,
        function_oid: row.try_get("function_oid")?,
        internal: row.try_get("internal")?,
        table_oid: row.try_get("table_oid")?,
        constraint_oid: row.try_get("constraint_oid")?,
        trigger_parent_oid: row.try_get("trigger_parent_oid")?,
        table_schema: row.try_get("table_schema")?,
        table_name: row.try_get("table_name")?,
        constraint_name: row.try_get("constraint_name")?,
        definition: row.try_get("definition")?,
        event: row.try_get("event")?,
        dependency_subject: row.try_get("dependency_subject")?,
        event_flags: row.try_get("event_flags")?,
        event_tags: row.try_get("event_tags")?,
        function_config: row.try_get("function_config")?,
        function_security_definer: row.try_get("function_security_definer")?,
        dependency: decode_dependency(row)?,
    })
}

pub async fn read_sequence_state(
    client: &(impl GenericClient + Sync),
    schema: &str,
    name: &str,
    can_select: bool,
) -> Result<SequenceStateObservation, TargetError> {
    if !can_select {
        return Ok(SequenceStateObservation::Unavailable {
            reason: "select_privilege_denied".into(),
        });
    }
    let row = client
        .query_one(
            &format!(
                "SELECT last_value,is_called FROM {}",
                super::qualified(schema, name)
            ),
            &[],
        )
        .await
        .map_err(|error| TargetError::database(error, CopyStage::Idle))?;
    Ok(SequenceStateObservation::Known {
        last_value: row
            .try_get("last_value")
            .map_err(|error| TargetError::database(error, CopyStage::Idle))?,
        is_called: row
            .try_get("is_called")
            .map_err(|error| TargetError::database(error, CopyStage::Idle))?,
    })
}

pub async fn inspect_triggers(
    client: &(impl GenericClient + Sync),
    schema: &str,
    table: Option<&str>,
) -> Result<Vec<TargetTriggerPart>, TargetError> {
    let failure = |error| TargetError::database(error, CopyStage::Idle);
    client
        .query(include_str!("observed_triggers.sql"), &[&schema, &table])
        .await
        .map_err(failure)?
        .iter()
        .map(decode_trigger_part)
        .collect::<Result<Vec<_>, _>>()
        .map_err(failure)
}

/// Query failure/decoding is distinct from a positively observed empty inventory.
/// Call inside the caller-owned read-only catalog transaction.
pub async fn inspect(
    client: &(impl GenericClient + Sync),
    schema: &str,
    table: &str,
) -> Result<TargetTableObserved, TargetError> {
    let failure = |error| TargetError::database(error, CopyStage::Idle);
    let row = client
        .query_one(include_str!("observed_table.sql"), &[&schema, &table])
        .await
        .map_err(failure)?;
    let mut observed = header(&row).map_err(failure)?;
    observed.columns = client
        .query(include_str!("observed_columns.sql"), &[&schema, &table])
        .await
        .map_err(failure)?
        .iter()
        .map(decode_column)
        .collect::<Result<Vec<_>, _>>()
        .map_err(failure)?;
    observed.index_parts = client
        .query(include_str!("observed_indexes.sql"), &[&schema, &table])
        .await
        .map_err(failure)?
        .iter()
        .map(decode_index_part)
        .collect::<Result<Vec<_>, _>>()
        .map_err(failure)?;
    observed.constraint_parts = client
        .query(include_str!("observed_constraints.sql"), &[&schema, &table])
        .await
        .map_err(failure)?
        .iter()
        .map(decode_constraint_part)
        .collect::<Result<Vec<_>, _>>()
        .map_err(failure)?;
    let rows = client
        .query(include_str!("observed_sequences.sql"), &[&schema, &table])
        .await
        .map_err(failure)?;
    let mut states = BTreeMap::new();
    for row in rows {
        let oid: u32 = row.try_get("sequence_oid").map_err(failure)?;
        if let std::collections::btree_map::Entry::Vacant(entry) = states.entry(oid) {
            let sequence_schema: String = row.try_get("sequence_schema").map_err(failure)?;
            let sequence_name: String = row.try_get("sequence_name").map_err(failure)?;
            let can_select: bool = row.try_get("can_select").map_err(failure)?;
            entry.insert(
                read_sequence_state(client, &sequence_schema, &sequence_name, can_select).await?,
            );
        }
        observed
            .sequence_parts
            .push(decode_sequence_part(&row, states[&oid].clone()).map_err(failure)?);
    }
    observed.trigger_parts = inspect_triggers(client, schema, Some(table)).await?;
    Ok(observed)
}
