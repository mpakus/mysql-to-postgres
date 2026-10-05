use my2pg::config::import::parse;
use my2pg::config::{self, import::*, *};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

const HEAD: &str = "LOAD DATABASE FROM mysql://reader:source-password@localhost/shop INTO postgresql://writer:target-password@localhost/warehouse ";
fn reviewed() -> ImportOptions {
    ImportOptions {
        target_schema: Some("legacy".into()),
        default_policy: DefaultPolicy::My2pgReviewed,
        ..Default::default()
    }
}
fn command(clause: &str) -> String {
    format!("{HEAD}{clause};")
}
fn cfg(clause: &str) -> ImportedConfig {
    parse(&command(clause), &reviewed()).unwrap()
}
fn assert_error(text: &str, code: &str) {
    let failure = parse(text, &reviewed()).unwrap_err();
    assert_eq!(failure.code, code, "{failure}");
    let display = format!(
        "{failure} {failure:?} {}",
        serde_json::to_string(&failure).unwrap()
    );
    assert!(!display.contains("source-password") && !display.contains("target-password"));
    assert!(failure.span.start <= failure.span.end && failure.span.end <= text.len());
}

#[test]
fn default_semantics_need_explicit_review_and_schema_never_defaults_to_public() {
    let options = ImportOptions {
        target_schema: Some("legacy".into()),
        ..Default::default()
    };
    assert_eq!(
        parse(&command(""), &options).unwrap_err().code,
        "IMPORT_DEFAULT_POLICY"
    );
    let options = ImportOptions {
        default_policy: DefaultPolicy::My2pgReviewed,
        ..Default::default()
    };
    assert_eq!(
        parse(&command(""), &options).unwrap_err().code,
        "IMPORT_SCHEMA_REQUIRED"
    );
    let result = parse(&command("ALTER SCHEMA 'shop' RENAME TO 'legacy'"), &options).unwrap();
    assert_eq!(result.config.target.schema, "legacy");
    assert_eq!(result.config.source.consistency, Consistency::Frozen);
    assert_eq!(
        result.compatibility.default_policy,
        DefaultPolicy::My2pgReviewed
    );
    assert!(
        result
            .compatibility
            .warnings
            .iter()
            .any(|s| s.contains("display-width"))
    );
    let reparsed = config::parse(&result.toml, Path::new("/tmp")).unwrap();
    assert_eq!(reparsed.target.schema, "legacy");
    let output = format!(
        "{} {:?} {}",
        result.toml,
        result,
        serde_json::to_string(&result.compatibility).unwrap()
    );
    assert!(
        !output.contains("source-password")
            && !output.contains("target-password")
            && !output.contains("mysql://")
    );
}

#[test]
fn modes_existing_policies_limits_and_identifiers_normalize() {
    let result = cfg(
        "WITH include no drop, no truncate, create tables, create indexes, foreign keys, reset sequences, quote identifiers, workers=1, batch rows=7, batch size=32 MB, max parallel create index=2",
    );
    assert_eq!(result.config.target.on_existing, ExistingPolicy::Error);
    assert_eq!(result.config.migration.mode, MigrationMode::Full);
    assert_eq!(result.config.migration.batch_rows, 7);
    assert_eq!(result.config.migration.batch_bytes, 32 * 1024 * 1024);
    assert_eq!(result.config.migration.index_workers, 2);
    assert_eq!(
        result.config.migration.identifiers,
        IdentifierPolicy::Preserve
    );
    assert_eq!(
        cfg("WITH include drop, no truncate")
            .config
            .target
            .on_existing,
        ExistingPolicy::Recreate
    );
    assert_eq!(
        cfg("WITH truncate").config.target.on_existing,
        ExistingPolicy::Truncate
    );
    assert_eq!(
        cfg("WITH schema only").config.migration.mode,
        MigrationMode::SchemaOnly
    );
    assert_error(
        &command("WITH create no tables, downcase identifiers, reset no sequences"),
        "IMPORT_CONFLICT",
    );
    assert_eq!(
        parse(
            &command("WITH create no tables, downcase identifiers, reset no sequences"),
            &ImportOptions {
                append_data_only: true,
                ..reviewed()
            }
        )
        .unwrap()
        .config
        .migration
        .mode,
        MigrationMode::DataOnly
    );
    assert_eq!(
        cfg("WITH snake_case identifiers")
            .config
            .migration
            .identifiers,
        IdentifierPolicy::SnakeCase
    );
    let append = ImportOptions {
        append_data_only: true,
        ..reviewed()
    };
    assert_eq!(
        parse(
            &command("WITH data only, no truncate, include no drop"),
            &append
        )
        .unwrap()
        .config
        .target
        .on_existing,
        ExistingPolicy::Append
    );
    for clause in [
        "WITH include drop, truncate",
        "WITH truncate, include drop",
        "WITH data only, schema only",
        "WITH batch rows=0",
        "WITH batch size=18446744073709551615 GB",
    ] {
        assert!(parse(&command(clause), &reviewed()).is_err(), "{clause}");
    }
    assert_error(&command("WITH batch size=1 MB"), "IMPORT_CONFIG");
    for clause in [
        "WITH data only, create tables",
        "WITH create tables, data only",
    ] {
        assert_error(&command(clause), "IMPORT_CONFLICT");
    }
}

#[test]
fn clauses_are_order_independent_and_comments_preserve_quoted_data() {
    let text = format!(
        "/* outer /* nested */ done */ {HEAD}\n-- line\nCAST type datetime to timestamp drop default drop not null using zero-dates-to-null\nINCLUDING ONLY TABLE NAMES MATCHING 'semi;--name', ~/^orders_[0-9]+$/\nWITH single reader per thread, concurrency=1\nEXCLUDING TABLE NAMES MATCHING 'ignore'\nALTER TABLE NAMES MATCHING 'orders' RENAME TO 'sales'\nALTER TABLE NAMES MATCHING 'orders' SET SCHEMA 'legacy'; -- tail"
    );
    let result = parse(&text, &reviewed()).unwrap();
    assert_eq!(
        result.config.cast[0].target_type.as_deref(),
        Some("timestamp")
    );
    assert!(result.config.cast[0].drop_default && result.config.cast[0].drop_not_null);
    assert_eq!(
        result.config.cast[0].transform.as_deref(),
        Some("zero-dates-to-null")
    );
    assert_eq!(result.config.tables.include, ["semi;--name"]);
    assert_eq!(result.config.tables.include_regex, ["^orders_[0-9]+$"]);
    assert_eq!(result.config.tables.rename[0].source, "orders");
    assert_eq!(result.config.tables.rename[0].target, "sales");
    assert_eq!(
        result.config.tables.rename[0].schema.as_deref(),
        Some("legacy")
    );
    assert_eq!(result.compatibility.mapped.len(), 8);
    for entry in &result.compatibility.mapped {
        assert!(text.is_char_boundary(entry.span.start));
    }
}

#[test]
fn casts_preserve_order_typed_guards_and_finite_or_expansion() {
    let result = cfg(
        "CAST type decimal when (or (and (= 18 precision) (= 6 scale)) (> precision 20)) to \"double precision\" drop typemod, column \"orders\".\"state\" using empty-string-to-null, type int when unsigned with extra auto_increment to bigint keep default",
    );
    assert_eq!(result.config.cast.len(), 4);
    assert_eq!(result.config.cast[0].precision.as_ref().unwrap().value, 18);
    assert!(matches!(
        result.config.cast[0].precision.as_ref().unwrap().op,
        Comparison::Eq
    ));
    assert_eq!(result.config.cast[0].scale.as_ref().unwrap().value, 6);
    assert!(matches!(
        result.config.cast[1].precision.as_ref().unwrap().op,
        Comparison::Gt
    ));
    assert_eq!(
        result.config.cast[2].source_table.as_deref(),
        Some("orders")
    );
    assert_eq!(result.config.cast[2].not_null, None);
    assert_eq!(result.config.cast[3].unsigned, Some(true));
    assert_eq!(result.config.cast[3].auto_increment, Some(true));
    for guard in [
        "(eval 1)",
        "(= precision scale)",
        "(= precision -1)",
        "(= precision 66)",
        "(and (> precision 1) (< precision 20))",
        "(or (= precision 1))",
    ] {
        assert!(
            parse(
                &command(&format!("CAST type decimal when {guard} to numeric")),
                &reviewed()
            )
            .is_err(),
            "{guard}"
        );
    }
}

#[test]
fn numeric_guards_preserve_decimal_semantics_and_refuse_display_width_or_unknown_column_types() {
    for source in ["decimal", "numeric"] {
        let result = cfg(&format!(
            "CAST type {source} when (and (= 18 precision) (= 6 scale)) to numeric(18,6)"
        ));
        assert_eq!(result.config.cast[0].precision.as_ref().unwrap().value, 18);
        assert_eq!(result.config.cast[0].scale.as_ref().unwrap().value, 6);
    }
    for source in [
        "type tinyint",
        "type int",
        "type char",
        "type bit",
        "column orders.value",
    ] {
        let input = command(&format!(
            "CAST {source} when (= precision 1) to boolean using tinyint-to-boolean"
        ));
        let error = parse(&input, &reviewed()).unwrap_err();
        assert_eq!(error.code, "IMPORT_GUARD", "{source}");
        assert_eq!(error.span.start, input.find(source).unwrap());
        assert!(error.message.contains("decimal/numeric"));
    }
}

#[test]
fn imported_type_casts_preserve_auto_increment_exclusion_and_column_guard_refusal() {
    use my2pg::{
        model::{SourceCatalog, TargetCatalog},
        plan,
    };
    let mut source: SourceCatalog =
        serde_json::from_str(include_str!("contracts/source-catalog.json")).unwrap();
    source.unsupported_objects.clear();
    let mut plain = source.tables[0].columns[0].clone();
    plain.name = "plain_id".into();
    plain.ordinal = 2;
    plain.extra.clear();
    source.tables[0].columns.push(plain);
    let target = TargetCatalog {
        server_version: "16.15".into(),
        can_create_schema: true,
        can_use_schema: true,
        can_create_objects: true,
        ..Default::default()
    };
    let imported = cfg("CAST type int to numeric(20,0) WITH workers=1");
    assert_eq!(imported.config.cast[0].auto_increment, Some(false));
    let plan = plan::build(&imported.config, &source, &target).unwrap();
    assert_eq!(plan.tables[0].columns[0].target_type, "bigint");
    assert!(plan.tables[0].columns[0].identity);
    assert_eq!(plan.tables[0].columns[1].target_type, "numeric(20,0)");
    let imported = cfg("CAST type int with extra auto_increment to bigint");
    assert_eq!(imported.config.cast[0].auto_increment, Some(true));
    plan::build(&imported.config, &source, &target).unwrap();
    for guard in [
        "when default 'zero'",
        "when signed",
        "when unsigned",
        "when not null",
        "and not null",
        "with extra auto_increment",
    ] {
        assert_error(
            &command(&format!("CAST column CamelCase.ID {guard} to bigint")),
            "IMPORT_GUARD",
        );
    }
    let column = cfg("CAST column CamelCase.ID drop default drop not null");
    assert!(column.config.cast[0].drop_default && column.config.cast[0].drop_not_null);
    assert!(
        column
            .compatibility
            .warnings
            .iter()
            .any(|warning| warning.contains("case-sensitive"))
    );
}

#[test]
fn every_supported_clause_can_follow_a_cast_without_becoming_an_extra_or_timezone_guard() {
    for clause in [
        "WITH workers=1",
        "SET timezone to 'UTC'",
        "INCLUDING ONLY TABLE NAMES MATCHING 'orders'",
        "EXCLUDING TABLE NAMES MATCHING 'Other'",
        "ALTER SCHEMA 'shop' RENAME TO 'legacy'",
        "ALTER TABLE NAMES MATCHING 'orders' RENAME TO 'sales'",
    ] {
        for target in [
            "timestamp",
            "timestamp with time zone",
            "timestamp without time zone",
        ] {
            cfg(&format!("CAST type datetime to {target} {clause}"));
        }
    }
}

#[test]
fn source_schemes_urls_percent_decoding_and_ipv6_share_the_offline_contract() {
    for source in [
        "sqlite:///a",
        "mssql://reader@localhost/shop",
        "postgresql://reader@localhost/shop",
        "jdbc:mysql://reader@localhost/shop",
        "mysql://localhost/shop",
        "mysql://reader@unix:/tmp/mysql.sock/shop",
        "mysql://reader@localhost/shop?useSSL=false",
        "mysql://reader:bad%00pass@localhost/shop",
        "mysql://reader:bad%ZZpass@localhost/shop",
        "mysql://user:escaped@@password@localhost/shop",
    ] {
        let text =
            format!("LOAD DATABASE FROM {source} INTO postgresql://writer@localhost/warehouse;");
        assert_error(&text, "IMPORT_URI");
    }
    let text = "LOAD DATABASE FROM 'mysql://u%3Aser:p%40ss%253Aword@[::1]:3306/sh%6Fp' INTO pgsql://writer@[::1]:5432/warehouse ALTER SCHEMA 'shop' RENAME TO 'legacy';";
    let result = parse(text, &reviewed()).unwrap();
    assert_eq!(
        result.config.target.url_env.as_deref(),
        Some("MY2PG_TARGET_URL")
    );
    assert!(!result.toml.contains("ss%253Aword"));
    for target in [
        "postgresql:///warehouse",
        "postgresql://writer@/warehouse",
        "postgresql://writer@%2Ftmp/warehouse",
        "postgresql://writer@localhost,127.0.0.1/warehouse",
        "jdbc:postgresql://writer@localhost/warehouse",
    ] {
        assert_error(
            &format!("LOAD DATABASE FROM mysql://reader@localhost/shop INTO {target};"),
            "IMPORT_URI",
        );
    }
}

#[test]
fn unsupported_clauses_and_read_forms_fail_entire_import_with_exact_spans() {
    for (clause, code) in [
        ("DISTRIBUTE orders USING id", "IMPORT_CLAUSE"),
        ("BEFORE LOAD DO $$ SELECT 1 $$", "IMPORT_READFORM"),
        ("MATERIALIZE ALL VIEWS", "IMPORT_CAPABILITY"),
        (
            "DECODING TABLE NAMES MATCHING ~/./ AS utf8",
            "IMPORT_CAPABILITY",
        ),
        ("WITH disable triggers", "IMPORT_OPTION"),
        ("WITH drop schema", "IMPORT_OPTION"),
        ("WITH create no indexes", "IMPORT_OPTION"),
        ("WITH no foreign keys", "IMPORT_OPTION"),
        ("WITH preserve index names", "IMPORT_OPTION"),
        ("WITH prefetch rows=100", "IMPORT_UNIT_POLICY"),
        ("WITH batch concurrency=4", "IMPORT_UNIT_POLICY"),
        ("WITH multiple readers per thread", "IMPORT_READER_POLICY"),
        ("WITH concurrency=2", "IMPORT_READER_POLICY"),
        (
            "CAST type text using arbitrary-transform",
            "IMPORT_TRANSFORM",
        ),
        ("CAST type text using (lambda (x) x)", "IMPORT_TRANSFORM"),
        (
            "CAST type timestamp with extra on update current timestamp to timestamptz",
            "IMPORT_SYNTAX",
        ),
        (
            "CAST type text using #.(run-program \"touch\" \"/tmp/must-not-exist\")",
            "IMPORT_READFORM",
        ),
    ] {
        assert_error(&command(clause), code);
    }
    let text = format!("{HEAD}\n  WITH disable triggers;");
    let failure = parse(&text, &reviewed()).unwrap_err();
    assert_eq!(failure.span.start, text.find("disable").unwrap());
    assert_eq!(failure.span.line, 2);
    assert_eq!(failure.span.column, 8);
    assert_error(
        &format!(
            "{} LOAD DATABASE FROM mysql://reader@localhost/a INTO postgresql://writer@localhost/b;",
            command("")
        ),
        "IMPORT_COMMANDS",
    );
    assert_error(
        &command("INCLUDING ONLY TABLE NAMES MATCHING ~/\\w+/"),
        "IMPORT_REGEX",
    );
}

#[test]
fn settings_and_types_use_existing_strict_validation() {
    let result = cfg(
        "SET PostgreSQL PARAMETERS maintenance_work_mem to '128MB', timezone to 'UTC' SET MySQL PARAMETERS net_read_timeout='120', time_zone='+00:00' CAST type timestamp to timestamp without time zone",
    );
    assert_eq!(result.config.source.session["net_read_timeout"], "120");
    assert_eq!(result.config.target.session["timezone"], "UTC");
    assert_eq!(
        result.config.cast[0].target_type.as_deref(),
        Some("timestamp without time zone")
    );
    for clause in [
        "SET search_path to 'legacy,public'",
        "SET MySQL PARAMETERS net_read_timeout='oops'",
        "CAST type unknown to text",
        "CAST type text to serial",
        "CAST type text to \"text; DROP SCHEMA public\"",
    ] {
        assert_error(&command(clause), "IMPORT_CONFIG");
    }
}

#[test]
fn publishing_is_private_atomic_and_never_overwrites_existing_or_symlink_outputs() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "my2pg-import-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let output = root.join("migration.toml");
    let result = cfg("");
    publish_new(&result, &output).unwrap();
    assert_eq!(fs::read_to_string(&output).unwrap(), result.toml);
    assert!(publish_new(&cfg("WITH schema only"), &output).is_err());
    assert_eq!(fs::read_to_string(&output).unwrap(), result.toml);
    assert!(publish_new(&result, &root.join("missing/target.toml")).is_err());
    assert_eq!(
        fs::read_dir(&root).unwrap().count(),
        1,
        "failed staging leaves no partial artifacts"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o077,
            0
        );
        let link = root.join("link.toml");
        symlink(&output, &link).unwrap();
        assert!(publish_new(&result, &link).is_err());
        assert_eq!(fs::read_to_string(&output).unwrap(), result.toml);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_and_adversarial_input_is_bounded_and_never_executes() {
    for input in [
        "",
        ";",
        "LOAD CSV FROM '/tmp/a' INTO postgresql://writer@localhost/a;",
        "LOAD DATABASE FROM 'unterminated",
        "/* unterminated",
    ] {
        assert!(parse(input, &reviewed()).is_err());
    }
    assert_error(
        &command("CAST type decimal when (= precision #.(run-command)) to numeric"),
        "IMPORT_READFORM",
    );
    assert_error(
        &format!("{HEAD}WITH workers={};", "9".repeat(1000)),
        "IMPORT_NUMBER",
    );
    let nested = format!(
        "{}{}{}",
        "(and ".repeat(20),
        "(= precision 1)",
        " (= scale 1))".repeat(20)
    );
    assert_error(
        &command(&format!("CAST type decimal when {nested} to numeric")),
        "IMPORT_DEPTH",
    );
    assert_eq!(
        parse(&"a".repeat(1024 * 1024 + 1), &reviewed())
            .unwrap_err()
            .code,
        "IMPORT_INPUT"
    );
    for ch in ['\'', '"', ';', '#', '$', '\0', '😀'] {
        let text = format!("{HEAD}{ch};");
        assert!(parse(&text, &reviewed()).is_err());
    }
}

#[test]
fn every_selected_upstream_load_is_classified_without_execution_or_secret_output() {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/upstream-manifest.json")).unwrap();
    let fixtures = manifest["load_fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 13);
    let reader_rejections = [
        "clojure/tests/mysql-unit-full/mysql-unit-full.load",
        "clojure/tests/mysql/sakila/sakila.load",
        "test/mysql-collision.load",
        "test/mysql/db789.load",
        "test/mysql/f1db-data.load",
        "test/mysql/my.load",
        "test/sakila.load",
    ];
    for fixture in fixtures {
        let path = fixture["path"].as_str().unwrap();
        let input = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/upstream")
                .join(path),
        )
        .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(input.as_bytes())),
            fixture["sha256"]
        );
        let error = parse(&input, &reviewed()).unwrap_err();
        let expected = if reader_rejections.contains(&path) {
            "IMPORT_READFORM"
        } else {
            "IMPORT_URI"
        };
        assert_eq!(error.code, expected, "{path}: {error}");
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains("rootpassword") && !serialized.contains("://"));
    }
}

#[test]
fn all_eighteen_selected_parser_behaviors_have_independent_strict_expectations() {
    let cases = [
        ("test-parse-mysql-database", "", None),
        (
            "test-parse-mysql-with-cast",
            "CAST type tinyint to smallint drop typemod",
            None,
        ),
        (
            "test-parse-cast-when-default-and-not-null",
            "CAST type datetime when default '0000-00-00 00:00:00' and not null to timestamp drop default drop not null using zero-dates-to-null",
            None,
        ),
        (
            "test-distribute-reference-parse",
            "DISTRIBUTE shop.orders AS REFERENCE TABLE",
            Some("IMPORT_CLAUSE"),
        ),
        (
            "test-distribute-using-parse",
            "DISTRIBUTE shop.orders USING id",
            Some("IMPORT_CLAUSE"),
        ),
        (
            "test-distribute-multiple-rules",
            "DISTRIBUTE shop.orders USING id DISTRIBUTE shop.users AS REFERENCE TABLE",
            Some("IMPORT_CLAUSE"),
        ),
        (
            "test-no-distribute-rules-default",
            "WITH reset sequences",
            None,
        ),
        (
            "test-decoding-as-regex-parse",
            "DECODING TABLE NAMES MATCHING ~/notes/ AS utf8",
            Some("IMPORT_CAPABILITY"),
        ),
        (
            "test-decoding-as-multiple-patterns",
            "DECODING TABLE NAMES MATCHING 'notes', ~/legacy/ AS utf8",
            Some("IMPORT_CAPABILITY"),
        ),
        (
            "test-with-drop-schema",
            "WITH drop schema",
            Some("IMPORT_OPTION"),
        ),
        ("test-with-reindex", "WITH reindex", Some("IMPORT_OPTION")),
        (
            "test-with-preserve-index-names",
            "WITH preserve index names",
            Some("IMPORT_OPTION"),
        ),
        (
            "test-cast-column-no-type",
            "CAST column orders.notes drop not null using empty-string-to-null",
            None,
        ),
        (
            "test-cast-target-type-multi-word",
            "CAST type timestamp to timestamp with time zone",
            None,
        ),
        (
            "test-cast-rule-when-not-null",
            "CAST type datetime when not null to timestamp",
            None,
        ),
        ("test-parse-mysql-uri", "", None),
        ("test-parse-jdbc-mysql-uri", "", Some("IMPORT_URI")),
        ("test-jdbc-urls-accepted-by-drivers", "", Some("IMPORT_URI")),
    ];
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/upstream-manifest.json")).unwrap();
    let selected: std::collections::BTreeSet<_> = manifest["parser_cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(
        selected,
        cases.iter().map(|(symbol, _, _)| *symbol).collect()
    );
    for (symbol, clause, expected) in cases {
        let input = if symbol.contains("jdbc") {
            format!(
                "LOAD DATABASE FROM jdbc:mysql://reader@localhost/shop INTO postgresql://writer@localhost/warehouse {clause};"
            )
        } else {
            command(clause)
        };
        match (parse(&input, &reviewed()), expected) {
            (Ok(imported), None) => {
                config::parse(&imported.toml, Path::new("/tmp")).unwrap();
            }
            (Err(error), Some(code)) => assert_eq!(error.code, code, "{symbol}: {error}"),
            (actual, expected) => panic!("{symbol}: expected {expected:?}, got {actual:?}"),
        }
    }
}

#[test]
fn documented_positive_example_reparses_and_preserves_effective_options() {
    let result = parse(
        include_str!("../docs/examples/pgloader-subset.load"),
        &reviewed(),
    )
    .unwrap();
    assert_eq!(result.config.migration.table_workers, 4);
    assert_eq!(result.config.migration.batch_rows, 25000);
    assert_eq!(result.config.target.schema, "legacy");
    assert_eq!(
        result.config.cast[0].source_type.as_deref(),
        Some("datetime")
    );
}

#[test]
fn emitted_filters_rename_and_cast_drive_the_actual_planner_and_encoder() {
    use my2pg::{
        convert,
        model::{RawValue, SourceCatalog, TargetCatalog},
        plan,
    };
    let mut source: SourceCatalog =
        serde_json::from_str(include_str!("contracts/source-catalog.json")).unwrap();
    source.tables[0].columns[0].extra.clear();
    source.tables[0].next_auto_increment = None;
    source.unsupported_objects.clear();
    let mut omitted = source.tables[0].clone();
    omitted.name = "Other".into();
    source.tables.push(omitted);
    let imported = cfg(
        "INCLUDING ONLY TABLE NAMES MATCHING ~/^Camel/ EXCLUDING TABLE NAMES MATCHING 'Other' ALTER TABLE NAMES MATCHING 'CamelCase' RENAME TO 'Imported' CAST type int when unsigned to numeric(20,0)",
    );
    let target = TargetCatalog {
        server_version: "16.15".into(),
        can_create_schema: true,
        can_use_schema: true,
        can_create_objects: true,
        ..Default::default()
    };
    let planned = plan::build(&imported.config, &source, &target).unwrap();
    assert_eq!(planned.tables.len(), 1);
    assert_eq!(planned.tables[0].target_name, "Imported");
    assert_eq!(planned.tables[0].columns[0].target_type, "numeric(20,0)");
    assert_eq!(
        convert::encode_row(&planned.tables[0], &[RawValue::UInt(4294967295)]).unwrap(),
        b"4294967295\n"
    );
    assert_eq!(planned.exclusions.len(), 1);
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "my2pg-import-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn cli(arguments: &[&std::ffi::OsStr]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .args(arguments)
        .env_remove("MY2PG_SOURCE_URL")
        .env_remove("MY2PG_TARGET_URL")
        .output()
        .unwrap()
}

#[test]
fn both_actual_cli_aliases_publish_the_same_config_and_explicit_policy_report() {
    use std::ffi::OsStr;
    let directory = TempDir::new();
    let input = directory.0.join("source.load");
    fs::write(&input, command("WITH data only, no truncate")).unwrap();
    let mut artifacts = Vec::new();
    for command in [vec!["import-load"], vec!["config", "import"]] {
        let output = directory.0.join(format!("{}.toml", artifacts.len()));
        let mut args: Vec<&OsStr> = command.iter().map(OsStr::new).collect();
        args.extend([
            input.as_os_str(),
            OsStr::new("--output"),
            output.as_os_str(),
            OsStr::new("--target-schema"),
            OsStr::new("legacy"),
            OsStr::new("--use-my2pg-defaults"),
            OsStr::new("--append-data-only"),
            OsStr::new("--consistency"),
            OsStr::new("single_snapshot"),
            OsStr::new("--source-env"),
            OsStr::new("IMPORT_SOURCE"),
            OsStr::new("--target-env"),
            OsStr::new("IMPORT_TARGET"),
        ]);
        let result = cli(&args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["default_policy"], "my2pg_reviewed");
        assert!(result.stderr.is_empty());
        let content = fs::read_to_string(&output).unwrap();
        assert!(!content.contains("source-password") && !content.contains("target-password"));
        let config = config::parse(&content, &directory.0).unwrap();
        assert_eq!(config.target.on_existing, ExistingPolicy::Append);
        assert_eq!(config.source.consistency, Consistency::SingleSnapshot);
        assert_eq!(config.source.url_env.as_deref(), Some("IMPORT_SOURCE"));
        let checked = std::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .args(["check"])
            .arg(&output)
            .env("IMPORT_SOURCE", "mysql://reader@127.0.0.1:1/shop")
            .env("IMPORT_TARGET", "postgresql://writer@127.0.0.1:1/warehouse")
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
        artifacts.push(content);
    }
    assert_eq!(artifacts[0], artifacts[1]);
}

#[test]
fn actual_cli_failures_are_atomic_sanitized_and_distinguish_input_from_publication() {
    use std::ffi::OsStr;
    let directory = TempDir::new();
    let input = directory.0.join("source.load");
    let output = directory.0.join("output.toml");
    fs::write(&input, command("")).unwrap();
    let common = [
        OsStr::new("import-load"),
        input.as_os_str(),
        OsStr::new("--output"),
        output.as_os_str(),
        OsStr::new("--target-schema"),
        OsStr::new("legacy"),
    ];
    let failure = cli(&common);
    assert_eq!(failure.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&failure.stderr).contains("IMPORT_DEFAULT_POLICY"));
    assert!(!output.exists());
    let mut explicit = common.to_vec();
    explicit.push(OsStr::new("--use-my2pg-defaults"));
    explicit.push(OsStr::new("--quiet"));
    let success = cli(&explicit);
    assert!(success.status.success());
    assert!(success.stdout.is_empty() && success.stderr.is_empty());
    let previous = fs::read(&output).unwrap();
    let conflict = cli(&explicit);
    assert_eq!(conflict.status.code(), Some(1));
    assert_eq!(fs::read(&output).unwrap(), previous);
    for clause in [
        "WITH disable triggers",
        "CAST type text using #.(run-command)",
    ] {
        fs::write(&input, command(clause)).unwrap();
        let failure = cli(&explicit);
        assert_eq!(failure.status.code(), Some(2));
        assert_eq!(fs::read(&output).unwrap(), previous);
        let diagnostics = String::from_utf8_lossy(&failure.stderr);
        assert!(
            !diagnostics.contains("source-password") && !diagnostics.contains("target-password")
        );
        assert!(failure.stdout.is_empty());
    }
    for bytes in [vec![b'x'; 1024 * 1024 + 1], vec![0xff]] {
        fs::write(&input, bytes).unwrap();
        assert_eq!(cli(&explicit).status.code(), Some(2));
        assert_eq!(fs::read(&output).unwrap(), previous);
    }
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
}
