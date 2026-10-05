use clap::Parser;
use my2pg::{cli::Args, config::*, model::TargetCatalog, mysql, plan, postgres, report::Console};
use mysql_async::{Row, prelude::Queryable};
use std::env;

#[tokio::test]
#[ignore = "requires owned Oracle MySQL 5.7/8.0/8.4 and PostgreSQL 16/17/18 TLS fixture"]
async fn supplementary_enum_set_labels_are_indistinguishable_from_question_marks_in_catalogs() {
    let config: SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_MYSQL_URL", "consistency":"frozen",
        "ca_file":env::var("MY2PG_TLS_CA").unwrap()
    }))
    .unwrap();
    let mut source = mysql::connect(&config, &env::var("MY2PG_MYSQL_ROOT_URL").unwrap(), 16384)
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE IF EXISTS source.t10_enum_metadata")
        .await
        .unwrap();
    source.query_drop("CREATE TABLE source.t10_enum_metadata(id INT PRIMARY KEY, lost_enum ENUM('😀','plain') DEFAULT NULL, control_enum ENUM('?','plain') DEFAULT NULL, lost_set SET('😀','plain') DEFAULT NULL, control_set SET('?','plain') DEFAULT NULL, bmp ENUM('é','plain') DEFAULT NULL, native_default ENUM('😀','plain') DEFAULT '😀') ENGINE=InnoDB CHARACTER SET utf8mb4").await.unwrap();
    let (version, system): (String, String) = source
        .query_first("SELECT VERSION(), @@character_set_system")
        .await
        .unwrap()
        .unwrap();
    let expected_system = if version.starts_with("5.7.") {
        "utf8"
    } else if version.starts_with("8.0.") || version.starts_with("8.4.") {
        "utf8mb3"
    } else {
        panic!("unreviewed Oracle MySQL version: {version}");
    };
    assert_eq!(system, expected_system);
    for charset in ["utf16", "utf16le", "utf32", "gb18030"] {
        for kind in ["enum", "set"] {
            source.query_drop(format!("ALTER TABLE source.t10_enum_metadata ADD COLUMN {kind}_{charset} {kind}('😀','plain') CHARACTER SET {charset} DEFAULT NULL")).await.unwrap();
        }
    }
    source
        .query_drop("START TRANSACTION READ ONLY")
        .await
        .unwrap();
    let count: u64 = source
        .query_first("SELECT COUNT(*) FROM source.t10_enum_metadata")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        count, 0,
        "unused labels cannot be reconstructed by scanning source rows"
    );
    for result_charset in ["utf8mb4", "binary"] {
        source
            .query_drop(format!("SET character_set_results={result_charset}"))
            .await
            .unwrap();
        let metadata:Vec<(String,String,String)>=source.query("SELECT COLUMN_NAME,COLUMN_TYPE,HEX(COLUMN_TYPE) FROM information_schema.COLUMNS WHERE TABLE_SCHEMA='source' AND TABLE_NAME='t10_enum_metadata' ORDER BY ORDINAL_POSITION").await.unwrap();
        let kind = |name: &str| metadata.iter().find(|row| row.0 == name).unwrap().1.clone();
        assert_eq!(kind("lost_enum"), "enum('?','plain')");
        assert_eq!(kind("lost_enum"), kind("control_enum"));
        assert_eq!(kind("lost_set"), "set('?','plain')");
        assert_eq!(kind("lost_set"), kind("control_set"));
        assert_eq!(kind("bmp"), "enum('é','plain')");
        for charset in ["utf16", "utf16le", "utf32", "gb18030"] {
            for source_kind in ["enum", "set"] {
                assert_eq!(
                    kind(&format!("{source_kind}_{charset}")),
                    format!("{source_kind}('?','plain')"),
                    "{charset}"
                );
            }
        }
        println!("synthetic {result_charset} type metadata: {metadata:?}");
        let create: (String, String) = source
            .query_first("SHOW CREATE TABLE source.t10_enum_metadata")
            .await
            .unwrap()
            .unwrap();
        assert!(create.1.contains("`lost_enum` enum('?','plain')"));
        assert!(create.1.contains("`lost_set` set('?','plain')"));
        let full: Vec<Row> = source
            .query("SHOW FULL COLUMNS FROM source.t10_enum_metadata")
            .await
            .unwrap();
        for (name, expected) in [
            ("lost_enum", "enum('?','plain')"),
            ("lost_set", "set('?','plain')"),
        ] {
            let column = full
                .iter()
                .find(|row| row.get::<String, _>(0).as_deref() == Some(name))
                .unwrap();
            assert_eq!(column.get::<String, _>(1).as_deref(), Some(expected));
        }
        // A complete lossless native default does not reveal other declared labels.
        let default: String=source.query_first("SELECT CONVERT(DEFAULT(t.native_default) USING utf8mb4) FROM source.t10_enum_metadata AS t RIGHT JOIN (SELECT 1) AS anchor ON FALSE").await.unwrap().unwrap();
        assert_eq!(default, "😀");
    }
    source
        .query_drop("SET character_set_results=utf8mb4")
        .await
        .unwrap();
    let statement = source
        .prep("SELECT lost_enum,lost_set FROM source.t10_enum_metadata")
        .await
        .unwrap();
    assert_eq!(statement.columns().len(), 2);
    println!("synthetic prepared metadata: {:?}", statement.columns());
    let empty: Vec<Row> = source.exec(&statement, ()).await.unwrap();
    assert!(empty.is_empty());
    let dictionary = source
        .query::<Row, _>("SELECT * FROM mysql.column_type_elements LIMIT 1")
        .await;
    assert!(
        dictionary.is_err(),
        "release dictionary has no direct read-only label access"
    );
    println!(
        "synthetic protected dictionary error: {}",
        dictionary.unwrap_err()
    );
    source.query_drop("ROLLBACK").await.unwrap();
    source.query_drop("INSERT INTO source.t10_enum_metadata(id,lost_enum,control_enum,lost_set,control_set,bmp) VALUES(1,'😀','?','😀,plain','?,plain','é')").await.unwrap();
    let actual:(String,String,String,String,u64,u64)=source.query_first("SELECT lost_enum,control_enum,lost_set,control_set,lost_enum+0,lost_set+0 FROM source.t10_enum_metadata WHERE id=1").await.unwrap().unwrap();
    assert_eq!(
        actual,
        (
            "😀".into(),
            "?".into(),
            "😀,plain".into(),
            "?,plain".into(),
            1,
            3
        )
    );
    let mut migration:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":env::var("MY2PG_TLS_CA").unwrap()},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":"t10_enum_metadata_guard","ca_file":env::var("MY2PG_TLS_CA").unwrap()},
        "tables":{"include":["t10_enum_metadata"]},
        "report":{"directory":std::path::PathBuf::from(env::var("MY2PG_ARTIFACT_DIR").unwrap()).join("t10_enum_metadata_guard"),"progress":"never"}
    })).unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let target_catalog = TargetCatalog {
        server_version: "16.15".into(),
        can_create_schema: true,
        can_use_schema: true,
        can_create_objects: true,
        ..Default::default()
    };
    let plan::PlanError::Blocked(diagnostics) =
        plan::build(&migration, &catalog, &target_catalog).unwrap_err()
    else {
        panic!("expected blocking diagnostics")
    };
    assert!(
        diagnostics
            .iter()
            .any(|error| error.code == "LOSSY_LABEL_METADATA")
    );
    migration.cast.push(CastRule {
        source_table: Some("t10_enum_metadata".into()),
        source_column: Some("lost_enum".into()),
        target_type: Some("text".into()),
        ..Default::default()
    });
    let credentials = resolve_credentials(&migration).unwrap();
    let args = Args::try_parse_from(["my2pg", "run", "synthetic.toml", "--quiet"]).unwrap();
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    assert_eq!(
        crate::test_pipeline::run(
            &migration,
            &credentials,
            &mut Console::new(&migration, &args),
            receiver
        )
        .await
        .unwrap_err()
        .exit_code(),
        2
    );
    let target = postgres::connect(&migration.target, credentials.target.expose())
        .await
        .unwrap();
    let exists: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT FROM pg_catalog.pg_namespace WHERE nspname=$1)",
            &[&migration.target.schema],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !exists,
        "ambiguous metadata must fail before any target schema mutation"
    );
    target.close().await.unwrap();
    source
        .query_drop("DROP TABLE source.t10_enum_metadata")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
}
