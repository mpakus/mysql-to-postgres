use super::*;

#[test]
fn existing_type_signatures_keep_exact_modifiers_and_timezone() {
    assert!(compatible_type(
        "timestamp(6) with time zone",
        "timestamptz(6)"
    ));
    assert!(compatible_type("numeric(65, 30)", "numeric(65,30)"));
    assert!(compatible_type("character varying(24)", "varchar(24)"));
    assert!(!compatible_type(
        "timestamp(3) with time zone",
        "timestamptz(6)"
    ));
    assert!(!compatible_type(
        "timestamp(6) without time zone",
        "timestamptz(6)"
    ));
    assert!(!compatible_type("numeric(65,29)", "numeric(65,30)"));
    assert!(!compatible_type("unknown_type", "unknown_type"));
}

#[test]
fn explicitly_selected_existing_view_is_planned_as_a_snapshot_table() {
    let (mut config, mut source, target) = inputs();
    let view = &mut source.tables[0];
    view.is_view = true;
    view.engine = "VIEW".into();
    config.source.consistency = Consistency::SingleSnapshot;
    config.tables.include = vec![view.name.clone()];
    config.tables.views = vec![view.name.clone()];

    let plan = build(&config, &source, &target).unwrap();
    assert_eq!(plan.tables.len(), 1);
    assert!(plan.tables[0].is_view);
    assert!(plan.ddl.iter().any(|step| {
        step.sql.starts_with("CREATE TABLE")
            && step
                .sql
                .contains(&quote_identifier(&plan.tables[0].target_name))
    }));

    config.tables.views.clear();
    assert!(build(&config, &source, &target).unwrap().tables.is_empty());
}

#[test]
fn hooks_are_hashed_and_kept_in_configured_before_then_after_order() {
    let (mut config, source, target) = inputs();
    let directory = std::env::temp_dir().join(format!(
        "my2pg-plan-hooks-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let before_a = directory.join("before-a.sql");
    let before_b = directory.join("before-b.sql");
    let after = directory.join("after.sql");
    std::fs::write(&before_a, "SELECT 1;").unwrap();
    std::fs::write(&before_b, "SELECT 2;").unwrap();
    std::fs::write(&after, "SELECT 3;").unwrap();
    config.hooks.before = vec![before_a.clone(), before_b.clone()];
    config.hooks.after = vec![after.clone()];

    let plan = build(&config, &source, &target).unwrap();
    assert_eq!(
        plan.hooks
            .iter()
            .map(|hook| hook.before)
            .collect::<Vec<_>>(),
        [true, true, false]
    );
    assert_eq!(plan.hooks[0].path, before_a.to_string_lossy());
    assert_eq!(plan.hooks[1].path, before_b.to_string_lossy());
    assert_eq!(plan.hooks[2].path, after.to_string_lossy());
    for hook in &plan.hooks {
        let bytes = std::fs::read(&hook.path).unwrap();
        assert_eq!(hook.sha256, format!("{:x}", Sha256::digest(bytes)));
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn existing_topology_requires_positive_proof_in_every_valid_mode_and_policy() {
    let (mut config, mut source, mut target) = inputs();
    source.tables[0].columns[0].extra.clear();
    source.tables[0].next_auto_increment = None;
    target.schema_exists = true;
    // Legacy serialized catalogs have no topology proof and must fail closed.
    let existing: TargetTable = serde_json::from_value(serde_json::json!({
        "schema":"legacy","name":"camel_case","row_count":3,
        "columns":[{"name":"ID","data_type":"bigint","nullable":false,"generated":false,"identity":false}],
        "can_insert":true,"can_select":true,"can_alter":true,"can_truncate":true,"row_security_active":false
    })).unwrap();
    assert!(!existing.ordinary_standalone);
    target.tables.push(existing);
    for (mode, policy) in [
        (MigrationMode::Full, ExistingPolicy::Error),
        (MigrationMode::Full, ExistingPolicy::Recreate),
        (MigrationMode::Full, ExistingPolicy::Truncate),
        (MigrationMode::SchemaOnly, ExistingPolicy::Error),
        (MigrationMode::SchemaOnly, ExistingPolicy::Recreate),
        (MigrationMode::DataOnly, ExistingPolicy::Append),
        (MigrationMode::DataOnly, ExistingPolicy::Truncate),
    ] {
        config.migration.mode = mode;
        config.migration.reset_sequences = false;
        config.target.on_existing = policy;
        assert!(
            codes(build(&config, &source, &target)).contains(&"TARGET_TABLE_TOPOLOGY".into()),
            "missing positive topology proof: {mode:?}/{policy:?}"
        );
        target.tables[0].ordinary_standalone = true;
        let result = build(&config, &source, &target);
        if policy == ExistingPolicy::Error {
            let diagnostics = codes(result);
            assert!(diagnostics.contains(&"TARGET_EXISTS".into()));
            assert!(!diagnostics.contains(&"TARGET_TABLE_TOPOLOGY".into()));
        } else {
            let diagnostics = codes(result);
            assert!(diagnostics.contains(&"TARGET_OBSERVATION_REQUIRED".into()));
            assert!(!diagnostics.contains(&"TARGET_TABLE_TOPOLOGY".into()));
        }
        target.tables[0].ordinary_standalone = false;
    }
    // A separate unselected/public table's unknown topology does not block selected DDL.
    config.migration.mode = MigrationMode::Full;
    config.target.on_existing = ExistingPolicy::Error;
    target.tables[0].schema = "public".into();
    build(&config, &source, &target).unwrap();
}

fn inputs() -> (MigrationConfig, SourceCatalog, TargetCatalog) {
    (
        toml::from_str(include_str!("../../tests/contracts/config.toml")).unwrap(),
        serde_json::from_str(include_str!("../../tests/contracts/source-catalog.json")).unwrap(),
        TargetCatalog {
            server_version: "16.15".into(),
            can_create_schema: true,
            can_use_schema: true,
            can_create_objects: true,
            ..Default::default()
        },
    )
}

fn generated_inputs() -> (MigrationConfig, SourceCatalog, TargetCatalog) {
    let (config, mut source, target) = inputs();
    let mut source_text = column(&source, "source_text", "varchar");
    source_text.ordinal = 2;
    source_text.character_length = Some(32);
    source_text.nullable = false;
    let mut derived = column(&source, "derived", "varchar");
    derived.ordinal = 3;
    derived.character_length = Some(32);
    derived.generation_expression = Some("LOWER(`source_text`)".into());
    derived.extra = "STORED GENERATED".into();
    source.tables[0].columns.extend([source_text, derived]);
    (config, source, target)
}

#[test]
fn typed_temporal_defaults_reject_calendar_and_precision_errors_before_ddl() {
    let (config, mut source, target) = inputs();
    let mut date = column(&source, "date_value", "datetime");
    date.column_type = "datetime(3)".into();
    date.datetime_precision = Some(3);
    date.default = Some("2024-02-29 01:02:03.123".into());
    source.tables[0].columns.push(date);
    let plan = build(&config, &source, &target).unwrap();
    assert_eq!(
        plan.tables[0]
            .columns
            .last()
            .unwrap()
            .default_sql
            .as_deref(),
        Some("E'2024-02-29 01:02:03.123000'")
    );
    for invalid in [
        "2023-02-29 00:00:00",
        "2024-02-29 01:02:03.123001",
        "2024-13-01 00:00:00",
    ] {
        source.tables[0].columns.last_mut().unwrap().default = Some(invalid.into());
        assert!(codes(build(&config, &source, &target)).contains(&"DEFAULT_UNSUPPORTED".into()));
    }
}

#[test]
fn enum_label_order_escaping_defaults_and_long_label_policy_are_exact() {
    let (config, mut source, target) = inputs();
    let mut enumeration = column(&source, "state", "enum");
    enumeration.column_type = "enum('','a,b','quote''label','slash\\\\label')".into();
    enumeration.default = Some("".into());
    source.tables[0].columns.push(enumeration);
    let resolved = build(&config, &source, &target).unwrap();
    let column = resolved.tables[0].columns.last().unwrap();
    assert_eq!(
        column.enum_labels,
        ["", "a,b", "quote'label", "slash\\label"]
    );
    assert_eq!(column.default_sql.as_deref(), Some("E''"));
    assert!(resolved.ddl.iter().any(|step| {
        step.sql
            .contains("CREATE TYPE \"legacy\".\"enum_camel_case_state_")
            && step.sql.contains("E'quote''label'")
            && step.sql.contains("E'slash\\\\label'")
    }));
    source.tables[0].columns.last_mut().unwrap().column_type =
        format!("enum('{}')", "😀".repeat(16));
    source.tables[0].columns.last_mut().unwrap().default = None;
    assert!(codes(build(&config, &source, &target)).contains(&"ENUM_LABEL_TOO_LONG".into()));
}

#[test]
fn ambiguous_enum_set_metadata_is_blocked_before_casts_and_ineffective_omission() {
    for kind in ["enum", "set"] {
        for charset in [
            "utf8mb4",
            "utf16",
            "utf16le",
            "utf32",
            "gb18030",
            "future_unknown",
        ] {
            let (mut config, mut source, target) = inputs();
            let mut ambiguous = column(&source, "ambiguous", kind);
            ambiguous.charset = Some(charset.into());
            ambiguous.column_type = format!("{kind}('?','plain')");
            source.tables[0].columns.push(ambiguous);
            assert!(
                codes(build(&config, &source, &target)).contains(&"LOSSY_LABEL_METADATA".into()),
                "{kind}/{charset}"
            );
            config.cast.push(CastRule {
                source_table: Some("CamelCase".into()),
                source_column: Some("ambiguous".into()),
                target_type: Some("text".into()),
                charset: Some("utf8mb3".into()),
                ..Default::default()
            });
            assert!(
                codes(build(&config, &source, &target)).contains(&"LOSSY_LABEL_METADATA".into()),
                "text override cannot recover source labels"
            );
            config.overrides.push(ObjectOverride {
                object: format!("{}.CamelCase.ambiguous", source.database),
                omit: true,
                target_expression: None,
                target_sql: None,
                materialize: None,
            });
            assert!(
                codes(build(&config, &source, &target)).contains(&"LOSSY_LABEL_METADATA".into()),
                "nominal omit does not actually remove source column"
            );
            config.cast.clear();
            source.tables[0].columns.last_mut().unwrap().column_type =
                format!("{kind}('é','plain')");
            build(&config, &source, &target).unwrap();
            source.tables[0].columns.last_mut().unwrap().column_type =
                format!("{kind}('?','plain')");
            source.tables[0].columns.last_mut().unwrap().charset = Some("utf8mb3".into());
            build(&config, &source, &target).unwrap();
        }
    }
    let (mut config, mut source, target) = inputs();
    let mut unselected = source.tables[0].clone();
    unselected.name = "ambiguous_table".into();
    let mut ambiguous = column(&source, "ambiguous", "enum");
    ambiguous.column_type = "enum('?','plain')".into();
    ambiguous.charset = None;
    unselected.charset = Some("utf8mb4".into());
    unselected.columns.push(ambiguous);
    source.tables.push(unselected);
    config.tables.include = vec!["CamelCase".into()];
    build(&config, &source, &target).unwrap();
    config.tables.include.clear();
    config.tables.exclude = vec!["ambiguous_table".into()];
    build(&config, &source, &target).unwrap();
    config.tables.exclude.clear();
    config.tables.include = vec!["CamelCase".into()];
    config.tables.include.push("ambiguous_table".into());
    assert!(
        codes(build(&config, &source, &target)).contains(&"LOSSY_LABEL_METADATA".into()),
        "table charset fallback still prevents loss"
    );
    source.tables[1].charset = None;
    assert!(
        codes(build(&config, &source, &target)).contains(&"LOSSY_LABEL_METADATA".into()),
        "missing charset is not positive BMP-only proof"
    );
    for charset in ["utf8", "utf8mb3", "ascii", "latin1"] {
        source.tables[1].columns.last_mut().unwrap().charset = Some(charset.into());
        build(&config, &source, &target).unwrap();
    }
}

#[test]
fn enum_set_bit_binary_and_transform_defaults_use_the_resolved_value_plan() {
    let (mut config, mut source, target) = inputs();
    let mut set = column(&source, "members", "set");
    set.column_type = "set('a','quote''label','back\\\\slash')".into();
    set.default = Some("a,back\\slash".into());
    source.tables[0].columns.push(set);
    let mut bit = column(&source, "bits", "bit");
    bit.column_type = "bit(8)".into();
    bit.default = Some("b'101'".into());
    source.tables[0].columns.push(bit);
    let mut binary = column(&source, "padded", "binary");
    binary.column_type = "binary(4)".into();
    binary.character_length = Some(4);
    binary.default = Some("0x61".into());
    source.tables[0].columns.push(binary);
    let mut truth = column(&source, "truth", "tinyint");
    truth.default = Some("-1".into());
    source.tables[0].columns.push(truth);
    config.cast.push(CastRule {
        source_table: Some("CamelCase".into()),
        source_column: Some("truth".into()),
        target_type: Some("boolean".into()),
        transform: Some("tinyint-to-boolean".into()),
        ..Default::default()
    });
    let resolved = build(&config, &source, &target).unwrap();
    let table = &resolved.tables[0];
    let defaults: BTreeMap<_, _> = table
        .columns
        .iter()
        .map(|column| (column.source_name.as_str(), column.default_sql.as_deref()))
        .collect();
    assert_eq!(
        defaults["members"],
        Some("E'{\"a\",\"back\\\\\\\\slash\"}'")
    );
    assert_eq!(defaults["bits"], Some("B'00000101'"));
    assert_eq!(defaults["padded"], Some("E'\\\\x61000000'"));
    assert_eq!(defaults["truth"], Some("true"));
    assert_eq!(
        table.structure.checks[0]
            .set_membership
            .as_ref()
            .unwrap()
            .labels,
        ["a", "quote'label", "back\\slash"]
    );
    assert!(resolved.ddl.iter().any(|step| step.sql.contains(
        "CHECK (\"members\" <@ ARRAY[E'a', E'quote''label', E'back\\\\slash']::text[])"
    )));
    source.tables[0]
        .columns
        .iter_mut()
        .find(|column| column.name == "bits")
        .unwrap()
        .default = Some("b'100000000'".into());
    assert!(codes(build(&config, &source, &target)).contains(&"DEFAULT_UNSUPPORTED".into()));
}

#[test]
fn finite_scientific_float_defaults_preserve_source_width() {
    let (mut config, mut source, target) = inputs();
    let mut float = column(&source, "float_value", "float");
    float.default = Some("1e-1".into());
    source.tables[0].columns.push(float);
    config.cast.push(CastRule {
        source_table: Some("CamelCase".into()),
        source_column: Some("float_value".into()),
        target_type: Some("double precision".into()),
        ..Default::default()
    });
    let resolved = build(&config, &source, &target).unwrap();
    assert_eq!(
        resolved.tables[0]
            .columns
            .last()
            .unwrap()
            .default_sql
            .as_deref(),
        Some("0.10000000149011612")
    );
    source.tables[0].columns.last_mut().unwrap().default = Some("1e999".into());
    assert!(codes(build(&config, &source, &target)).contains(&"DEFAULT_UNSUPPORTED".into()));
}

fn column(source: &SourceCatalog, name: &str, kind: &str) -> SourceColumn {
    let mut column = source.tables[0].columns[0].clone();
    column.name = name.into();
    column.data_type = kind.into();
    column.column_type = kind.into();
    column.extra.clear();
    column.default = None;
    column.collation = None;
    column
}

fn codes(result: Result<MigrationPlan, PlanError>) -> Vec<String> {
    let PlanError::Blocked(errors) = result.unwrap_err() else {
        panic!("expected blocking diagnostics")
    };
    errors.into_iter().map(|error| error.code).collect()
}

#[test]
fn basic_plan_is_deterministic_qualified_and_preserves_source_names() {
    let (config, source, mut target) = inputs();
    target.tables.push(TargetTable {
        schema: "public".into(),
        name: "camel_case".into(),
        ordinary_standalone: true,
        columns: vec![],
        row_count: Some(1),
        can_insert: true,
        can_select: true,
        can_alter: true,
        can_truncate: true,
        can_lock: Some(true),
        row_security_active: false,
        observed: None,
    });
    let first = build(&config, &source, &target).unwrap();
    let second = build(&config, &source, &target).unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap()
    );
    let table = &first.tables[0];
    assert_eq!(table.source_name, "CamelCase");
    assert_eq!(table.target_name, "camel_case");
    assert_eq!(table.columns[0].source_name, "ID");
    assert_eq!(table.columns[0].target_type, "bigint");
    assert!(Regex::new(r"^t_[0-9a-f]{16}$").unwrap().is_match(&table.id));
    assert!(
        first
            .ddl
            .iter()
            .any(|step| step.sql.contains("CREATE TABLE \"legacy\".\"camel_case\""))
    );
    assert!(!first.ddl.iter().any(|step| step.sql.contains("public")));
    assert!(first.ddl.iter().any(|step| step.sql.contains("4294967295")));
}

#[test]
fn renamed_target_primary_key_keeps_original_source_reader_identity() {
    let (mut config, mut source, target) = inputs();
    config.migration.identifiers = IdentifierPolicy::SnakeCase;
    source.tables[0].columns[0].name = "MixedKey".into();
    source.tables[0].indexes[0].parts[0].column = Some("MixedKey".into());
    let resolved = build(&config, &source, &target).unwrap();
    let table = &resolved.tables[0];
    assert_eq!(table.primary_key, ["MixedKey"]);
    let target_primary = table
        .structure
        .indexes
        .iter()
        .find(|index| index.primary)
        .unwrap();
    assert_eq!(target_primary.columns, ["mixed_key"]);
    assert!(
        resolved
            .ddl
            .iter()
            .any(|step| step.sql.contains("PRIMARY KEY (\"mixed_key\")"))
    );
    source.tables[0].columns[0].nullable = true;
    assert!(codes(build(&config, &source, &target)).contains(&"PRIMARY_KEY_NULLABLE".into()));
}

#[test]
fn collisions_and_existing_target_block_before_a_runnable_plan() {
    let (mut config, mut source, mut target) = inputs();
    config.tables.rename.clear();
    config.migration.identifiers = IdentifierPolicy::Downcase;
    let mut other = source.tables[0].clone();
    other.name = "camelcase".into();
    source.tables.push(other);
    assert!(codes(build(&config, &source, &target)).contains(&"TABLE_NAME_COLLISION".into()));
    source.tables.pop();
    target.tables.push(TargetTable {
        schema: "legacy".into(),
        name: "camelcase".into(),
        ordinary_standalone: true,
        columns: vec![],
        row_count: Some(1),
        can_insert: true,
        can_select: true,
        can_alter: true,
        can_truncate: true,
        can_lock: Some(true),
        row_security_active: false,
        observed: None,
    });
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_EXISTS".into()));
}

#[test]
fn long_unicode_names_preserve_boundaries_and_distinguish_shared_prefixes() {
    let prefix = "界".repeat(30);
    let first = stable_name(&(prefix.clone() + "first"), "source.first");
    let second = stable_name(&(prefix + "second"), "source.second");
    assert!(first.len() <= 63);
    assert!(second.len() <= 63);
    assert_ne!(first, second);
    assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
}

#[test]
fn unsigned_bigint_and_time_have_lossless_default_types() {
    let (config, mut source, target) = inputs();
    let mut big = column(&source, "wide", "bigint");
    big.column_type = "bigint unsigned".into();
    let mut duration = column(&source, "duration", "time");
    duration.datetime_precision = Some(6);
    source.tables[0].columns.extend([big, duration]);
    let plan = build(&config, &source, &target).unwrap();
    assert_eq!(plan.tables[0].columns[1].target_type, "numeric(20,0)");
    assert_eq!(plan.tables[0].columns[2].target_type, "interval(6)");
}

#[test]
fn unsupported_cast_does_not_silently_fall_back_to_builtin_type() {
    let (mut config, source, target) = inputs();
    config.cast.push(CastRule {
        source_type: Some("int".into()),
        target_type: Some("uuid".into()),
        ..Default::default()
    });
    assert!(codes(build(&config, &source, &target)).contains(&"COLUMN_TYPE_UNSUPPORTED".into()));
}

#[test]
fn first_matching_cast_wins_and_empty_string_default_survives() {
    let (mut config, mut source, target) = inputs();
    let mut text = column(&source, "value", "varchar");
    text.character_length = Some(20);
    text.default = Some("".into());
    source.tables[0].columns.push(text);
    config.cast = vec![
        CastRule {
            source_type: Some("varchar".into()),
            target_type: Some("text".into()),
            ..Default::default()
        },
        CastRule {
            source_type: Some("varchar".into()),
            target_type: Some("bytea".into()),
            ..Default::default()
        },
    ];
    let plan = build(&config, &source, &target).unwrap();
    assert_eq!(plan.tables[0].columns[1].target_type, "text");
    assert_eq!(
        plan.tables[0].columns[1].default_sql.as_deref(),
        Some("E''")
    );
}

#[test]
fn collation_sensitive_uniqueness_requires_explicit_acknowledgment() {
    let (mut config, mut source, target) = inputs();
    let mut text = column(&source, "label", "varchar");
    text.character_length = Some(50);
    text.collation = Some("utf8mb4_0900_ai_ci".into());
    source.tables[0].columns.push(text);
    source.tables[0].indexes.push(SourceIndex {
        name: "unique_label".into(),
        primary: false,
        unique: true,
        kind: "BTREE".into(),
        parts: vec![IndexPart {
            column: Some("label".into()),
            expression: None,
            prefix_length: None,
            descending: false,
        }],
    });
    assert!(codes(build(&config, &source, &target)).contains(&"COLLATION_SEMANTICS".into()));
    source.tables[0].columns[1].collation = Some("utf8mb4_bin".into());
    assert!(codes(build(&config, &source, &target)).contains(&"COLLATION_SEMANTICS".into()));
    config.overrides.push(ObjectOverride {
        object: "source_fixture.CamelCase.label.collation".into(),
        omit: true,
        target_expression: None,
        target_sql: None,
        materialize: None,
    });
    let plan = build(&config, &source, &target).unwrap();
    assert!(plan.diagnostics.iter().any(|error| error.code == "COLLATION_SEMANTICS" && error.severity == Severity::Warning));
}

#[test]
fn unsupported_structures_and_filtered_fk_parent_are_explicit() {
    let (config, mut source, target) = inputs();
    source.tables[0].foreign_keys.push(SourceForeignKey {
        name: "fk_missing".into(),
        columns: vec!["ID".into()],
        referenced_schema: "source_fixture".into(),
        referenced_table: "missing".into(),
        referenced_columns: vec!["ID".into()],
        on_update: "CASCADE".into(),
        on_delete: "RESTRICT".into(),
    });
    assert!(codes(build(&config, &source, &target)).contains(&"FK_PARENT_MISSING".into()));
    source.tables[0].foreign_keys.clear();
    source.tables[0].columns[0].generation_expression = Some("ID+1".into());
    assert!(codes(build(&config, &source, &target)).contains(&"GENERATED_POLICY".into()));
}

#[test]
fn functional_index_needs_explicit_target_expression_or_omission() {
    let (mut config, mut source, target) = inputs();
    let mut email = column(&source, "email", "varchar");
    email.ordinal = 2;
    email.character_length = Some(100);
    source.tables[0].columns.push(email);
    source.tables[0].indexes.push(SourceIndex {
        name: "email_lower_unique".into(),
        primary: false,
        unique: true,
        kind: "BTREE".into(),
        parts: vec![IndexPart {
            column: None,
            expression: Some("lower(`email`)".into()),
            prefix_length: None,
            descending: false,
        }],
    });
    let object = "source_fixture.CamelCase.email_lower_unique";

    assert!(codes(build(&config, &source, &target)).contains(&"INDEX_EXPRESSION_POLICY".into()));

    config.overrides.push(ObjectOverride {
        object: object.into(),
        omit: false,
        target_expression: Some("lower((email)::text)".into()),
        target_sql: None,
        materialize: None,
    });
    let plan = build(&config, &source, &target).unwrap();
    let index = plan.tables[0]
        .structure
        .indexes
        .iter()
        .find(|index| index.name.contains("email_lower_unique"))
        .unwrap();
    assert!(index.columns.is_empty());
    assert_eq!(index.expressions, [Some("lower((email)::text)".into())]);
    assert!(index.unique);
    assert!(
        plan.ddl
            .iter()
            .any(|step| { step.object == object && step.sql.contains("(lower((email)::text))") })
    );

    config.overrides[0].omit = true;
    config.overrides[0].target_expression = None;
    let plan = build(&config, &source, &target).unwrap();
    assert!(plan.exclusions.iter().any(|excluded| excluded == object));
    assert!(
        !plan.tables[0]
            .structure
            .indexes
            .iter()
            .any(|index| index.name.contains("email_lower_unique"))
    );
}

#[test]
fn unsupported_functional_index_shapes_require_explicit_omission() {
    let object = "source_fixture.CamelCase.unsupported_index";
    let unsupported = [
        SourceIndex {
            name: "unsupported_index".into(),
            primary: false,
            unique: true,
            kind: "BTREE".into(),
            parts: vec![
                IndexPart {
                    column: None,
                    expression: Some("lower(`ID`)".into()),
                    prefix_length: None,
                    descending: false,
                },
                IndexPart {
                    column: None,
                    expression: Some("upper(`ID`)".into()),
                    prefix_length: None,
                    descending: false,
                },
            ],
        },
        SourceIndex {
            name: "unsupported_index".into(),
            primary: false,
            unique: true,
            kind: "BTREE".into(),
            parts: vec![IndexPart {
                column: Some("ID".into()),
                expression: None,
                prefix_length: Some(4),
                descending: false,
            }],
        },
        SourceIndex {
            name: "unsupported_index".into(),
            primary: false,
            unique: false,
            kind: "SPATIAL".into(),
            parts: vec![IndexPart {
                column: Some("ID".into()),
                expression: None,
                prefix_length: None,
                descending: false,
            }],
        },
    ];

    for index in unsupported {
        let (mut config, mut source, target) = inputs();
        source.tables[0].indexes.push(index);
        assert!(codes(build(&config, &source, &target)).contains(&"INDEX_UNSUPPORTED".into()));

        config.overrides.push(ObjectOverride {
            object: object.into(),
            omit: true,
            target_expression: None,
            target_sql: None,
            materialize: None,
        });
        let plan = build(&config, &source, &target).unwrap();
        assert!(plan.exclusions.iter().any(|excluded| excluded == object));
        assert!(
            plan.diagnostics
                .iter()
                .all(|item| item.severity != Severity::Error)
        );
    }
}

#[test]
fn geometry_requires_postgis_and_maps_mysql_spatial_types_to_geometry() {
    let (config, mut source, mut target) = inputs();
    let mut geometry = column(&source, "shape", "geometry");
    geometry.ordinal = 2;
    source.tables[0].columns.push(geometry);

    assert!(codes(build(&config, &source, &target)).contains(&"POSTGIS_REQUIRED".into()));

    target.extensions.push("postgis".into());
    let plan = build(&config, &source, &target).unwrap();
    let shape = plan.tables[0]
        .columns
        .iter()
        .find(|column| column.source_name == "shape")
        .unwrap();
    assert_eq!(shape.kind, ValueKind::Geometry);
    assert_eq!(shape.target_type, "geometry");
    assert!(
        plan.ddl
            .iter()
            .any(|step| step.sql.contains("\"shape\" geometry"))
    );
}

#[test]
fn generated_target_expression_is_explicit_and_omitted_from_copy() {
    let (mut config, source, target) = generated_inputs();
    config.overrides.push(ObjectOverride {
        object: "source_fixture.CamelCase.derived".into(),
        omit: false,
        target_expression: Some("lower(source_text)".into()),
        target_sql: None,
        materialize: None,
    });

    let plan = build(&config, &source, &target).unwrap();
    let derived = plan.tables[0]
        .columns
        .iter()
        .find(|column| column.source_name == "derived")
        .unwrap();
    assert_eq!(
        derived.generated_expression.as_deref(),
        Some("lower(source_text)")
    );
    assert!(!derived.copy);
    let select = crate::mysql::select_sql("source_fixture", &plan.tables[0]).unwrap();
    assert!(
        !select.contains("derived"),
        "PG-generated values are omitted from MySQL projection"
    );
    let create = plan
        .ddl
        .iter()
        .find(|step| step.sql.starts_with("CREATE TABLE"))
        .unwrap();
    assert!(
        create
            .sql
            .contains("\"derived\" varchar(32) GENERATED ALWAYS AS (lower(source_text)) STORED"),
        "{}",
        create.sql
    );
    assert!(
        !create.sql.contains("LOWER(`source_text`)"),
        "MySQL expression is never translated implicitly"
    );
}

#[test]
fn explicit_materialization_keeps_source_generated_value_as_ordinary_copy_column() {
    let (mut config, source, target) = generated_inputs();
    config.overrides.push(ObjectOverride {
        object: "source_fixture.CamelCase.derived".into(),
        omit: false,
        target_expression: None,
        target_sql: None,
        materialize: Some(true),
    });

    let plan = build(&config, &source, &target).unwrap();
    let derived = plan.tables[0]
        .columns
        .iter()
        .find(|column| column.source_name == "derived")
        .unwrap();
    assert_eq!(derived.generated_expression, None);
    assert!(derived.copy);
    let select = crate::mysql::select_sql("source_fixture", &plan.tables[0]).unwrap();
    assert!(
        select.contains("derived"),
        "materialization reads MySQL's generated value"
    );
    let create = plan
        .ddl
        .iter()
        .find(|step| step.sql.starts_with("CREATE TABLE"))
        .unwrap();
    assert!(!create.sql.contains("GENERATED ALWAYS AS"));
}

fn check_inputs(enforced: bool) -> (MigrationConfig, SourceCatalog, TargetCatalog, String) {
    let (config, mut source, target) = inputs();
    source.tables[0].checks.push(SourceCheck {
        name: "amount_nonnegative".into(),
        expression: "`amount` >= 0".into(),
        enforced,
    });
    let object = format!("{}.CamelCase.amount_nonnegative", source.database);
    (config, source, target, object)
}

#[test]
fn check_needs_explicit_target_expression_and_exact_catalog_expectation() {
    let (mut config, source, target, object) = check_inputs(true);
    assert!(codes(build(&config, &source, &target)).contains(&"CHECK_NOT_IMPLEMENTED".into()));

    let expression = "(amount >= 0)";
    config.overrides.push(ObjectOverride {
        object: object.clone(),
        omit: false,
        target_expression: Some(expression.into()),
        target_sql: None,
        materialize: None,
    });
    let plan = build(&config, &source, &target).unwrap();
    let check = &plan.tables[0].structure.checks[0];
    assert_eq!(check.name, "amount_nonnegative");
    assert_eq!(check.expression, expression);
    assert!(check.set_membership.is_none());
    assert!(plan.ddl.iter().any(|step| {
        step.sql
            .contains("ADD CONSTRAINT \"amount_nonnegative\" CHECK ((amount >= 0))")
    }));
    assert!(!plan.exclusions.contains(&object));
}

#[test]
fn check_omission_is_explicit_and_unenforced_checks_cannot_be_strengthened() {
    let (mut config, source, target, object) = check_inputs(true);
    config.overrides.push(ObjectOverride {
        object: object.clone(),
        omit: true,
        target_expression: None,
        target_sql: None,
        materialize: None,
    });
    let plan = build(&config, &source, &target).unwrap();
    assert!(plan.exclusions.contains(&object));
    assert!(plan.tables[0].structure.checks.is_empty());

    let (mut config, source, target, object) = check_inputs(false);
    config.overrides.push(ObjectOverride {
        object: object.clone(),
        omit: false,
        target_expression: Some("(amount >= 0)".into()),
        target_sql: None,
        materialize: None,
    });
    assert!(codes(build(&config, &source, &target)).contains(&"CHECK_ENFORCEMENT_MISMATCH".into()));

    config.overrides[0].omit = true;
    config.overrides[0].target_expression = None;
    let plan = build(&config, &source, &target).unwrap();
    assert!(plan.exclusions.contains(&object));
}

fn fk_inputs() -> (MigrationConfig, SourceCatalog, TargetCatalog) {
    let (mut config, mut source, target) = inputs();
    source.unsupported_objects.clear();
    source.tables[0].columns[0].column_type = "int".into();
    source.tables[0].next_auto_increment = Some(42);
    let mut child = source.tables[0].clone();
    child.name = "Child".into();
    child.next_auto_increment = None;
    child.columns[0].extra.clear();
    let mut parent_id = child.columns[0].clone();
    parent_id.name = "ParentID".into();
    parent_id.ordinal = 2;
    child.columns.push(parent_id);
    child.foreign_keys.push(SourceForeignKey {
        name: "fk_parent".into(),
        columns: vec!["ParentID".into()],
        referenced_schema: source.database.clone(),
        referenced_table: "CamelCase".into(),
        referenced_columns: vec!["ID".into()],
        on_update: "RESTRICT".into(),
        on_delete: "CASCADE".into(),
    });
    source.tables.push(child);
    config.tables.rename.push(TableRename {
        source: "Child".into(),
        target: "renamed_child".into(),
        schema: None,
    });
    (config, source, target)
}

#[test]
fn fk_imported_identity_exclusion_blocks_numeric_child_before_ddl_and_keeps_precedence() {
    let (mut config, source, target) = fk_inputs();
    config.cast.push(CastRule {
        source_type: Some("int".into()),
        auto_increment: Some(false),
        target_type: Some("numeric(20,0)".into()),
        ..Default::default()
    });
    let PlanError::Blocked(errors) = build(&config, &source, &target).unwrap_err() else {
        panic!("FK mismatch must block the complete plan");
    };
    let error = errors
        .iter()
        .find(|d| d.code == "FK_TYPE_INCOMPATIBLE")
        .unwrap();
    assert_eq!(
        error.object.as_deref(),
        Some("source_fixture.Child.fk_parent")
    );
    assert!(
        error
            .message
            .contains("source_fixture.Child.ParentID -> source_fixture.CamelCase.ID")
    );
    assert!(
        error
            .message
            .contains("\"legacy\".\"renamed_child\".\"ParentID\" (numeric(20,0))")
    );
    assert!(
        error
            .message
            .contains("\"legacy\".\"camel_case\".\"ID\" (integer)")
    );
    config.cast.insert(
        0,
        CastRule {
            source_table: Some("Child".into()),
            source_column: Some("ParentID".into()),
            target_type: Some("integer".into()),
            ..Default::default()
        },
    );
    let prepared = build(&config, &source, &target).unwrap();
    assert_eq!(prepared.tables[0].columns[0].target_type, "integer");
    assert!(prepared.tables[0].columns[0].identity);
    assert_eq!(prepared.tables[1].columns[1].target_type, "integer");
    assert!(
        prepared.ddl.iter().any(|s| s
            .sql
            .contains(" FOREIGN KEY (\"ParentID\") REFERENCES \"legacy\".\"camel_case\" (\"ID\")"))
    );
}

#[test]
fn fk_direction_exact_types_and_all_integer_widths_follow_builtin_equality() {
    let (config, source, target) = fk_inputs();
    let prepared = build(&config, &source, &target).unwrap();
    let mut child = prepared.tables[1].columns[1].clone();
    let mut parent = prepared.tables[0].columns[0].clone();
    for child_type in ["smallint", "integer", "bigint"] {
        for parent_type in ["smallint", "integer", "bigint"] {
            child.target_type = child_type.into();
            parent.target_type = parent_type.into();
            assert!(
                fk_type_compatible(&child, &parent),
                "{child_type}->{parent_type}"
            );
        }
    }
    for (child_type, parent_type, expected) in [
        ("numeric(20,0)", "integer", false),
        ("integer", "numeric(20,0)", true),
        ("decimal(18,6)", "numeric(30,12)", true),
        ("int4", "integer", true),
        ("varchar(20)", "character varying(40)", true),
        ("text[]", "text[]", true),
        ("integer[]", "bigint[]", false),
        ("json", "json", false),
        ("public.domain", "public.domain", false),
        ("integer garbage", "integer garbage", false),
    ] {
        child.target_type = child_type.into();
        parent.target_type = parent_type.into();
        assert_eq!(
            fk_type_compatible(&child, &parent),
            expected,
            "{child_type}->{parent_type}"
        );
    }
    child.kind = ValueKind::Enum;
    parent.kind = ValueKind::Enum;
    child.target_type = "\"legacy\".\"shared_state\"".into();
    parent.target_type = child.target_type.clone();
    child.enum_labels = vec!["a".into(), "b".into()];
    parent.enum_labels = child.enum_labels.clone();
    assert!(fk_type_compatible(&child, &parent));
    parent.enum_labels.reverse();
    assert!(!fk_type_compatible(&child, &parent));
    parent.enum_labels = child.enum_labels.clone();
    parent.target_type = "\"legacy\".\"other_state\"".into();
    assert!(!fk_type_compatible(&child, &parent));
    // Prove the reversed direction through full planner resolution, not just the helper.
    let mut source = source;
    source.tables[0].columns[0].extra.clear();
    source.tables[0].next_auto_increment = None;
    let mut config = config;
    config.cast.push(CastRule {
        source_table: Some("CamelCase".into()),
        source_column: Some("ID".into()),
        target_type: Some("numeric(20,0)".into()),
        ..Default::default()
    });
    let prepared = build(&config, &source, &target).unwrap();
    assert_eq!(prepared.tables[0].columns[0].target_type, "numeric(20,0)");
    assert_eq!(prepared.tables[1].columns[1].target_type, "integer");
    assert_eq!(prepared.tables[1].structure.foreign_keys.len(), 1);
}

#[test]
fn fk_composite_renames_filtered_parent_and_explicit_omission_are_preserved() {
    let (mut config, mut source, target) = fk_inputs();
    let mut second = source.tables[0].columns[0].clone();
    second.name = "SecondKey".into();
    second.ordinal = 2;
    second.extra.clear();
    source.tables[0].columns.push(second.clone());
    source.tables[0].indexes[0].parts.push(IndexPart {
        column: Some("SecondKey".into()),
        expression: None,
        prefix_length: None,
        descending: false,
    });
    second.name = "SecondParent".into();
    second.ordinal = 3;
    source.tables[1].columns.push(second);
    source.tables[1].foreign_keys[0]
        .columns
        .push("SecondParent".into());
    source.tables[1].foreign_keys[0]
        .referenced_columns
        .push("SecondKey".into());
    config.migration.identifiers = IdentifierPolicy::SnakeCase;
    let prepared = build(&config, &source, &target).unwrap();
    let fk = &prepared.tables[1].structure.foreign_keys[0];
    assert_eq!(fk.columns, ["parent_id", "second_parent"]);
    assert_eq!(fk.referenced_columns, ["id", "second_key"]);
    config.cast.push(CastRule {
        source_table: Some("Child".into()),
        source_column: Some("SecondParent".into()),
        target_type: Some("numeric(20,0)".into()),
        ..Default::default()
    });
    let PlanError::Blocked(errors) = build(&config, &source, &target).unwrap_err() else {
        panic!("second ordered FK pair must fail");
    };
    let failures: Vec<_> = errors
        .iter()
        .filter(|d| d.code == "FK_TYPE_INCOMPATIBLE")
        .collect();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].message.contains("Child.SecondParent"));
    assert!(
        failures[0]
            .message
            .contains("\"second_parent\" (numeric(20,0))")
    );
    config.tables.exclude.push("CamelCase".into());
    let blocked = codes(build(&config, &source, &target));
    assert!(blocked.contains(&"FK_PARENT_MISSING".into()));
    assert!(!blocked.contains(&"FK_TYPE_INCOMPATIBLE".into()));
    config.overrides.push(ObjectOverride {
        object: "source_fixture.Child.fk_parent".into(),
        omit: true,
        target_expression: None,
        target_sql: None,
        materialize: None,
    });
    let prepared = build(&config, &source, &target).unwrap();
    assert!(prepared.tables[0].structure.foreign_keys.is_empty());
    assert!(
        prepared
            .exclusions
            .contains(&"source_fixture.Child.fk_parent".into())
    );
    assert!(!prepared.ddl.iter().any(|s| s.sql.contains(" FOREIGN KEY ")));
    config.tables.exclude.clear();
    let prepared = build(&config, &source, &target).unwrap();
    assert_eq!(prepared.tables.len(), 2);
    assert!(prepared.tables[1].structure.foreign_keys.is_empty());
    assert!(!prepared.ddl.iter().any(|s| s.sql.contains(" FOREIGN KEY ")));
}

#[test]
fn target_privilege_and_data_only_gates_are_explicit() {
    let (mut config, mut source, mut target) = inputs();
    target.can_create_schema = false;
    assert!(
        codes(build(&config, &source, &target)).contains(&"TARGET_CREATE_SCHEMA_DENIED".into())
    );
    target.schema_exists = true;
    target.can_create_schema = true;
    target.can_use_schema = false;
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_SCHEMA_PRIVILEGE".into()));
    config.migration.mode = MigrationMode::DataOnly;
    config.target.on_existing = ExistingPolicy::Append;
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_SCHEMA_USAGE_DENIED".into()));
    target.can_use_schema = true;
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_TABLE_MISSING".into()));
    target.tables.push(TargetTable {
        schema: "legacy".into(),
        name: "camel_case".into(),
        ordinary_standalone: true,
        columns: vec![TargetColumn {
            name: "ID".into(),
            data_type: "bigint".into(),
            nullable: false,
            generated: false,
            identity: true,
        }],
        row_count: Some(3),
        can_insert: true,
        can_select: true,
        can_alter: true,
        can_truncate: true,
        can_lock: Some(true),
        row_security_active: false,
        observed: None,
    });
    assert!(
        codes(build(&config, &source, &target)).contains(&"TARGET_OBSERVATION_REQUIRED".into())
    );
    source.tables[0].columns[0].extra.clear();
    target.tables[0].columns[0].identity = false;
    config.migration.reset_sequences = false;
    assert!(
        codes(build(&config, &source, &target)).contains(&"TARGET_OBSERVATION_REQUIRED".into())
    );
    target.tables[0].can_insert = false;
    target.tables[0].can_select = false;
    target.tables[0].can_alter = false;
    let errors = codes(build(&config, &source, &target));
    for code in ["TARGET_INSERT_DENIED", "TARGET_SELECT_DENIED"] {
        assert!(errors.contains(&code.into()));
    }
    target.tables[0].can_insert = true;
    target.tables[0].can_select = true;
    config.migration.mode = MigrationMode::Full;
    config.target.on_existing = ExistingPolicy::Recreate;
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_OWNERSHIP_DENIED".into()));
    config.migration.mode = MigrationMode::DataOnly;
    target.tables[0].can_alter = true;
    target.tables[0].can_truncate = false;
    config.target.on_existing = ExistingPolicy::Truncate;
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_TRUNCATE_DENIED".into()));
    target.tables[0].can_truncate = true;
    target.tables[0].row_security_active = true;
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_ROW_SECURITY".into()));
}

#[test]
fn identity_sequence_bounds_use_actual_target_integer_width() {
    let (mut config, mut source, target) = inputs();
    config.cast.push(CastRule {
        source_type: Some("int".into()),
        target_type: Some("smallint".into()),
        ..Default::default()
    });
    source.tables[0].next_auto_increment = Some(32768);
    assert!(codes(build(&config, &source, &target)).contains(&"IDENTITY_RANGE".into()));
    source.tables[0].next_auto_increment = Some(32767);
    let plan = build(&config, &source, &target).unwrap();
    let sequence = &plan.tables[0].structure.sequences[0];
    assert_eq!(
        (sequence.min_value, sequence.max_value, sequence.increment),
        (1, 32767, 1)
    );
    assert_eq!(sequence.next_minimum, 32767);
    assert!(!sequence.cycle);
    config.cast[0].target_type = Some("integer".into());
    source.tables[0].next_auto_increment = Some(2_147_483_648);
    assert!(codes(build(&config, &source, &target)).contains(&"IDENTITY_RANGE".into()));
}

#[test]
fn typed_structure_and_comments_follow_generated_names() {
    let (config, mut source, target) = inputs();
    source.tables[0].comment = "table's comment \\ exact".into();
    source.tables[0].columns[0].comment = "column's comment".into();
    source.tables[0].indexes.clear();
    source.tables[0].indexes.push(SourceIndex {
        name: "PRIMARY".into(),
        primary: true,
        unique: true,
        kind: "BTREE".into(),
        parts: vec![IndexPart {
            column: Some("ID".into()),
            expression: None,
            prefix_length: None,
            descending: false,
        }],
    });
    let plan = build(&config, &source, &target).unwrap();
    let table = &plan.tables[0];
    assert!(table.structure.complete);
    assert_eq!(table.structure.indexes[0].columns, ["ID"]);
    assert!(table.structure.indexes[0].primary);
    assert!(plan.ddl.iter().any(|step| {
        step.sql
            .contains(&quote_identifier(&table.structure.indexes[0].name))
    }));
    assert!(
        plan.ddl.iter().any(|step| step.sql.contains(
            "COMMENT ON COLUMN \"legacy\".\"camel_case\".\"ID\" IS E'column''s comment'"
        ))
    );
    assert_eq!(
        table.structure.comment.as_deref(),
        Some("table's comment \\ exact")
    );
}

#[test]
fn occupied_relations_and_unselected_dependencies_block() {
    let (mut config, source, mut target) = inputs();
    target
        .occupied_names
        .insert("camel_case".into(), "v".into());
    assert!(codes(build(&config, &source, &target)).contains(&"TARGET_RELATION_COLLISION".into()));
    target.occupied_names.clear();
    config.target.on_existing = ExistingPolicy::Recreate;
    target.dependencies.push(TargetDependency {
        kind: "fk".into(),
        name: "unselected_fk".into(),
        dependent_schema: "legacy".into(),
        dependent_table: "outsider".into(),
        referenced_schema: "legacy".into(),
        referenced_table: "camel_case".into(),
    });
    // Legacy name-only edges cannot manufacture an existing target or mutation root.
    build(&config, &source, &target).unwrap();
    target.dependencies[0].dependent_table = "camel_case".into();
    build(&config, &source, &target).unwrap();
}

#[test]
fn source_aliases_narrowed_defaults_and_transforms_are_checked() {
    let (mut config, mut source, target) = inputs();
    source.tables[0].columns[0].extra.clear();
    source.tables[0].columns[0].default = Some("32768".into());
    config.cast.push(CastRule {
        source_type: Some("INTEGER".into()),
        target_type: Some("SMALLINT".into()),
        ..Default::default()
    });
    assert!(codes(build(&config, &source, &target)).contains(&"DEFAULT_UNSUPPORTED".into()));
    source.tables[0].columns[0].default = None;
    config.cast[0].transform = Some("tinyint-to-boolean".into());
    assert!(codes(build(&config, &source, &target)).contains(&"TRANSFORM_TARGET_MISMATCH".into()));
}

#[test]
fn dropping_temporal_typemod_preserves_time_zone_semantics() {
    let (mut config, mut source, target) = inputs();
    let mut timestamp = column(&source, "instant", "timestamp");
    timestamp.datetime_precision = Some(6);
    source.tables[0].columns.push(timestamp);
    config.cast.push(CastRule {
        source_type: Some("timestamp".into()),
        target_type: Some("timestamp(6) with time zone".into()),
        drop_typemod: true,
        ..Default::default()
    });
    let resolved = build(&config, &source, &target).unwrap();
    assert_eq!(
        resolved.tables[0].columns[1].target_type,
        "timestamp with time zone"
    );
    assert_eq!(resolved.tables[0].columns[1].kind, ValueKind::Timestamp);
}

#[test]
fn unselected_storage_and_index_features_do_not_block_selected_tables() {
    let (mut config, mut source, target) = inputs();
    let mut unselected = source.tables[0].clone();
    unselected.name = "out_of_scope".into();
    source.tables.push(unselected);
    source.unsupported_objects.push(UnsupportedObject {
        object: "out_of_scope.hidden_index".into(),
        kind: "invisible_index".into(),
        reason: "requires explicit decision if selected".into(),
    });
    config.tables.include = vec!["CamelCase".into()];
    assert_eq!(build(&config, &source, &target).unwrap().tables.len(), 1);
    config.tables.include.push("out_of_scope".into());
    assert!(codes(build(&config, &source, &target)).contains(&"SOURCE_UNSUPPORTED_OBJECT".into()));
}
