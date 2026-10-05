use my2pg::{config::MigrationConfig, mysql, plan, postgres};
use mysql_async::prelude::Queryable;

type DefaultMetadata = (String, Option<String>, Option<String>, Option<String>);

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4 TLS fixture"]
async fn native_binary_defaults_recover_actual_bytes_without_source_rows_or_writes() {
    let mut config: MigrationConfig =
        toml::from_str(include_str!("../contracts/config.toml")).unwrap();
    config.source.ca_file = Some(required("MY2PG_TLS_CA").into());
    config.target.ca_file = config.source.ca_file.clone();
    config.target.schema = "t10_binary_default".into();
    config.tables.rename.clear();
    config.tables.include = vec!["t10_binary_default".into()];
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    source
        .query_drop(
            r#"CREATE TABLE source.t10_binary_default (
        id integer PRIMARY KEY,
        lead_bytes binary(5) DEFAULT 0x00095c7fff,
        interior binary(5) DEFAULT 0x6100627fff,
        padded binary(5) DEFAULT 0x61,
        varied varbinary(5) DEFAULT 0x00095c7fff,
        empty_bytes varbinary(5) DEFAULT '',
        nullable_bytes varbinary(5) DEFAULT NULL,
        absent varbinary(5) NOT NULL,
        emoji_value varchar(100) CHARACTER SET utf8mb4 DEFAULT '{"face":"😀"}',
        char_value char(5) CHARACTER SET utf8mb4 DEFAULT '😀x',
        enum_value enum('😀','plain') CHARACTER SET utf8mb4 DEFAULT '😀',
        set_value set('😀','plain') CHARACTER SET utf8mb4 DEFAULT '😀,plain',
        latin_value varchar(2) CHARACTER SET latin1 DEFAULT '€ÿ'
    ) ENGINE=InnoDB"#,
        )
        .await
        .unwrap();
    source
        .query_drop(
            r#"CREATE TABLE source.t10_binary_default_guard (
        id integer PRIMARY KEY,
        `odd``name` binary(3) DEFAULT 0x000961,
        expression_value binary(16) DEFAULT (UUID_TO_BIN(UUID())),
        generated_value varbinary(3) GENERATED ALWAYS AS (0x616263) STORED
    ) ENGINE=InnoDB"#,
        )
        .await
        .unwrap();
    let metadata: Vec<DefaultMetadata> = source.query(
        "SELECT COLUMN_NAME,COLUMN_DEFAULT,HEX(COLUMN_DEFAULT),HEX(CAST(COLUMN_DEFAULT AS BINARY)) FROM information_schema.columns WHERE TABLE_SCHEMA='source' AND TABLE_NAME='t10_binary_default' ORDER BY ORDINAL_POSITION"
    ).await.unwrap();
    for row in &metadata {
        println!("synthetic binary metadata: {row:?}");
    }
    let lead_bytes = metadata.iter().find(|r| r.0 == "lead_bytes").unwrap();
    assert_eq!(lead_bytes.1.as_deref(), Some("0x"));
    assert_eq!(lead_bytes.2.as_deref(), Some("3078"));
    assert_eq!(
        lead_bytes.3, lead_bytes.2,
        "a cast cannot recover already lost bytes"
    );
    let interior = metadata.iter().find(|r| r.0 == "interior").unwrap();
    assert_eq!(interior.1.as_deref(), Some("0x61"));
    assert_eq!(
        metadata
            .iter()
            .find(|r| r.0 == "empty_bytes")
            .unwrap()
            .1
            .as_deref(),
        Some("")
    );
    assert_eq!(
        metadata.iter().find(|r| r.0 == "nullable_bytes").unwrap().1,
        None
    );
    assert_eq!(metadata.iter().find(|r| r.0 == "absent").unwrap().1, None);
    let create: (String, String) = source
        .query_first("SHOW CREATE TABLE source.t10_binary_default")
        .await
        .unwrap()
        .unwrap();
    println!("synthetic SHOW CREATE: {}", create.1);
    assert!(create.1.to_ascii_lowercase().contains("0x00095c7fff"));
    source
        .query_drop("SET character_set_results=binary")
        .await
        .unwrap();
    let binary_metadata: Option<Vec<u8>> = source.query_first("SELECT COLUMN_DEFAULT FROM information_schema.columns WHERE TABLE_SCHEMA='source' AND TABLE_NAME='t10_binary_default' AND COLUMN_NAME='lead_bytes'").await.unwrap().unwrap();
    assert_eq!(binary_metadata.as_deref(), Some(b"0x".as_slice()));
    source
        .query_drop("SET character_set_results=utf8mb4")
        .await
        .unwrap();

    let text_query = "SELECT CONVERT(DEFAULT(t.emoji_value) USING utf8mb4),CONVERT(DEFAULT(t.char_value) USING utf8mb4),CONVERT(DEFAULT(t.enum_value) USING utf8mb4),CONVERT(DEFAULT(t.set_value) USING utf8mb4),CONVERT(DEFAULT(t.latin_value) USING utf8mb4) FROM source.t10_binary_default AS t RIGHT JOIN (SELECT 1 AS anchor) AS a ON FALSE";

    // The anchor yields one result for an empty table, without reading or creating rows.
    let query = "SELECT HEX(DEFAULT(t.lead_bytes)),HEX(DEFAULT(t.interior)),HEX(DEFAULT(t.padded)),HEX(DEFAULT(t.varied)),HEX(DEFAULT(t.empty_bytes)),HEX(DEFAULT(t.nullable_bytes)) FROM source.t10_binary_default AS t RIGHT JOIN (SELECT 1 AS anchor) AS a ON FALSE";
    source
        .query_drop("START TRANSACTION READ ONLY")
        .await
        .unwrap();
    let expected: (String, String, String, String, String, Option<String>) =
        source.query_first(query).await.unwrap().unwrap();
    assert_eq!(
        expected,
        (
            "00095C7FFF".into(),
            "6100627FFF".into(),
            "6100000000".into(),
            "00095C7FFF".into(),
            "".into(),
            None
        )
    );
    let count: u64 = source
        .query_first("SELECT count(*) FROM source.t10_binary_default")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count, 0);
    let text_expected: (String, String, String, String, String) =
        source.query_first(text_query).await.unwrap().unwrap();
    assert_eq!(
        text_expected,
        (
            "{\"face\":\"😀\"}".into(),
            "😀x".into(),
            "😀".into(),
            "😀,plain".into(),
            "€ÿ".into()
        )
    );
    let type_metadata: Vec<(String,String)> = source.query("SELECT COLUMN_NAME,COLUMN_TYPE FROM information_schema.columns WHERE TABLE_SCHEMA='source' AND TABLE_NAME='t10_binary_default' AND COLUMN_NAME IN ('enum_value','set_value') ORDER BY COLUMN_NAME").await.unwrap();
    println!("synthetic enum/set metadata: {type_metadata:?}");
    source.query_drop("ROLLBACK").await.unwrap();
    source
        .query_drop("INSERT INTO source.t10_binary_default(id,absent) VALUES(1,X'')")
        .await
        .unwrap();
    let actual: (String,String,String,String,String,Option<String>) = source.query_first("SELECT HEX(lead_bytes),HEX(interior),HEX(padded),HEX(varied),HEX(empty_bytes),HEX(nullable_bytes) FROM source.t10_binary_default WHERE id=1").await.unwrap().unwrap();
    assert_eq!(
        actual, expected,
        "default metadata must equal an independently inserted source default"
    );
    let text_actual: (String,String,String,String,String) = source.query_first("SELECT emoji_value,char_value,enum_value,set_value,CONVERT(latin_value USING utf8mb4) FROM source.t10_binary_default WHERE id=1").await.unwrap().unwrap();
    assert_eq!(text_actual, text_expected);
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let table = catalog
        .tables
        .iter()
        .find(|t| t.name == "t10_binary_default")
        .unwrap();
    for (name, expected) in [
        ("lead_bytes", "0x00095C7FFF"),
        ("interior", "0x6100627FFF"),
        ("padded", "0x6100000000"),
        ("varied", "0x00095C7FFF"),
        ("empty_bytes", "0x"),
        ("emoji_value", "{\"face\":\"😀\"}"),
        ("char_value", "😀x"),
        ("enum_value", "😀"),
        ("set_value", "😀,plain"),
        ("latin_value", "€ÿ"),
    ] {
        assert_eq!(
            table
                .columns
                .iter()
                .find(|c| c.name == name)
                .unwrap()
                .default
                .as_deref(),
            Some(expected),
            "lossless inspected default {name}"
        );
    }
    for name in ["nullable_bytes", "absent"] {
        assert_eq!(
            table
                .columns
                .iter()
                .find(|c| c.name == name)
                .unwrap()
                .default,
            None
        );
    }
    let guard = catalog
        .tables
        .iter()
        .find(|t| t.name == "t10_binary_default_guard")
        .unwrap();
    assert_eq!(
        guard
            .columns
            .iter()
            .find(|c| c.name == "odd`name")
            .unwrap()
            .default
            .as_deref(),
        Some("0x000961")
    );
    let expression = guard
        .columns
        .iter()
        .find(|c| c.name == "expression_value")
        .unwrap();
    assert!(expression.default_is_expression);
    assert!(
        expression
            .default
            .as_deref()
            .unwrap()
            .to_ascii_lowercase()
            .contains("uuid")
    );
    assert!(
        guard
            .columns
            .iter()
            .find(|c| c.name == "generated_value")
            .unwrap()
            .generation_expression
            .is_some()
    );
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let target_catalog = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    // Recovering defaults does not recover corrupted ENUM/SET type labels.
    let plan::PlanError::Blocked(diagnostics) =
        plan::build(&config, &catalog, &target_catalog).unwrap_err()
    else {
        panic!("expected explicit unsupported default")
    };
    assert!(diagnostics.iter().any(|d| {
        d.code == "LOSSY_LABEL_METADATA"
            && d.object
                .as_deref()
                .is_some_and(|object| object.ends_with("enum_value"))
    }));
    source
        .query_drop(
            "ALTER TABLE source.t10_binary_default DROP COLUMN enum_value,DROP COLUMN set_value",
        )
        .await
        .unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let resolved = plan::build(&config, &catalog, &target_catalog).unwrap();
    for step in &resolved.ddl {
        target.client.batch_execute(&step.sql).await.unwrap();
    }
    let pg_actual=target.client.query_one("INSERT INTO t10_binary_default.t10_binary_default(id,absent) VALUES(2,''::bytea) RETURNING encode(lead_bytes,'hex'),encode(interior,'hex'),encode(padded,'hex'),encode(varied,'hex'),encode(empty_bytes,'hex'),encode(nullable_bytes,'hex'),emoji_value,char_value,latin_value",&[]).await.unwrap();
    for (index, source_bytes) in [&actual.0, &actual.1, &actual.2, &actual.3, &actual.4]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            pg_actual.get::<_, String>(index).to_ascii_uppercase(),
            *source_bytes,
            "independent source/target default bytes"
        );
    }
    assert_eq!(pg_actual.get::<_, Option<String>>(5), actual.5);
    assert_eq!(pg_actual.get::<_, String>(6), text_actual.0);
    assert_eq!(pg_actual.get::<_, String>(7), text_actual.1);
    assert_eq!(pg_actual.get::<_, String>(8), text_actual.4);
    target
        .client
        .batch_execute("DROP SCHEMA t10_binary_default CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
    source
        .query_drop("DROP TABLE source.t10_binary_default,source.t10_binary_default_guard")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
}
