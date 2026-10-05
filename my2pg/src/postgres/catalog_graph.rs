//! Fallible type/dependency observations. Inspection roots authorize no mutation.
use super::{CopyStage, TargetError, observed};
use crate::model::{
    TargetDependencyEdge, TargetDependencyGraph, TargetDependencyGraphPart, TargetTypePart,
};
use tokio_postgres::{GenericClient, Row};

fn database(error: tokio_postgres::Error) -> TargetError {
    TargetError::database(error, CopyStage::Idle)
}

pub fn decode_type_part(row: &Row) -> Result<TargetTypePart, TargetError> {
    let part = (|| -> Result<TargetTypePart, tokio_postgres::Error> {
        Ok(TargetTypePart {
            namespace_oid: row.try_get("namespace_oid")?,
            namespace_name: row.try_get("namespace_name")?,
            namespace_can_use: row.try_get("namespace_can_use")?,
            namespace_can_create: row.try_get("namespace_can_create")?,
            type_oid: row.try_get("type_oid")?,
            type_owner_role_oid: row.try_get("type_owner_role_oid")?,
            type_relation_oid: row.try_get("type_relation_oid")?,
            type_element_oid: row.try_get("type_element_oid")?,
            type_array_oid: row.try_get("type_array_oid")?,
            type_base_oid: row.try_get("type_base_oid")?,
            type_collation_oid: row.try_get("type_collation_oid")?,
            extension_oid: row.try_get("extension_oid")?,
            type_schema: row.try_get("type_schema")?,
            type_name: row.try_get("type_name")?,
            type_owner_role: row.try_get("type_owner_role")?,
            type_kind: row.try_get("type_kind")?,
            type_category: row.try_get("type_category")?,
            type_alignment: row.try_get("type_alignment")?,
            type_storage: row.try_get("type_storage")?,
            type_default_expression: row.try_get("type_default_expression")?,
            type_comment: row.try_get("type_comment")?,
            extension_name: row.try_get("extension_name")?,
            type_defined: row.try_get("type_defined")?,
            type_by_value: row.try_get("type_by_value")?,
            domain_not_null: row.try_get("domain_not_null")?,
            can_use: row.try_get("can_use")?,
            can_alter: row.try_get("can_alter")?,
            type_length: row.try_get("type_length")?,
            type_modifier: row.try_get("type_modifier")?,
            domain_array_dimensions: row.try_get("domain_array_dimensions")?,
            enum_label_oid: row.try_get("enum_label_oid")?,
            enum_label: row.try_get("enum_label")?,
            enum_sort_order: row.try_get("enum_sort_order")?,
            domain_constraint_oid: row.try_get("domain_constraint_oid")?,
            domain_constraint_name: row.try_get("domain_constraint_name")?,
            domain_constraint_kind: row.try_get("domain_constraint_kind")?,
            domain_constraint_definition: row.try_get("domain_constraint_definition")?,
            domain_constraint_expression: row.try_get("domain_constraint_expression")?,
            domain_constraint_validated: row.try_get("domain_constraint_validated")?,
            domain_constraint_deferrable: row.try_get("domain_constraint_deferrable")?,
            domain_constraint_initially_deferred: row
                .try_get("domain_constraint_initially_deferred")?,
        })
    })()
    .map_err(database)?;
    validate_type_part(&part)?;
    Ok(part)
}

fn validate_type_part(part: &TargetTypePart) -> Result<(), TargetError> {
    let invalid = || TargetError::configuration("invalid observed type inventory");
    if part.namespace_oid == 0 || part.namespace_name.is_empty() {
        return Err(invalid());
    }
    if let Some(oid) = part.type_oid {
        if oid == 0
            || part.type_owner_role_oid.is_none_or(|oid| oid == 0)
            || part.type_schema.as_deref() != Some(part.namespace_name.as_str())
            || part.type_name.as_deref().is_none_or(str::is_empty)
            || part.type_owner_role.is_none()
            || part.type_kind.is_none()
            || part.type_category.is_none()
            || part.type_alignment.is_none()
            || part.type_storage.is_none()
            || part.type_defined.is_none()
            || part.type_by_value.is_none()
            || part.domain_not_null.is_none()
            || part.can_use.is_none()
            || part.can_alter.is_none()
            || part.type_length.is_none()
            || part.type_modifier.is_none()
            || part.domain_array_dimensions.is_none()
            || part.type_relation_oid.is_none()
            || part.type_element_oid.is_none()
            || part.type_array_oid.is_none()
            || part.type_base_oid.is_none()
            || part.type_collation_oid.is_none()
        {
            return Err(invalid());
        }
    } else if part.type_name.is_some()
        || part.type_schema.is_some()
        || part.type_kind.is_some()
        || part.type_owner_role_oid.is_some()
        || part.type_owner_role.is_some()
        || part.type_relation_oid.is_some()
        || part.type_element_oid.is_some()
        || part.type_array_oid.is_some()
        || part.type_base_oid.is_some()
        || part.type_collation_oid.is_some()
        || part.type_category.is_some()
        || part.type_alignment.is_some()
        || part.type_storage.is_some()
        || part.type_default_expression.is_some()
        || part.type_comment.is_some()
        || part.type_defined.is_some()
        || part.type_by_value.is_some()
        || part.domain_not_null.is_some()
        || part.can_use.is_some()
        || part.can_alter.is_some()
        || part.type_length.is_some()
        || part.type_modifier.is_some()
        || part.domain_array_dimensions.is_some()
        || part.extension_oid.is_some()
        || part.extension_name.is_some()
        || part.enum_label_oid.is_some()
        || part.domain_constraint_oid.is_some()
    {
        return Err(invalid());
    }
    match (part.enum_label_oid, &part.enum_label, part.enum_sort_order) {
        (Some(oid), Some(_), Some(order))
            if oid > 0 && order.is_finite() && part.type_kind.as_deref() == Some("e") => {}
        (None, None, None) => {}
        _ => return Err(invalid()),
    }
    if part.extension_oid.is_some() != part.extension_name.is_some()
        || part.extension_oid == Some(0)
    {
        return Err(invalid());
    }
    if let Some(oid) = part.domain_constraint_oid {
        if oid == 0
            || part.type_kind.as_deref() != Some("d")
            || part.domain_constraint_name.is_none()
            || part.domain_constraint_kind.is_none()
            || part.domain_constraint_definition.is_none()
            || part.domain_constraint_validated.is_none()
            || part.domain_constraint_deferrable.is_none()
            || part.domain_constraint_initially_deferred.is_none()
        {
            return Err(invalid());
        }
    } else if part.domain_constraint_name.is_some()
        || part.domain_constraint_kind.is_some()
        || part.domain_constraint_definition.is_some()
        || part.domain_constraint_expression.is_some()
        || part.domain_constraint_validated.is_some()
        || part.domain_constraint_deferrable.is_some()
        || part.domain_constraint_initially_deferred.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

pub async fn inspect_types(
    client: &(impl GenericClient + Sync),
    schema: &str,
) -> Result<Vec<TargetTypePart>, TargetError> {
    client
        .query(include_str!("observed_types.sql"), &[&schema])
        .await
        .map_err(database)?
        .iter()
        .map(decode_type_part)
        .collect()
}

fn graph_header(row: &Row) -> Result<(u32, u64, u64, u64), TargetError> {
    let invalid = || TargetError::configuration("invalid observed dependency roots");
    let database_oid: u32 = row.try_get("database_oid").map_err(database)?;
    let valid: bool = row.try_get("roots_valid").map_err(database)?;
    if database_oid == 0 || !valid {
        return Err(invalid());
    }
    let count = |name| -> Result<u64, TargetError> {
        let count: i64 = row.try_get(name).map_err(database)?;
        u64::try_from(count).map_err(|_| invalid())
    };
    Ok((
        database_oid,
        count("relation_root_count")?,
        count("type_root_count")?,
        count("event_trigger_count")?,
    ))
}

fn graph_part(row: &Row) -> Result<Option<TargetDependencyGraphPart>, TargetError> {
    let invalid = || TargetError::configuration("invalid observed dependency identity");
    let node_class_oid: Option<u32> = row.try_get("node_class_oid").map_err(database)?;
    let node_oid: Option<u32> = row.try_get("node_oid").map_err(database)?;
    let node_sub_id: Option<i32> = row.try_get("node_sub_id").map_err(database)?;
    let dependent_class_oid: Option<u32> = row.try_get("dependent_class_oid").map_err(database)?;
    let referenced_class_oid: Option<u32> =
        row.try_get("referenced_class_oid").map_err(database)?;
    let dependency = observed::decode_dependency(row).map_err(database)?;
    let edge = match (dependent_class_oid, referenced_class_oid) {
        (None, None) if dependency == empty_dependency() => None,
        (Some(dependent_class_oid), Some(referenced_class_oid))
            if dependent_class_oid > 0
                && referenced_class_oid > 0
                && dependency.dependent_oid.is_some_and(|oid| oid > 0)
                && dependency.referenced_oid.is_some_and(|oid| oid > 0)
                && dependency.dependent_sub_id.is_some()
                && dependency.referenced_sub_id.is_some()
                && dependency.dependent_catalog.is_some()
                && dependency.referenced_catalog.is_some()
                && dependency.dependency_catalog.is_some()
                && dependency.dependency_direction.is_some()
                && dependency.dependency_kind.is_some() =>
        {
            Some(TargetDependencyEdge {
                dependent_class_oid,
                referenced_class_oid,
                dependency,
            })
        }
        _ => return Err(invalid()),
    };
    let (node_class_oid, node_oid, node_sub_id) = match (node_class_oid, node_oid, node_sub_id) {
        (None, None, None) if edge.is_none() => {
            validate_empty_node(row)?;
            return Ok(None);
        }
        (Some(class), Some(oid), Some(sub)) if class > 0 && oid > 0 => (class, oid, sub),
        _ => return Err(invalid()),
    };
    let part = (|| -> Result<_, tokio_postgres::Error> {
        Ok(TargetDependencyGraphPart {
            node_class_oid,
            node_oid,
            node_sub_id,
            node_catalog: row.try_get("node_catalog")?,
            node_type: row.try_get("node_type")?,
            node_schema: row.try_get("node_schema")?,
            node_name: row.try_get("node_name")?,
            node_identity: row.try_get("node_identity")?,
            relation_root: row.try_get("relation_root")?,
            type_root: row.try_get("type_root")?,
            global_event_root: row.try_get("global_event_root")?,
            node_owner_relation_oid: row.try_get("node_owner_relation_oid")?,
            node_owner_role_oid: row.try_get("node_owner_role_oid")?,
            node_extension_oid: row.try_get("node_extension_oid")?,
            node_owner_column_ordinal: row.try_get("node_owner_column_ordinal")?,
            node_owner_relation_schema: row.try_get("node_owner_relation_schema")?,
            node_owner_relation_name: row.try_get("node_owner_relation_name")?,
            node_owner_role: row.try_get("node_owner_role")?,
            node_extension_name: row.try_get("node_extension_name")?,
            node_relation_kind: row.try_get("node_relation_kind")?,
            node_type_kind: row.try_get("node_type_kind")?,
            node_constraint_kind: row.try_get("node_constraint_kind")?,
            node_function_kind: row.try_get("node_function_kind")?,
            node_function_definition: row.try_get("node_function_definition")?,
            node_event_enabled: row.try_get("node_event_enabled")?,
            node_event: row.try_get("node_event")?,
            node_function_body_parsed: row.try_get("node_function_body_parsed")?,
            edge,
        })
    })()
    .map_err(database)?;
    validate_graph_part(&part)?;
    Ok(Some(part))
}

fn validate_graph_part(part: &TargetDependencyGraphPart) -> Result<(), TargetError> {
    let invalid = || TargetError::configuration("incoherent observed dependency facts");
    let column_subid = |catalog: &str, subid: i32| subid == 0 || catalog == "pg_catalog.pg_class";
    if part.node_catalog.is_empty()
        || part.node_type.is_empty()
        || part.node_identity.is_empty()
        || !column_subid(&part.node_catalog, part.node_sub_id)
        || part.node_owner_relation_oid.is_some() != part.node_owner_relation_schema.is_some()
        || part.node_owner_relation_oid.is_some() != part.node_owner_relation_name.is_some()
        || part.node_owner_relation_oid == Some(0)
        || part.node_owner_role_oid.is_some() != part.node_owner_role.is_some()
        || part.node_owner_role_oid == Some(0)
        || part.node_extension_oid.is_some() != part.node_extension_name.is_some()
        || part.node_extension_oid == Some(0)
        || part
            .node_owner_column_ordinal
            .is_some_and(|ordinal| ordinal == 0)
        || (part.node_owner_column_ordinal.is_some() && part.node_owner_relation_oid.is_none())
    {
        return Err(invalid());
    }
    if let Some(edge) = &part.edge {
        let facts = &edge.dependency;
        let dependent = facts.dependent_sub_id.ok_or_else(invalid)?;
        let referenced = facts.referenced_sub_id.ok_or_else(invalid)?;
        if !column_subid(
            facts.dependent_catalog.as_deref().ok_or_else(invalid)?,
            dependent,
        ) || !column_subid(
            facts.referenced_catalog.as_deref().ok_or_else(invalid)?,
            referenced,
        ) || !matches!(
            facts.dependency_catalog.as_deref(),
            Some("pg_catalog.pg_depend" | "pg_catalog.pg_shdepend")
        ) {
            return Err(invalid());
        }
        let covers = |class, oid, sub| {
            class == part.node_class_oid
                && oid == Some(part.node_oid)
                && (part.node_sub_id == 0 || sub == part.node_sub_id)
        };
        let incident = match facts.dependency_direction.as_deref() {
            Some("incoming")
                if facts.dependency_catalog.as_deref() == Some("pg_catalog.pg_depend") =>
            {
                covers(edge.referenced_class_oid, facts.referenced_oid, referenced)
                    && facts.referenced_catalog.as_deref() == Some(part.node_catalog.as_str())
            }
            Some("outgoing") => {
                covers(edge.dependent_class_oid, facts.dependent_oid, dependent)
                    && facts.dependent_catalog.as_deref() == Some(part.node_catalog.as_str())
            }
            _ => false,
        };
        if !incident {
            return Err(invalid());
        }
    }
    Ok(())
}

fn validate_empty_node(row: &Row) -> Result<(), TargetError> {
    let invalid = || TargetError::configuration("contradictory empty dependency inventory");
    for name in ["relation_root", "type_root", "global_event_root"] {
        if row.try_get::<_, bool>(name).map_err(database)? {
            return Err(invalid());
        }
    }
    for name in [
        "node_catalog",
        "node_type",
        "node_schema",
        "node_name",
        "node_identity",
        "node_owner_relation_schema",
        "node_owner_relation_name",
        "node_owner_role",
        "node_extension_name",
        "node_relation_kind",
        "node_type_kind",
        "node_constraint_kind",
        "node_function_kind",
        "node_function_definition",
        "node_event_enabled",
        "node_event",
    ] {
        if row
            .try_get::<_, Option<String>>(name)
            .map_err(database)?
            .is_some()
        {
            return Err(invalid());
        }
    }
    for name in [
        "node_owner_relation_oid",
        "node_owner_role_oid",
        "node_extension_oid",
    ] {
        if row
            .try_get::<_, Option<u32>>(name)
            .map_err(database)?
            .is_some()
        {
            return Err(invalid());
        }
    }
    if row
        .try_get::<_, Option<i16>>("node_owner_column_ordinal")
        .map_err(database)?
        .is_some()
        || row
            .try_get::<_, Option<bool>>("node_function_body_parsed")
            .map_err(database)?
            .is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

fn empty_dependency() -> crate::model::TargetObjectDependency {
    crate::model::TargetObjectDependency {
        dependency_catalog: None,
        dependency_direction: None,
        dependency_kind: None,
        dependent_catalog: None,
        dependent_oid: None,
        dependent_sub_id: None,
        referenced_catalog: None,
        referenced_oid: None,
        referenced_sub_id: None,
        dependent_type: None,
        dependent_schema: None,
        dependent_name: None,
        dependent_identity: None,
        referenced_type: None,
        referenced_schema: None,
        referenced_name: None,
        referenced_identity: None,
    }
}

pub fn decode_dependency_graph(rows: &[Row]) -> Result<TargetDependencyGraph, TargetError> {
    let invalid = || TargetError::configuration("incomplete observed dependency inventory");
    let header = graph_header(rows.first().ok_or_else(invalid)?)?;
    let mut graph = TargetDependencyGraph {
        database_oid: header.0,
        relation_root_count: header.1,
        type_root_count: header.2,
        event_trigger_count: header.3,
        parts: Vec::new(),
    };
    for row in rows {
        if graph_header(row)? != header {
            return Err(invalid());
        }
        match graph_part(row)? {
            Some(part) => graph.parts.push(part),
            None if rows.len() == 1 && header.1 == 0 && header.2 == 0 && header.3 == 0 => {}
            None => return Err(invalid()),
        }
    }
    let roots = |flag: fn(&TargetDependencyGraphPart) -> bool| {
        graph
            .parts
            .iter()
            .filter(|part| flag(part))
            .map(|part| (part.node_class_oid, part.node_oid, part.node_sub_id))
            .collect::<std::collections::BTreeSet<_>>()
    };
    let relations = roots(|part| part.relation_root);
    let types = roots(|part| part.type_root);
    let events = roots(|part| part.global_event_root);
    if relations.len() as u64 != header.1
        || types.len() as u64 != header.2
        || events.len() as u64 != header.3
        || graph.parts.iter().any(|part| {
            (part.relation_root
                && (part.node_sub_id != 0 || part.node_catalog != "pg_catalog.pg_class"))
                || (part.type_root
                    && (part.node_sub_id != 0 || part.node_catalog != "pg_catalog.pg_type"))
                || (part.global_event_root
                    && (part.node_sub_id != 0
                        || part.node_catalog != "pg_catalog.pg_event_trigger"))
        })
    {
        return Err(invalid());
    }
    Ok(graph)
}

pub async fn inspect_dependency_graph(
    client: &(impl GenericClient + Sync),
    relation_roots: &[u32],
    type_roots: &[u32],
) -> Result<TargetDependencyGraph, TargetError> {
    let rows = client
        .query(
            include_str!("observed_dependency_graph.sql"),
            &[&relation_roots, &type_roots],
        )
        .await
        .map_err(database)?;
    let graph = decode_dependency_graph(&rows)?;
    let distinct_count = |roots: &[u32]| {
        roots
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    if graph.relation_root_count != distinct_count(relation_roots) as u64
        || graph.type_root_count != distinct_count(type_roots) as u64
    {
        return Err(TargetError::configuration("dependency root count mismatch"));
    }
    let actual_roots = |flag: fn(&TargetDependencyGraphPart) -> bool| {
        graph
            .parts
            .iter()
            .filter(|part| flag(part))
            .map(|part| part.node_oid)
            .collect::<std::collections::BTreeSet<_>>()
    };
    if actual_roots(|part| part.relation_root) != relation_roots.iter().copied().collect()
        || actual_roots(|part| part.type_root) != type_roots.iter().copied().collect()
    {
        return Err(TargetError::configuration(
            "dependency root identity mismatch",
        ));
    }
    Ok(graph)
}
