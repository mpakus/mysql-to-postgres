//! Positive existing-object proof. Broad inspection reachability is never mutation selection.
use super::*;
use crate::verify::defaults::{
    self, DefaultCatalogContext, DefaultComparison, DeparseSettings, EnumIdentity,
};

fn error(plan: &mut MigrationPlan, code: &str, object: &str, message: &str) {
    plan.diagnostics
        .push(diagnostic(code, object, message, Severity::Error));
}

pub(super) struct NamespaceFacts<'a> {
    pub exists: bool,
    pub can_use: bool,
    pub can_create: bool,
    pub names: &'a BTreeMap<String, String>,
    pub types: &'a BTreeSet<String>,
}
pub(super) fn namespace<'a>(
    target: &'a TargetCatalog,
    schema: &str,
    primary: &str,
) -> Option<NamespaceFacts<'a>> {
    if let Some(namespaces) = &target.namespaces {
        let mut matches = namespaces.iter().filter(|n| n.schema == schema);
        let n = matches.next()?;
        if matches.next().is_some()
            || (n.schema_exists && n.oid.is_none_or(|oid| oid == 0))
            || (!n.schema_exists
                && (n.oid.is_some()
                    || n.can_use
                    || n.can_create_objects
                    || !n.occupied_names.is_empty()
                    || !n.occupied_types.is_empty()))
        {
            return None;
        }
        Some(NamespaceFacts {
            exists: n.schema_exists,
            can_use: n.can_use,
            can_create: n.can_create_objects,
            names: &n.occupied_names,
            types: &n.occupied_types,
        })
    } else if schema == primary {
        // Compatibility for the original primary-only serialized fixture contract.
        // Native inspection publishes namespace-qualified observations instead.
        Some(NamespaceFacts {
            exists: target.schema_exists,
            can_use: target.can_use_schema,
            can_create: target.can_create_objects,
            names: &target.occupied_names,
            types: &target.occupied_types,
        })
    } else {
        None
    }
}
pub(super) fn namespace_policy(
    config: &MigrationConfig,
    target: &TargetCatalog,
    table: &TablePlan,
    creates: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut error = |code: &str, object: &str, message: &str| {
        diagnostics.push(diagnostic(code, object, message, Severity::Error))
    };
    let schema = &table.target_schema;
    let Some(facts) = namespace(target, schema, &config.target.schema) else {
        error(
            "TARGET_SCHEMA_UNINSPECTED",
            schema,
            "mapped destination requires unique positive namespace facts",
        );
        return;
    };
    if !facts.exists && creates && !target.can_create_schema {
        error(
            "TARGET_CREATE_SCHEMA_DENIED",
            schema,
            "target role lacks database CREATE for absent mapped namespace",
        );
    }
    if facts.exists && (!facts.can_use || (creates && !facts.can_create)) {
        error(
            if creates {
                "TARGET_SCHEMA_PRIVILEGE"
            } else {
                "TARGET_SCHEMA_USAGE_DENIED"
            },
            schema,
            "mapped namespace privileges contradict the requested operation",
        );
    }
    if !facts.exists && !creates {
        error(
            "TARGET_SCHEMA_USAGE_DENIED",
            schema,
            "data-only/retained target namespace is absent",
        );
    }
}

pub(super) fn enum_type(
    target: &TargetCatalog,
    schema: &str,
    name: &str,
    labels: &[String],
) -> Option<u32> {
    let parts: Vec<_> = target
        .type_parts
        .as_ref()?
        .iter()
        .filter(|p| {
            p.type_schema.as_deref() == Some(schema) && p.type_name.as_deref() == Some(name)
        })
        .collect();
    let first = *parts.first()?;
    let oid = first.type_oid?;
    if oid == 0
        || !target.dependency_graph.as_ref()?.parts.iter().any(|p| {
            p.node_catalog == "pg_catalog.pg_type"
                && p.node_oid == oid
                && p.node_sub_id == 0
                && p.type_root
                && p.node_type_kind.as_deref() == Some("e")
                && p.node_extension_oid.is_none()
        })
    {
        return None;
    }
    if parts.iter().any(|p| {
        p.type_oid != Some(oid)
            || p.type_kind.as_deref() != Some("e")
            || p.type_defined != Some(true)
            || p.can_use != Some(true)
            || !p.namespace_can_use
            || p.extension_oid.is_some()
            || p.domain_constraint_oid.is_some()
    }) {
        return None;
    }
    let mut ordered = Vec::new();
    for p in parts {
        let order = p.enum_sort_order?;
        if !order.is_finite() || p.enum_label_oid == Some(0) || p.enum_label_oid.is_none() {
            return None;
        }
        ordered.push((order, p.enum_label.as_ref()?));
    }
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    if ordered.windows(2).any(|p| p[0].0 >= p[1].0) || ordered.iter().map(|p| p.1).ne(labels.iter())
    {
        return None;
    }
    Some(oid)
}

pub(super) fn reuses_enum(target: &TargetCatalog, column: &ColumnPlan) -> bool {
    target.type_parts.as_ref().is_some_and(|parts| {
        parts.iter().any(|p| {
            p.type_schema
                .as_deref()
                .zip(p.type_name.as_deref())
                .is_some_and(|(schema, name)| {
                    qualified_name(schema, name) == column.target_type
                        && enum_type(target, schema, name, &column.enum_labels).is_some()
                })
        })
    })
}

fn generation_matches(expected: Option<&str>, actual_kind: &str, actual: Option<&str>) -> bool {
    match (expected, actual_kind, actual) {
        (Some(expected), "s", Some(actual)) => expected == actual,
        (None, "", None) => true,
        _ => false,
    }
}

pub(super) fn apply(config: &MigrationConfig, target: &TargetCatalog, plan: &mut MigrationPlan) {
    // Move tables out while collecting diagnostics; no mutable/immutable plan aliasing.
    let mut tables = std::mem::take(&mut plan.tables);
    for table in &mut tables {
        let Some(existing) = target
            .tables
            .iter()
            .find(|t| t.schema == table.target_schema && t.name == table.target_name)
        else {
            continue;
        };
        let Some(observed) = &existing.observed else {
            error(
                plan,
                "TARGET_OBSERVATION_REQUIRED",
                &table.source_name,
                "existing target requires positive typed catalog observations",
            );
            continue;
        };
        if observed.oid == 0
            || observed.relation_kind != "r"
            || observed.persistence != "p"
            || observed.row_security_enabled
            || observed.row_security_forced
        {
            error(
                plan,
                "TARGET_TABLE_UNSUPPORTED",
                &table.source_name,
                "existing target kind/persistence/row-security is outside the ordinary-table policy",
            );
        }
        if config.target.on_existing == ExistingPolicy::Recreate {
            recreate_inventory(observed, plan);
            continue;
        }
        if config.migration.reset_sequences
            && table.columns.iter().any(|column| column.identity)
            && existing.can_lock != Some(true)
        {
            error(
                plan,
                "TARGET_SEQUENCE_LOCK_PRIVILEGE",
                &table.source_name,
                "requested sequence adjustment requires positively observed selected-table lock privileges before copying",
            );
        }
        if observed.columns.len() != table.columns.len() {
            error(
                plan,
                "TARGET_EXTRA_COLUMNS",
                &table.source_name,
                "observed target column set differs from the selected source",
            );
        }
        for (position, column) in table.columns.iter().enumerate() {
            let Some(actual) = observed
                .columns
                .get(position)
                .filter(|a| a.name == column.target_name)
            else {
                error(
                    plan,
                    "TARGET_COLUMN_INCOMPATIBLE",
                    &column.source_name,
                    "observed ordered destination column is missing or reordered",
                );
                continue;
            };
            let enum_oid = (column.kind == ValueKind::Enum)
                .then(|| {
                    enum_type(
                        target,
                        &actual.type_schema,
                        &actual.type_name,
                        &column.enum_labels,
                    )
                })
                .flatten();
            let type_matches = if column.kind == ValueKind::Enum {
                enum_oid == Some(actual.type_oid)
            } else {
                actual.type_kind == "b"
                    && actual.type_base_oid == 0
                    && compatible_type(&actual.data_type, &column.target_type)
            };
            let generated_matches = generation_matches(
                column.generated_expression.as_deref(),
                &actual.generated_kind,
                actual.generation_expression.as_deref(),
            );
            if !type_matches
                || actual.nullable != column.nullable
                || !generated_matches
                || (column.identity && actual.identity_mode != "d")
                || (!column.identity && !actual.identity_mode.is_empty())
            {
                error(
                    plan,
                    "TARGET_COLUMN_INCOMPATIBLE",
                    &column.source_name,
                    "observed type/nullability/generation/identity differs from the resolved column",
                );
            }
            if column.comment.is_some() && column.comment != actual.comment {
                error(
                    plan,
                    "TARGET_COMMENT_INCOMPATIBLE",
                    &column.source_name,
                    "requested column comment differs from the existing target",
                );
            }
            if !column.identity {
                let matches = match (&column.default_sql, &actual.default_expression) {
                    (None, None) => true,
                    (Some(_), Some(expression)) => {
                        let settings = target.deparse.as_ref();
                        let style = settings
                            .and_then(|s| defaults::IntervalStyle::parse(&s.interval_style));
                        if column.kind == ValueKind::Interval && style.is_none() {
                            error(
                                plan,
                                "TARGET_DEFAULT_CONTEXT_REQUIRED",
                                &column.source_name,
                                "interval default equivalence requires positively captured IntervalStyle",
                            );
                        }
                        let context = DefaultCatalogContext {
                            target_type_proven: type_matches,
                            interval_style: style.unwrap_or_default(),
                            enum_identity: enum_oid.map(|oid| EnumIdentity {
                                expected_oid: oid,
                                column_oid: actual.type_oid,
                                cast_oid: defaults::enum_cast(expression)
                                    .filter(|cast| compatible_type(cast, &column.target_type))
                                    .map(|_| oid),
                            }),
                        };
                        defaults::compare_with_catalog(
                            column,
                            expression,
                            &DeparseSettings {
                                exact_float_output: settings
                                    .is_some_and(|s| s.extra_float_digits >= 1),
                            },
                            &context,
                        ) == DefaultComparison::Equal
                    }
                    _ => false,
                };
                if !matches {
                    error(
                        plan,
                        "TARGET_DEFAULT_INCOMPATIBLE",
                        &column.source_name,
                        "typed existing default is different or cannot be proved equivalent",
                    );
                }
            }
        }
        if table.structure.comment.is_some() && table.structure.comment != observed.comment {
            error(
                plan,
                "TARGET_COMMENT_INCOMPATIBLE",
                &table.source_name,
                "requested table comment differs from the existing target",
            );
        }
        bind_indexes(observed, table, plan);
        bind_constraints(observed, table, plan);
        identity(config, observed, table, plan);
        for trigger in &observed.trigger_parts {
            if !matches!(trigger.enabled.as_str(), "O" | "D" | "R" | "A")
                || trigger.internal.is_none()
            {
                error(
                    plan,
                    "TARGET_TRIGGER_UNKNOWN",
                    &trigger.trigger_name,
                    "trigger enable/internal state is unrecognized",
                );
            } else if trigger.internal == Some(false) {
                if config.target.on_existing == ExistingPolicy::Truncate
                    && trigger.enabled != "D"
                    && trigger.event_flags.is_some_and(|f| f & 32 != 0)
                {
                    error(
                        plan,
                        "TARGET_TRUNCATE_TRIGGER",
                        &trigger.trigger_name,
                        "enabled ON TRUNCATE trigger needs a reviewed scoped side-effect policy",
                    );
                } else {
                    plan.diagnostics.push(diagnostic("TARGET_TRIGGER_PRESERVED",&trigger.trigger_name,"existing user trigger remains enabled/disabled as observed; COPY effects are subject to normal error/accounting checks",Severity::Warning));
                }
            }
        }
    }
    plan.tables = tables;
    destructive(config, target, plan);
}

#[cfg(test)]
mod generated_column_tests {
    use super::generation_matches;

    #[test]
    fn existing_generated_expression_requires_exact_stored_catalog_match() {
        assert!(generation_matches(
            Some("lower(source_text)"),
            "s",
            Some("lower(source_text)")
        ));
        assert!(!generation_matches(
            Some("lower(source_text)"),
            "s",
            Some("upper(source_text)")
        ));
        assert!(!generation_matches(
            Some("lower(source_text)"),
            "v",
            Some("lower(source_text)")
        ));
        assert!(!generation_matches(Some("lower(source_text)"), "s", None));
        assert!(generation_matches(None, "", None));
        assert!(!generation_matches(None, "s", Some("lower(source_text)")));
    }
}

fn indexes(observed: &TargetTableObserved) -> BTreeMap<u32, Vec<&TargetIndexPart>> {
    let mut groups: BTreeMap<u32, Vec<_>> = BTreeMap::new();
    for p in &observed.index_parts {
        groups.entry(p.index_oid).or_default().push(p);
    }
    for parts in groups.values_mut() {
        parts.sort_by_key(|p| p.ordinal);
    }
    groups
}
fn basic_index(
    parts: &[&TargetIndexPart],
    observed: &TargetTableObserved,
    selected: (&str, &str),
) -> bool {
    let deferred = linked_deferred_key(parts, observed, selected);
    !parts.is_empty()
        && parts.iter().enumerate().all(|(i, p)| {
            p.index_oid > 0
                && p.table_oid == observed.oid
                && p.ordinal == i as i64 + 1
                && p.method == "btree"
                && p.valid
                && p.ready
                && p.live
                && (p.immediate || deferred)
                && !p.is_exclusion
                && !p.nulls_not_distinct
                && p.predicate.is_none()
                && p.expression.is_none()
                && !p.included
                && p.column_ordinal > 0
                && p.column_name.is_some()
                && p.descending.is_some()
                && p.nulls_first == p.descending
                && p.default_operator_class == Some(true)
                && (deferred || p.constraint_deferrable != Some(true))
                && (deferred || p.constraint_initially_deferred != Some(true))
                && p.collation_deterministic != Some(false)
                && observed.columns.iter().any(|c| {
                    c.ordinal == p.column_ordinal
                        && Some(&c.name) == p.column_name.as_ref()
                        && c.collation_oid == p.collation_oid
                })
        })
}
fn linked_deferred_key(
    parts: &[&TargetIndexPart],
    observed: &TargetTableObserved,
    selected: (&str, &str),
) -> bool {
    let Some(index) = parts.first() else {
        return false;
    };
    let mut keys: Vec<_> = observed
        .constraint_parts
        .iter()
        .filter(|p| p.supporting_index_oid == Some(index.index_oid))
        .collect();
    keys.sort_by_key(|p| p.ordinal);
    keys.dedup();
    let Some(first) = keys.first() else {
        return false;
    };
    if first.constraint_oid == 0
        || !matches!(first.kind.as_str(), "p" | "u")
        || !first.validated
        || !first.deferrable
        || first.definition.is_empty()
        || keys.len() != parts.len()
    {
        return false;
    }
    keys.iter().zip(parts).enumerate().all(|(i, (key, part))| {
        let mut header = (*key).clone();
        header.ordinal = first.ordinal;
        header.column_name = first.column_name.clone();
        header == **first
            && key.ordinal == Some(i as i64 + 1)
            && key.column_name == part.column_name
            && key.column_expression.is_none()
            && key.check_expression.is_none()
            && key.supporting_index_schema.as_deref() == Some(index.index_schema.as_str())
            && key.supporting_index_name.as_deref() == Some(index.index_name.as_str())
            && part.index_oid == index.index_oid
            && part.index_schema == index.index_schema
            && part.index_name == index.index_name
            && part.table_oid == observed.oid
            && part.table_schema == selected.0
            && part.table_name == selected.1
            && part.index_schema == selected.0
            && part.table_schema == index.table_schema
            && part.table_name == index.table_name
            && part.index_schema == part.table_schema
            && part.ordinal == i as i64 + 1
            && part.column_name.is_some()
            && part.is_unique
            && part.is_primary == (first.kind == "p")
            && !part.immediate
            && part.constraint_name.as_deref() == Some(first.name.as_str())
            && part.constraint_deferrable == Some(true)
            && part.constraint_initially_deferred == Some(first.initially_deferred)
    })
}
fn index_shape(parts: &[&TargetIndexPart], expected: &IndexExpectation) -> bool {
    !parts.is_empty()
        && parts.len() == expected.expressions.len()
        && parts.iter().all(|p| {
            p.immediate
                && p.constraint_deferrable != Some(true)
                && p.constraint_initially_deferred != Some(true)
                && p.is_unique == expected.unique
                && p.is_primary == expected.primary
        })
        && parts
            .iter()
            .filter_map(|p| p.column_name.as_ref())
            .eq(expected.columns.iter())
        && parts
            .iter()
            .map(|part| part.expression.as_deref())
            .eq(expected.expressions.iter().map(Option::as_deref))
        && parts
            .iter()
            .filter_map(|p| p.descending)
            .eq(expected.descending.iter().copied())
}
fn recreatable_constraint_kind(kind: &str) -> bool {
    matches!(kind, "p" | "u" | "f" | "c" | "x" | "n")
}
fn recreate_inventory(observed: &TargetTableObserved, plan: &mut MigrationPlan) {
    for parts in indexes(observed).values() {
        if !matches!(
            parts[0].method.as_str(),
            "btree" | "hash" | "gist" | "gin" | "spgist" | "brin"
        ) {
            error(
                plan,
                "TARGET_DROP_INDEX_UNSUPPORTED",
                &parts[0].index_name,
                "selected table has an unrecognized index access method",
            );
        }
        plan.diagnostics.push(diagnostic(
            "TARGET_RECREATE_REMOVE_INDEX",
            &parts[0].index_name,
            "selected table recreation removes this observed owned index",
            Severity::Warning,
        ));
    }
    let mut seen = BTreeSet::new();
    for constraint in &observed.constraint_parts {
        if !seen.insert(constraint.constraint_oid) {
            continue;
        }
        if !recreatable_constraint_kind(&constraint.kind) {
            error(
                plan,
                "TARGET_DROP_CONSTRAINT_UNSUPPORTED",
                &constraint.name,
                "selected table has an unrecognized constraint kind",
            );
        }
        plan.diagnostics.push(diagnostic(
            "TARGET_RECREATE_REMOVE_CONSTRAINT",
            &constraint.name,
            "selected table recreation removes this observed owned constraint",
            Severity::Warning,
        ));
    }
    let mut seen = BTreeSet::new();
    for trigger in &observed.trigger_parts {
        if !seen.insert(trigger.trigger_oid) {
            continue;
        }
        if !matches!(trigger.enabled.as_str(), "O" | "D" | "R" | "A") || trigger.internal.is_none()
        {
            error(
                plan,
                "TARGET_TRIGGER_UNKNOWN",
                &trigger.trigger_name,
                "selected trigger state is unrecognized",
            );
        }
        if trigger.internal == Some(false) {
            plan.diagnostics.push(diagnostic(
                "TARGET_RECREATE_REMOVE_TRIGGER",
                &trigger.trigger_name,
                "selected table recreation removes this user table trigger; its function remains",
                Severity::Warning,
            ));
        }
    }
}

fn bind_indexes(observed: &TargetTableObserved, table: &mut TablePlan, plan: &mut MigrationPlan) {
    let groups = indexes(observed);
    for parts in groups.values() {
        if !basic_index(parts, observed, (&table.target_schema, &table.target_name)) {
            error(
                plan,
                "TARGET_INDEX_UNSUPPORTED",
                &parts[0].index_name,
                "observed index validity/key/include/predicate/operator/collation/deferral state is unsupported",
            );
        }
    }
    let mut used = BTreeSet::new();
    for expected in &mut table.structure.indexes {
        let found = groups.iter().find(|(oid, parts)| {
            !used.contains(*oid)
                && basic_index(parts, observed, (&table.target_schema, &table.target_name))
                && index_shape(parts, expected)
        });
        if let Some((oid, parts)) = found {
            used.insert(*oid);
            expected.name = parts[0].index_name.clone();
        } else {
            error(
                plan,
                "TARGET_INDEX_INCOMPATIBLE",
                &expected.name,
                "required ordered index/key shape is missing or incompatible",
            );
        }
    }
    if groups.values().any(|p| {
        p[0].is_primary
            && !table
                .structure
                .indexes
                .iter()
                .any(|e| e.primary && e.name == p[0].index_name)
    }) {
        error(
            plan,
            "TARGET_PRIMARY_KEY_INCOMPATIBLE",
            &table.source_name,
            "target primary key differs from the explicitly resolved key policy",
        );
    }
}
fn action(code: &str) -> Option<&'static str> {
    match code {
        "a" => Some("NO ACTION"),
        "r" => Some("RESTRICT"),
        "c" => Some("CASCADE"),
        "n" => Some("SET NULL"),
        "d" => Some("SET DEFAULT"),
        _ => None,
    }
}
fn linked_constraint_trigger(
    parts: &[&TargetConstraintPart],
    observed: &TargetTableObserved,
    table: &TablePlan,
) -> bool {
    let first = parts[0];
    if first.constraint_oid == 0
        || !first.validated
        || first.definition.is_empty()
        || first.ordinal.is_some()
        || first.column_name.is_some()
        || first.column_expression.is_some()
        || first.check_expression.is_some()
        || (first.initially_deferred && !first.deferrable)
        || parts.iter().any(|part| **part != *first)
    {
        return false;
    }
    let mut linked = observed
        .trigger_parts
        .iter()
        .filter(|trigger| trigger.constraint_oid == Some(first.constraint_oid));
    let Some(trigger) = linked.next() else {
        return false;
    };
    if trigger.trigger_oid == 0
        || trigger.scope != "table"
        || trigger.table_oid != Some(observed.oid)
        || trigger.table_schema.as_deref() != Some(&table.target_schema)
        || trigger.table_name.as_deref() != Some(&table.target_name)
        || trigger.constraint_name.as_deref() != Some(&first.name)
        || trigger.trigger_name != first.name
        || trigger.internal != Some(false)
        || trigger.trigger_parent_oid.is_some()
        || !matches!(trigger.enabled.as_str(), "O" | "D" | "R" | "A")
        || !trigger.event_flags.is_some_and(|flags| {
            flags & 1 == 1 && flags & (4 | 8 | 16) != 0 && flags & !(1 | 4 | 8 | 16) == 0
        })
        || trigger.definition.as_ref().is_none_or(String::is_empty)
        || trigger.function_oid == 0
        || trigger.function_schema.is_empty()
        || trigger.function_name.is_empty()
        || trigger.function_identity.is_empty()
        || trigger.function_definition.is_empty()
        || trigger.function_language.is_empty()
        || trigger.function_owner_role.is_empty()
        || !matches!(trigger.function_volatility.as_str(), "i" | "s" | "v")
    {
        return false;
    }
    linked.all(|part| {
        // Only the repeated dependency edge may vary; preserve every header fact.
        let mut header = part.clone();
        header.dependency = trigger.dependency.clone();
        header.dependency_subject = trigger.dependency_subject.clone();
        header == *trigger
    })
}

fn bind_constraints(
    observed: &TargetTableObserved,
    table: &mut TablePlan,
    plan: &mut MigrationPlan,
) {
    let mut groups: BTreeMap<u32, Vec<&TargetConstraintPart>> = BTreeMap::new();
    for p in &observed.constraint_parts {
        groups.entry(p.constraint_oid).or_default().push(p);
    }
    for parts in groups.values_mut() {
        parts.sort_by_key(|p| p.ordinal);
    }
    let planned_not_null: BTreeSet<_> = table
        .columns
        .iter()
        .filter(|column| !column.nullable)
        .map(|column| column.target_name.clone())
        .collect();
    let observed_not_null: BTreeSet<_> = observed
        .columns
        .iter()
        .filter(|column| !column.nullable)
        .map(|column| column.name.clone())
        .collect();
    let mut seen_not_null = BTreeSet::new();
    for trigger in &observed.trigger_parts {
        if trigger.internal == Some(false)
            && let Some(oid) = trigger.constraint_oid
            && groups.get(&oid).is_none_or(|parts| parts[0].kind != "t")
        {
            error(
                plan,
                "TARGET_CONSTRAINT_UNSUPPORTED",
                &trigger.trigger_name,
                "user constraint trigger has no positively linked constraint header",
            );
        }
    }
    for parts in groups.values() {
        let first = parts[0];
        if first.kind == "t" {
            if !linked_constraint_trigger(parts, observed, table) {
                error(
                    plan,
                    "TARGET_CONSTRAINT_UNSUPPORTED",
                    &first.name,
                    "constraint trigger has missing, inconsistent or unsupported linked observations",
                );
            }
            continue;
        }
        if first.kind == "n" {
            let name = supported_not_null_constraint(parts, &planned_not_null, &observed_not_null);
            if name.is_none_or(|column| !seen_not_null.insert(column)) {
                error(
                    plan,
                    "TARGET_CONSTRAINT_UNSUPPORTED",
                    &first.name,
                    "not-null constraint has missing, inconsistent or unsupported column linkage",
                );
            }
            continue;
        }
        if matches!(first.kind.as_str(), "p" | "u") && first.deferrable {
            let linked = indexes(observed);
            if !first
                .supporting_index_oid
                .and_then(|oid| linked.get(&oid))
                .is_some_and(|index| {
                    basic_index(index, observed, (&table.target_schema, &table.target_name))
                        && linked_deferred_key(
                            index,
                            observed,
                            (&table.target_schema, &table.target_name),
                        )
                })
            {
                error(
                    plan,
                    "TARGET_CONSTRAINT_UNSUPPORTED",
                    &first.name,
                    "deferred key has missing, inconsistent or unsupported ordered index linkage",
                );
            }
            continue;
        }
        if !matches!(first.kind.as_str(), "p" | "u" | "f" | "c")
            || parts
                .iter()
                .any(|p| !p.validated || p.deferrable || p.initially_deferred)
            || (first.kind == "f"
                && (first.match_type != "s" || first.delete_set_columns.is_some()))
        {
            error(
                plan,
                "TARGET_CONSTRAINT_UNSUPPORTED",
                &first.name,
                "extra/required constraint has unknown, unvalidated, deferred or unsupported matching semantics",
            );
        }
    }
    for expected in &mut table.structure.foreign_keys {
        if let Some(parts) = groups.values().find(|parts| {
            parts[0].kind == "f"
                && parts.iter().enumerate().all(|(i, p)| {
                    p.ordinal == Some(i as i64 + 1)
                        && p.referenced_schema.as_deref() == Some(&expected.referenced_schema)
                        && p.referenced_table.as_deref() == Some(&expected.referenced_table)
                        && action(&p.update_action) == Some(expected.on_update.as_str())
                        && action(&p.delete_action) == Some(expected.on_delete.as_str())
                })
                && parts
                    .iter()
                    .filter_map(|p| p.column_name.as_ref())
                    .eq(expected.columns.iter())
                && parts
                    .iter()
                    .filter_map(|p| p.referenced_column.as_ref())
                    .eq(expected.referenced_columns.iter())
        }) {
            expected.name = parts[0].name.clone();
        } else {
            error(
                plan,
                "TARGET_FK_INCOMPATIBLE",
                &expected.name,
                "required ordered foreign key endpoints/actions are missing or incompatible",
            );
        }
    }
    for expected in &mut table.structure.checks {
        if let Some(parts) = groups.values().find(|parts| {
            parts[0].kind == "c"
                && parts[0]
                    .check_expression
                    .as_deref()
                    .is_some_and(|expression| {
                        expected.set_membership.as_ref().map_or(
                            expression == expected.expression,
                            |membership| {
                                crate::verify::set_membership_matches(expression, membership)
                                    == Some(true)
                            },
                        )
                    })
        }) {
            expected.name = parts[0].name.clone();
        } else {
            error(
                plan,
                "TARGET_CHECK_INCOMPATIBLE",
                &expected.name,
                "required supported CHECK is missing or incompatible",
            );
        }
    }
}

fn supported_not_null_constraint(
    parts: &[&TargetConstraintPart],
    planned_not_null: &BTreeSet<String>,
    observed_not_null: &BTreeSet<String>,
) -> Option<String> {
    let [part] = parts else {
        return None;
    };
    if !part.validated
        || part.kind != "n"
        || part.deferrable
        || part.initially_deferred
        || part.no_inherit
        || part.ordinal != Some(1)
        || part.column_expression.is_some()
        || part.check_expression.is_some()
        || part.referenced_table_oid.is_some()
        || part.referenced_column.is_some()
        || part.referenced_schema.is_some()
        || part.referenced_table.is_some()
        || part.supporting_index_oid.is_some()
        || part.delete_set_columns.is_some()
        || part.pk_fk_operator_oid.is_some()
        || part.pk_fk_operator.is_some()
        || part.pk_pk_operator_oid.is_some()
        || part.pk_pk_operator.is_some()
        || part.fk_fk_operator_oid.is_some()
        || part.fk_fk_operator.is_some()
    {
        return None;
    }
    let column = part.column_name.as_ref()?;
    (planned_not_null.contains(column) && observed_not_null.contains(column))
        .then(|| column.clone())
}

fn identity(
    config: &MigrationConfig,
    observed: &TargetTableObserved,
    table: &mut TablePlan,
    plan: &mut MigrationPlan,
) {
    for column in table.columns.iter().filter(|c| c.identity) {
        let parts: Vec<_> = observed
            .sequence_parts
            .iter()
            .filter(|p| p.bound_column_name == column.target_name)
            .collect();
        let Some(first) = parts.first().copied() else {
            error(
                plan,
                "TARGET_SEQUENCE_INCOMPATIBLE",
                &column.source_name,
                "identity sequence was not positively inspected",
            );
            continue;
        };
        if parts.iter().any(|p| {
            p.sequence_oid != first.sequence_oid
                || p.bound_identity_mode != "d"
                || p.ownership_dependency.as_deref() != Some("i")
                || p.owner_table_schema.as_deref() != Some(&table.target_schema)
                || p.owner_table_name.as_deref() != Some(&table.target_name)
                || p.owner_column_name.as_deref() != Some(&column.target_name)
                || p.extension_name.is_some()
                || p.increment_by != 1
                || p.min_value != 1
                || Some(p.max_value) != sequence_max(&column.target_type)
                || p.cycle
                || p.cache_size != 1
                || !p.can_select
                || (config.migration.reset_sequences && !p.can_update)
        }) {
            error(
                plan,
                "TARGET_SEQUENCE_INCOMPATIBLE",
                &column.source_name,
                "existing generation requires visible column-owned BY DEFAULT identity with unit/min1/cache1/noncycling range and requested privileges",
            );
            continue;
        }
        let SequenceStateObservation::Known {
            last_value,
            is_called,
        } = first.state
        else {
            error(
                plan,
                "TARGET_SEQUENCE_STATE_UNKNOWN",
                &column.source_name,
                "identity current state is unavailable",
            );
            continue;
        };
        let Some(next) = last_value.checked_add(i64::from(is_called)) else {
            error(
                plan,
                "TARGET_SEQUENCE_EXHAUSTED",
                &column.source_name,
                "identity next value is exhausted",
            );
            continue;
        };
        if next < 1 || next > first.max_value {
            error(
                plan,
                "TARGET_SEQUENCE_EXHAUSTED",
                &column.source_name,
                "identity next value is outside its supported range",
            );
            continue;
        }
        table.structure.sequences.push(SequenceExpectation {
            column: column.target_name.clone(),
            increment: 1,
            min_value: 1,
            max_value: first.max_value,
            cycle: false,
            next_minimum: if config.migration.reset_sequences {
                table.next_auto_increment.unwrap_or(1).max(next as u64)
            } else {
                next as u64
            },
            preserved_state: (!config.migration.reset_sequences).then_some(
                SequenceStateObservation::Known {
                    last_value,
                    is_called,
                },
            ),
        });
    }
}

type Key = (String, u32, i32);
fn covers(set: &BTreeSet<Key>, key: &Key) -> bool {
    set.contains(key) || set.contains(&(key.0.clone(), key.1, 0))
}
fn endpoints(edge: &TargetDependencyEdge) -> Option<(Key, Key)> {
    let d = &edge.dependency;
    Some((
        (
            d.dependent_catalog.clone()?,
            d.dependent_oid?,
            d.dependent_sub_id?,
        ),
        (
            d.referenced_catalog.clone()?,
            d.referenced_oid?,
            d.referenced_sub_id?,
        ),
    ))
}
fn destructive(config: &MigrationConfig, target: &TargetCatalog, plan: &mut MigrationPlan) {
    let selected: BTreeSet<_> = plan
        .tables
        .iter()
        .filter_map(|t| {
            target
                .tables
                .iter()
                .find(|e| e.schema == t.target_schema && e.name == t.target_name)
                .and_then(|e| e.observed.as_ref())
                .map(|o| ("pg_catalog.pg_class".into(), o.oid, 0))
        })
        .collect();
    if selected.is_empty() {
        return;
    }
    let Some(graph) = &target.dependency_graph else {
        error(
            plan,
            "TARGET_DEPENDENCY_METADATA_REQUIRED",
            "target",
            "existing policy requires a positive raw dependency inventory",
        );
        return;
    };
    if !valid_graph(graph) {
        error(
            plan,
            "TARGET_DEPENDENCY_MALFORMED",
            "target",
            "raw graph identities/header/provenance are inconsistent",
        );
        return;
    }
    for key in &selected {
        if !graph.parts.iter().any(|p| {
            p.node_catalog == key.0 && p.node_oid == key.1 && p.node_sub_id == 0 && p.relation_root
        }) {
            error(
                plan,
                "TARGET_DEPENDENCY_ROOT_MISSING",
                "target",
                "selected target does not have a positively inspected dependency root",
            );
        }
    }
    let owned = automatic_closure(graph, &selected);
    for node in &graph.parts {
        let key = (node.node_catalog.clone(), node.node_oid, node.node_sub_id);
        if covers(
            &owned,
            &(node.node_catalog.clone(), node.node_oid, node.node_sub_id),
        ) && node.node_extension_oid.is_some()
        {
            error(
                plan,
                "TARGET_EXTENSION_MEMBER",
                &node.node_identity,
                "selected target belongs to an extension",
            );
        }
        if covers(&owned, &key)
            && !matches!(
                node.node_catalog.as_str(),
                "pg_catalog.pg_class"
                    | "pg_catalog.pg_type"
                    | "pg_catalog.pg_attrdef"
                    | "pg_catalog.pg_constraint"
                    | "pg_catalog.pg_trigger"
            )
        {
            error(
                plan,
                "TARGET_OWNED_OBJECT_UNSUPPORTED",
                &node.node_identity,
                "selected target owns an unsupported object class",
            );
        }
        if let Some(edge) = &node.edge
            && let Some((dependent, reference)) = endpoints(edge)
            && (covers(&owned, &dependent) || covers(&owned, &reference))
        {
            let known = match edge.dependency.dependency_catalog.as_deref() {
                Some("pg_catalog.pg_depend") => matches!(
                    edge.dependency.dependency_kind.as_deref(),
                    Some("n" | "a" | "i")
                ),
                Some("pg_catalog.pg_shdepend") => {
                    matches!(edge.dependency.dependency_kind.as_deref(), Some("o" | "a"))
                }
                _ => false,
            };
            if !known {
                error(
                    plan,
                    "TARGET_DEPENDENCY_UNSUPPORTED",
                    &node.node_identity,
                    "selected target intersects unknown, extension or partition dependency semantics",
                );
            }
        }
    }
    if config.target.on_existing == ExistingPolicy::Recreate {
        let owned = automatic_closure(graph, &selected);
        for table in target
            .tables
            .iter()
            .filter_map(|t| t.observed.as_ref())
            .filter(|t| selected.contains(&("pg_catalog.pg_class".into(), t.oid, 0)))
        {
            let objects = table
                .index_parts
                .iter()
                .map(|p| ("pg_catalog.pg_class", p.index_oid, p.index_name.as_str()))
                .chain(table.constraint_parts.iter().map(|p| {
                    (
                        "pg_catalog.pg_constraint",
                        p.constraint_oid,
                        p.name.as_str(),
                    )
                }))
                .chain(table.trigger_parts.iter().map(|p| {
                    (
                        "pg_catalog.pg_trigger",
                        p.trigger_oid,
                        p.trigger_name.as_str(),
                    )
                }));
            for (catalog, oid, name) in objects {
                if !owned.contains(&(catalog.into(), oid, 0))
                    || !graph.parts.iter().any(|p| {
                        p.node_catalog == catalog
                            && p.node_oid == oid
                            && p.node_sub_id == 0
                            && p.node_owner_relation_oid == Some(table.oid)
                            && p.node_extension_oid.is_none()
                    })
                {
                    error(
                        plan,
                        "TARGET_OWNED_OBJECT_PROOF_MISSING",
                        name,
                        "observed removed object lacks exact selected automatic/internal ownership proof",
                    );
                }
            }
        }
        drop_closure(graph, &selected, plan);
    } else if config.target.on_existing == ExistingPolicy::Truncate {
        for node in &graph.parts {
            if node.node_catalog == "pg_catalog.pg_constraint"
                && node.node_constraint_kind.as_deref() == Some("f")
                && !node
                    .node_owner_relation_oid
                    .is_some_and(|oid| selected.contains(&("pg_catalog.pg_class".into(), oid, 0)))
                && node.edge.as_ref().is_some_and(|e| {
                    endpoints(e).is_some_and(|(_, reference)| covers(&selected, &reference))
                })
            {
                error(
                    plan,
                    "TARGET_DEPENDENCY_OUTSIDE_SELECTION",
                    &node.node_identity,
                    "unselected foreign-key owner references a selected truncate table",
                );
            }
        }
    }
}

fn valid_graph(graph: &TargetDependencyGraph) -> bool {
    if graph.database_oid == 0 {
        return false;
    }
    let mut classes = BTreeMap::new();
    let mut class_names = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    let mut relations = BTreeSet::new();
    let mut types = BTreeSet::new();
    let mut events = BTreeSet::new();
    for p in &graph.parts {
        if p.node_class_oid == 0
            || p.node_oid == 0
            || p.node_sub_id < 0
            || p.node_catalog.is_empty()
            || p.node_type.is_empty()
            || p.node_identity.is_empty()
        {
            return false;
        }
        if classes
            .insert(p.node_catalog.clone(), p.node_class_oid)
            .is_some_and(|old| old != p.node_class_oid)
            || class_names
                .insert(p.node_class_oid, p.node_catalog.clone())
                .is_some_and(|old| old != p.node_catalog)
        {
            return false;
        }
        let key = (p.node_catalog.as_str(), p.node_oid, p.node_sub_id);
        let facts = (
            &p.node_type,
            &p.node_identity,
            &p.node_schema,
            &p.node_name,
            p.relation_root,
            p.type_root,
            p.global_event_root,
            p.node_extension_oid,
        );
        if nodes.insert(key, facts).is_some_and(|old| old != facts) {
            return false;
        }
        if p.relation_root {
            if p.node_catalog != "pg_catalog.pg_class" || p.node_sub_id != 0 {
                return false;
            }
            relations.insert(p.node_oid);
        }
        if p.type_root {
            if p.node_catalog != "pg_catalog.pg_type" || p.node_sub_id != 0 {
                return false;
            }
            types.insert(p.node_oid);
        }
        if p.global_event_root {
            if p.node_catalog != "pg_catalog.pg_event_trigger" || p.node_sub_id != 0 {
                return false;
            }
            events.insert(p.node_oid);
        }
        if let Some(e) = &p.edge {
            let d = &e.dependency;
            let Some((dependent, reference)) = endpoints(e) else {
                return false;
            };
            if e.dependent_class_oid == 0
                || e.referenced_class_oid == 0
                || dependent.1 == 0
                || reference.1 == 0
                || dependent.2 < 0
                || reference.2 < 0
                || d.dependency_kind.as_ref().is_none_or(|v| v.len() != 1)
            {
                return false;
            }
            for (catalog, class_oid) in [
                (&dependent.0, e.dependent_class_oid),
                (&reference.0, e.referenced_class_oid),
            ] {
                if classes
                    .insert(catalog.clone(), class_oid)
                    .is_some_and(|old| old != class_oid)
                    || class_names
                        .insert(class_oid, catalog.clone())
                        .is_some_and(|old| old != *catalog)
                {
                    return false;
                }
            }
            let owns = |key: &Key| {
                key.0 == p.node_catalog
                    && key.1 == p.node_oid
                    && (p.node_sub_id == 0 || key.2 == p.node_sub_id)
            };
            if !match (
                d.dependency_catalog.as_deref(),
                d.dependency_direction.as_deref(),
            ) {
                (Some("pg_catalog.pg_depend"), Some("incoming")) => owns(&reference),
                (Some("pg_catalog.pg_depend" | "pg_catalog.pg_shdepend"), Some("outgoing")) => {
                    owns(&dependent)
                }
                _ => false,
            } {
                return false;
            }
        }
    }
    relations.len() as u64 == graph.relation_root_count
        && types.len() as u64 == graph.type_root_count
        && events.len() as u64 == graph.event_trigger_count
}
fn automatic_closure(graph: &TargetDependencyGraph, selected: &BTreeSet<Key>) -> BTreeSet<Key> {
    let mut allowed = selected.clone();
    loop {
        let before = allowed.len();
        for node in &graph.parts {
            if let Some(edge) = &node.edge
                && edge.dependency.dependency_catalog.as_deref() == Some("pg_catalog.pg_depend")
                && matches!(edge.dependency.dependency_kind.as_deref(), Some("a" | "i"))
                && let Some((dependent, reference)) = endpoints(edge)
                && covers(&allowed, &reference)
            {
                allowed.insert(dependent);
            }
        }
        if before == allowed.len() {
            break;
        }
    }
    allowed
}
pub(super) fn relation_is_replaced(
    target: &TargetCatalog,
    schema: &str,
    name: &str,
    tables: &[TablePlan],
) -> bool {
    let Some(graph) = &target.dependency_graph else {
        return false;
    };
    let selected: BTreeSet<_> = tables
        .iter()
        .filter_map(|t| {
            target
                .tables
                .iter()
                .find(|e| e.schema == t.target_schema && e.name == t.target_name)
                .and_then(|e| e.observed.as_ref())
                .map(|o| ("pg_catalog.pg_class".into(), o.oid, 0))
        })
        .collect();
    let allowed = automatic_closure(graph, &selected);
    target
        .tables
        .iter()
        .filter_map(|t| t.observed.as_ref())
        .any(|table| {
            selected.contains(&("pg_catalog.pg_class".into(), table.oid, 0))
                && table.index_parts.iter().any(|index| {
                    index.index_schema == schema
                        && index.index_name == name
                        && index.table_oid == table.oid
                        && allowed.contains(&("pg_catalog.pg_class".into(), index.index_oid, 0))
                        && graph.parts.iter().any(|p| {
                            p.node_catalog == "pg_catalog.pg_class"
                                && p.node_oid == index.index_oid
                                && p.node_sub_id == 0
                                && p.node_owner_relation_oid == Some(table.oid)
                                && p.node_extension_oid.is_none()
                                && p.node_relation_kind.as_deref() == Some("i")
                        })
                })
        })
}
fn drop_closure(graph: &TargetDependencyGraph, selected: &BTreeSet<Key>, plan: &mut MigrationPlan) {
    let allowed = automatic_closure(graph, selected);
    for node in &graph.parts {
        let key = (node.node_catalog.clone(), node.node_oid, node.node_sub_id);
        if node.global_event_root && node.node_event_enabled.as_deref() != Some("D") {
            error(
                plan,
                "TARGET_EVENT_TRIGGER",
                &node.node_identity,
                "enabled global event trigger blocks unreviewed destructive DDL",
            );
        }
        if covers(&allowed, &key)
            && (node.node_extension_oid.is_some()
                || !matches!(
                    node.node_catalog.as_str(),
                    "pg_catalog.pg_class"
                        | "pg_catalog.pg_type"
                        | "pg_catalog.pg_attrdef"
                        | "pg_catalog.pg_constraint"
                        | "pg_catalog.pg_trigger"
                ))
        {
            error(
                plan,
                "TARGET_DROP_OBJECT_UNSUPPORTED",
                &node.node_identity,
                "selected owned drop closure contains extension or unsupported object class",
            );
        }
        if let Some(edge) = &node.edge {
            let Some((dependent, reference)) = endpoints(edge) else {
                error(
                    plan,
                    "TARGET_DEPENDENCY_MALFORMED",
                    &node.node_identity,
                    "dependency endpoints are incomplete",
                );
                continue;
            };
            match edge.dependency.dependency_catalog.as_deref() {
                Some("pg_catalog.pg_depend") => {
                    if (covers(&allowed, &dependent) || covers(&allowed, &reference))
                        && !matches!(
                            edge.dependency.dependency_kind.as_deref(),
                            Some("n" | "a" | "i")
                        )
                    {
                        error(
                            plan,
                            "TARGET_DEPENDENCY_UNSUPPORTED",
                            &node.node_identity,
                            "unknown/extension/partition dependency intersects selected drop closure",
                        );
                    }
                    if covers(&allowed, &reference) && !covers(&allowed, &dependent) {
                        error(
                            plan,
                            "TARGET_DEPENDENCY_OUTSIDE_SELECTION",
                            &node.node_identity,
                            "external incoming user is outside explicitly selected automatic/internal drop closure",
                        );
                    }
                }
                Some("pg_catalog.pg_shdepend") => {
                    if covers(&allowed, &dependent)
                        && !matches!(edge.dependency.dependency_kind.as_deref(), Some("o" | "a"))
                    {
                        error(
                            plan,
                            "TARGET_DEPENDENCY_UNSUPPORTED",
                            &node.node_identity,
                            "unknown shared dependency intersects selected drop closure",
                        );
                    }
                }
                _ => error(
                    plan,
                    "TARGET_DEPENDENCY_MALFORMED",
                    &node.node_identity,
                    "dependency catalog is unknown",
                ),
            }
        }
    }
}

#[cfg(test)]
mod not_null_catalog_tests {
    use super::*;

    fn part() -> TargetConstraintPart {
        TargetConstraintPart {
            constraint_oid: 12,
            name: "rows_id_not_null".into(),
            kind: "n".into(),
            definition: "NOT NULL id".into(),
            validated: true,
            deferrable: false,
            initially_deferred: false,
            no_inherit: false,
            match_type: "".into(),
            update_action: "".into(),
            delete_action: "".into(),
            ordinal: Some(1),
            column_name: Some("id".into()),
            column_expression: None,
            check_expression: None,
            referenced_table_oid: None,
            referenced_column: None,
            referenced_schema: None,
            referenced_table: None,
            supporting_index_oid: None,
            supporting_index_schema: None,
            supporting_index_name: None,
            delete_set_columns: None,
            pk_fk_operator_oid: None,
            pk_fk_operator: None,
            pk_pk_operator_oid: None,
            pk_pk_operator: None,
            fk_fk_operator_oid: None,
            fk_fk_operator: None,
        }
    }

    #[test]
    fn postgres18_not_null_rows_need_valid_single_column_nonnull_linkage() {
        let expected = BTreeSet::from(["id".to_owned()]);
        let observed = BTreeSet::from(["id".to_owned()]);
        let part = part();
        assert_eq!(
            supported_not_null_constraint(&[&part], &expected, &observed),
            Some("id".to_owned())
        );

        let mut unvalidated = part.clone();
        unvalidated.validated = false;
        assert!(supported_not_null_constraint(&[&unvalidated], &expected, &observed).is_none());
        let mut deferrable = part.clone();
        deferrable.deferrable = true;
        assert!(supported_not_null_constraint(&[&deferrable], &expected, &observed).is_none());
        let mut no_inherit = part.clone();
        no_inherit.no_inherit = true;
        assert!(supported_not_null_constraint(&[&no_inherit], &expected, &observed).is_none());
        let mut malformed_link = part.clone();
        malformed_link.ordinal = None;
        assert!(supported_not_null_constraint(&[&malformed_link], &expected, &observed).is_none());
        assert!(supported_not_null_constraint(&[&part], &BTreeSet::new(), &observed).is_none());
        assert!(supported_not_null_constraint(&[&part], &expected, &BTreeSet::new()).is_none());
        assert!(supported_not_null_constraint(&[&part, &part], &expected, &observed).is_none());
    }

    #[test]
    fn postgres18_not_null_catalog_rows_are_known_for_relation_recreation() {
        assert!(recreatable_constraint_kind("n"));
        assert!(!recreatable_constraint_kind("?"));
    }
}
