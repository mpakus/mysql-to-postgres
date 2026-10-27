//! Strangler's explicit text-to-bytea policy preserves source bytes, including defaults.
use my2pg::{config::*, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs};
use tokio::process::Command;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixtures"]
async fn native_text_bytea_preserves_bytes_defaults_and_parallel_readers() {
    let suffix = std::process::id();
    let name = format!("t10_bytea_{suffix}");
    let table = mysql::quote_ident(&name);
    let directory = env::temp_dir().join(format!("my2pg-text-bytea-{suffix}"));
    fs::create_dir_all(&directory).unwrap();
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":name},
        "migration":{"reset_sequences":false,"batch_rows":2,"batch_bytes":8192,
            "max_row_bytes":4096,"memory_bytes":1048576},
        "tables":{"include":[name]},
        "verification":{"mode":"content"},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap();
    for column in [
        "raw`bytes",
        "notes",
        "fixed",
        "empty_value",
        "nullable_value",
        "hex_prefix",
    ] {
        config.cast.push(CastRule {
            source_table: Some(name.clone()),
            source_column: Some(column.into()),
            target_type: Some("bytea".into()),
            // Must never decode bytea through this deliberately unsuitable charset.
            charset: Some("utf8mb4".into()),
            ..Default::default()
        });
    }
    config.cast.push(CastRule {
        source_table: Some(name.clone()),
        source_column: Some("hex_text".into()),
        target_type: Some("bytea".into()),
        transform: Some("hex-to-bytea".into()),
        ..Default::default()
    });
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 4096)
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE {table}(id INT PRIMARY KEY,
        `raw``bytes` VARCHAR(512) DEFAULT 0x0080818d8f909dff5c27,
        notes TEXT, fixed CHAR(4) DEFAULT 0x8081a0ff,
        empty_value VARCHAR(4) DEFAULT '', nullable_value VARCHAR(4) DEFAULT NULL,
        hex_prefix VARCHAR(8) DEFAULT '0x41', hex_text VARCHAR(8) DEFAULT '00ff',
        normal_text VARCHAR(16) DEFAULT '€ÿ') ENGINE=InnoDB DEFAULT CHARSET=latin1"
        ))
        .await
        .unwrap();
    // Inspect while empty, then check the recovered bytes against a real inserted default.
    source
        .query_drop(format!(
            "CREATE FUNCTION {}() RETURNS INTEGER DETERMINISTIC RETURN 1",
            mysql::quote_ident(&format!("{name}_function"))
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE PROCEDURE {}() SELECT 1",
            mysql::quote_ident(&format!("{name}_procedure"))
        ))
        .await
        .unwrap();
    let empty_catalog = mysql::inspect(&mut source).await.unwrap();
    for kind in ["function", "procedure"] {
        assert!(
            empty_catalog
                .unsupported_objects
                .iter()
                .any(|object| object.kind == kind && object.object == format!("{name}_{kind}"))
        );
    }
    source
        .query_drop(format!("INSERT INTO {table}(id) VALUES (4)"))
        .await
        .unwrap();
    let all_bytes: String = (0..=255).map(|byte| format!("{byte:02x}")).collect();
    source.exec_drop(format!("INSERT INTO {table}(id,`raw``bytes`,notes) VALUES (1,CONVERT(UNHEX(?) USING latin1),CONVERT(UNHEX(?) USING latin1)),(2,X'',X''),(3,NULL,NULL)"),
        (&all_bytes, &all_bytes)).await.unwrap();
    let expected: Vec<mysql_async::Row> = source.query(format!("SELECT id,HEX(`raw``bytes`),HEX(notes),HEX(fixed),HEX(empty_value),HEX(nullable_value),HEX(hex_prefix),LOWER(hex_text),CONVERT(normal_text USING utf8mb4) FROM {table} ORDER BY id")).await.unwrap();
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    for readers in [1, 2] {
        config.target.schema = format!("{name}_{readers}");
        config.migration.readers_per_table = readers;
        config.migration.max_key_span = (readers > 1).then_some(2);
        // Exercise the pgloader spelling as well as transform-free bytea casts.
        if readers > 1 {
            for cast in &mut config.cast[..6] {
                cast.transform = Some("binary-to-bytea".into());
            }
        }
        let mut reader = mysql::connect(&config.source, &required("MY2PG_MYSQL_URL"), 4096)
            .await
            .unwrap();
        let encoding: String = reader
            .query_first("SELECT @@character_set_results")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(encoding, "utf8mb4");
        reader.disconnect().await.unwrap();
        let path = directory.join(format!("{readers}.toml"));
        fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(&path)
            .args(["--output", "json", "--progress", "never"])
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let ranges: std::collections::BTreeSet<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter_map(|event| event["range"].as_str().map(str::to_string))
            .collect();
        assert_eq!(ranges.len(), if readers > 1 { 2 } else { 0 });
        let qualified = postgres::qualified(&config.target.schema, &name);
        let outcome: serde_json::Value = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|event| event["kind"] == "outcome")
            .unwrap();
        assert_eq!(outcome["report"]["verification"]["status"], "complete");
        let run = outcome["report"]["artifact_dir"].as_str().unwrap();
        for (change, status, exit) in [
            (None, "complete", 0),
            (
                Some(format!(
                    "UPDATE {qualified} SET \"raw`bytes\"=set_byte(\"raw`bytes\",255,0) WHERE id=1"
                )),
                "different",
                4,
            ),
            (
                Some(format!(
                    "UPDATE {qualified} SET \"raw`bytes\"=set_byte(\"raw`bytes\",255,255) WHERE id=1"
                )),
                "complete",
                0,
            ),
        ] {
            if let Some(sql) = change {
                target.client.batch_execute(&sql).await.unwrap();
            }
            let checked = Command::new(env!("CARGO_BIN_EXE_my2pg"))
                .arg("verify")
                .arg(&path)
                .arg("--run-dir")
                .arg(run)
                .args(["--mode", "content", "--output", "json"])
                .output()
                .await
                .unwrap();
            assert_eq!(
                checked.status.code(),
                Some(exit),
                "{}",
                String::from_utf8_lossy(&checked.stderr)
            );
            let document: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
            assert_eq!(document["verification"]["status"], status);
        }
        let actual = target.client.query(&format!("SELECT id,encode(\"raw`bytes\",'hex'),encode(notes,'hex'),encode(fixed,'hex'),encode(empty_value,'hex'),encode(nullable_value,'hex'),encode(hex_prefix,'hex'),encode(hex_text,'hex'),normal_text FROM {qualified} ORDER BY id"), &[]).await.unwrap();
        assert_eq!(actual.len(), expected.len());
        for (row, original) in actual.iter().zip(&expected) {
            assert_eq!(row.get::<_, i32>(0), original.get::<i32, _>(0).unwrap());
            for index in 1..8 {
                assert_eq!(
                    row.get::<_, Option<String>>(index),
                    original
                        .get::<Option<String>, _>(index)
                        .unwrap()
                        .map(|s| s.to_ascii_lowercase()),
                    "column {index}, readers {readers}"
                );
            }
            assert_eq!(row.get::<_, String>(8), "€ÿ");
        }
        // Independently check both empty-table metadata and target INSERT defaults.
        let mut empty_config = config.clone();
        empty_config.target.schema = format!("{name}_empty_{readers}");
        let catalog = my2pg::postgres::inspect(&mut target.client, &empty_config.target.schema)
            .await
            .unwrap();
        let empty_plan = my2pg::plan::build(&empty_config, &empty_catalog, &catalog).unwrap();
        for kind in ["function", "procedure"] {
            assert!(
                empty_plan
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.object.as_deref()
                        == Some(&format!("{name}_{kind}"))
                        && diagnostic.severity == my2pg::model::Severity::Warning)
            );
        }
        for step in &empty_plan.ddl {
            target.client.batch_execute(&step.sql).await.unwrap();
        }
        for relation in [
            qualified,
            postgres::qualified(&empty_config.target.schema, &name),
        ] {
            let defaults = target.client.query_one(&format!("INSERT INTO {relation}(id) VALUES(5) RETURNING encode(\"raw`bytes\",'hex'),encode(fixed,'hex'),encode(empty_value,'hex'),encode(nullable_value,'hex'),encode(hex_prefix,'hex'),encode(hex_text,'hex')"), &[]).await.unwrap();
            assert_eq!(defaults.get::<_, String>(0), "0080818d8f909dff5c27");
            assert_eq!(defaults.get::<_, String>(1), "8081a0ff");
            assert_eq!(defaults.get::<_, String>(2), "");
            assert_eq!(defaults.get::<_, Option<String>>(3), None);
            assert_eq!(defaults.get::<_, String>(4), "30783431");
            assert_eq!(defaults.get::<_, String>(5), "00ff");
        }
        target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA {} CASCADE; DROP SCHEMA {} CASCADE",
                postgres::quote_ident(&config.target.schema),
                postgres::quote_ident(&empty_config.target.schema)
            ))
            .await
            .unwrap();
    }
    let count: u64 = source
        .query_first(format!("SELECT count(*) FROM {table}"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count, 4);
    source
        .query_drop(format!("DROP TABLE {table}"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "DROP FUNCTION {}",
            mysql::quote_ident(&format!("{name}_function"))
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "DROP PROCEDURE {}",
            mysql::quote_ident(&format!("{name}_procedure"))
        ))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}
