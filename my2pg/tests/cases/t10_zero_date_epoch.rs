//! Fresh migrations preserve soft-deletion sentinels without a target UPDATE pass.
use my2pg::{config::*, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs};
use tokio::process::Command;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixtures"]
async fn native_epoch_dates_preserve_null_defaults_and_temporal_content() {
    let name = format!("t10_epoch_{}", std::process::id());
    let table = mysql::quote_ident(&name);
    let directory = env::temp_dir().join(format!("my2pg-{name}"));
    fs::create_dir_all(&directory).unwrap();
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":name},
        "migration":{"reset_sequences":false,"batch_rows":2,"batch_bytes":8192,
            "max_row_bytes":4096,"memory_bytes":1048576},
        "tables":{"include":[name]}, "verification":{"mode":"content"},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap();
    for column in ["day", "wall", "discarded_at", "instant"] {
        config.cast.push(CastRule {
            source_table: Some(name.clone()),
            source_column: Some(column.into()),
            target_type: (column == "discarded_at").then(|| "timestamptz(6)".into()),
            transform: Some("zero-dates-to-epoch".into()),
            ..Default::default()
        });
    }
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 4096)
        .await
        .unwrap();
    source.query_drop("SET SESSION sql_mode=''").await.unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE {table}(id INT PRIMARY KEY,
        day DATE DEFAULT '0000-00-00', wall DATETIME(6) DEFAULT '0000-00-00 00:00:00',
        discarded_at DATETIME(6) DEFAULT '0000-00-00 00:00:00',
        instant TIMESTAMP(6) NULL DEFAULT '0000-00-00 00:00:00') ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("INSERT INTO {table}(id) VALUES (1)"))
        .await
        .unwrap();
    source.query_drop(format!("INSERT INTO {table} VALUES (2,NULL,NULL,NULL,NULL),
        (3,'2024-02-29','2024-02-29 12:34:56.123456','2024-02-29 12:34:56.123456','2024-02-29 12:34:56.123456'),
        (4,'1970-01-01','1970-01-01 00:00:00','1970-01-01 00:00:00','1970-01-01 00:00:01')")).await.unwrap();
    let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    for readers in [1, 2] {
        config.target.schema = format!("{name}_{readers}");
        config.migration.readers_per_table = readers;
        config.migration.max_key_span = (readers > 1).then_some(2);
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
        let outcome: serde_json::Value = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|event| event["kind"] == "outcome")
            .unwrap();
        assert_eq!(outcome["report"]["verification"]["status"], "complete");
        let transformations = outcome["report"]["tables"][0]["transformations"]
            .as_object()
            .unwrap();
        assert_eq!(transformations.len(), 4);
        assert!(transformations.values().all(|count| count == 1));
        let qualified = postgres::qualified(&config.target.schema, &name);
        let rows = target.client.query(&format!("SELECT id,day::text,wall::text,discarded_at::text,instant::text FROM {qualified} ORDER BY id"), &[]).await.unwrap();
        let expected = [
            [
                Some("1970-01-01"),
                Some("1970-01-01 00:00:00"),
                Some("1970-01-01 00:00:00+00"),
                Some("1970-01-01 00:00:00+00"),
            ],
            [None, None, None, None],
            [
                Some("2024-02-29"),
                Some("2024-02-29 12:34:56.123456"),
                Some("2024-02-29 12:34:56.123456+00"),
                Some("2024-02-29 12:34:56.123456+00"),
            ],
            [
                Some("1970-01-01"),
                Some("1970-01-01 00:00:00"),
                Some("1970-01-01 00:00:00+00"),
                Some("1970-01-01 00:00:01+00"),
            ],
        ];
        assert_eq!(rows.len(), expected.len());
        for ((row, values), id) in rows.iter().zip(expected).zip(1..=4) {
            assert_eq!(row.get::<_, i32>(0), id);
            for (index, value) in values.iter().enumerate() {
                assert_eq!(row.get::<_, Option<String>>(index + 1).as_deref(), *value);
            }
        }
        let run = outcome["report"]["artifact_dir"].as_str().unwrap();
        for (sql, exit) in [
            (
                format!("UPDATE {qualified} SET discarded_at='1970-01-01 00:00:00+00' WHERE id=2"),
                4,
            ),
            (
                format!("UPDATE {qualified} SET discarded_at=NULL WHERE id=2"),
                0,
            ),
            (
                format!("UPDATE {qualified} SET discarded_at=NULL WHERE id=1"),
                4,
            ),
            (
                format!("UPDATE {qualified} SET discarded_at='1970-01-01 00:00:00+00' WHERE id=1"),
                0,
            ),
            (
                format!(
                    "UPDATE {qualified} SET instant=instant+interval '1 microsecond' WHERE id=3"
                ),
                4,
            ),
            (
                format!(
                    "UPDATE {qualified} SET instant=instant-interval '1 microsecond' WHERE id=3"
                ),
                0,
            ),
        ] {
            target.client.batch_execute(&sql).await.unwrap();
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
                "{}{}",
                String::from_utf8_lossy(&checked.stdout),
                String::from_utf8_lossy(&checked.stderr)
            );
        }
        target
            .client
            .batch_execute(&format!("INSERT INTO {qualified}(id) VALUES (5)"))
            .await
            .unwrap();
        let defaults: bool = target.client.query_one(&format!("SELECT day=DATE '1970-01-01' AND wall=TIMESTAMP '1970-01-01' AND discarded_at=TIMESTAMPTZ '1970-01-01 UTC' AND instant=TIMESTAMPTZ '1970-01-01 UTC' FROM {qualified} WHERE id=5"), &[]).await.unwrap().get(0);
        assert!(defaults);
        target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA {} CASCADE",
                postgres::quote_ident(&config.target.schema)
            ))
            .await
            .unwrap();
    }
    // Known unsupported verification must refuse before creating any target schema or artifacts.
    source
        .query_drop(format!("ALTER TABLE {table} ADD rate DOUBLE"))
        .await
        .unwrap();
    config.target.schema = format!("{name}_unsupported");
    config.report.directory = directory.join("unsupported-runs");
    let path = directory.join("unsupported.toml");
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("run")
        .arg(&path)
        .args(["--output", "json"])
        .output()
        .await
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("CONTENT_VERIFICATION_UNSUPPORTED"));
    assert!(!config.report.directory.exists());
    let exists: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname=$1)",
            &[&config.target.schema],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists);
    let original: Option<String> = source
        .query_first(format!(
            "SELECT CAST(discarded_at AS CHAR) FROM {table} WHERE id=1"
        ))
        .await
        .unwrap();
    assert_eq!(original.as_deref(), Some("0000-00-00 00:00:00.000000"));
    source
        .query_drop(format!("DROP TABLE {table}"))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}
