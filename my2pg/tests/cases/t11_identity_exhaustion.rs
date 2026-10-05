//! Native source AUTO_INCREMENT exhaustion at the mapped PostgreSQL bound.
use my2pg::{config::MigrationConfig, postgres};
use mysql_async::prelude::Queryable;
use std::{env, path::Path, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}

fn migration_config(schema: &str, table: &str, report_dir: &Path) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":schema},
        "migration":{"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":[table]},
        "report":{"directory":report_dir,"console":"json","progress":"never"}
    }))
    .unwrap()
}

async fn run(config: &MigrationConfig, directory: &Path) -> std::process::Output {
    let path = directory.join("identity-exhaustion.toml");
    std::fs::write(&path, toml::to_string(config).unwrap()).unwrap();
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(path)
            .args(["--output", "json", "--progress", "never"])
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

async fn has_schema(client: &tokio_postgres::Client, schema: &str) -> bool {
    client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname=$1)",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn native_runner_refuses_source_auto_increment_beyond_smallint_sequence_max() {
    let suffix = std::process::id();
    let schema = format!("t11_identity_exhausted_{suffix}");
    let table = format!("T11IdentityExhausted{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t11-exhausted-{suffix}"));
    std::fs::create_dir_all(&directory).unwrap();

    let source_config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_MYSQL_URL",
        "ca_file":required("MY2PG_TLS_CA"),
        "consistency":"frozen"
    }))
    .unwrap();
    let mut source = my2pg::mysql::connect(&source_config, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_POSTGRES_URL",
        "schema":schema,
        "ca_file":required("MY2PG_TLS_CA")
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();

    target
        .client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE IF EXISTS `{table}`"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE `{table}` (id SMALLINT NOT NULL AUTO_INCREMENT PRIMARY KEY) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("INSERT INTO `{table}` (id) VALUES (32767)"))
        .await
        .unwrap();
    source
        .query_drop(format!("ALTER TABLE `{table}` AUTO_INCREMENT=32768"))
        .await
        .unwrap();

    let catalog = my2pg::mysql::inspect(&mut source).await.unwrap();
    let source_table = catalog
        .tables
        .iter()
        .find(|candidate| candidate.name == table)
        .unwrap_or_else(|| panic!("source inspector omitted selected table {table}"));
    assert_eq!(
        source_table.next_auto_increment,
        Some(32768),
        "source catalog must expose the next value beyond SMALLINT's signed maximum"
    );
    let migration = migration_config(&schema, &table, &directory.join("runs"));
    let output = run(&migration, &directory).await;
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "unrepresentable next identity must refuse before target DDL: {diagnostic}"
    );
    assert!(diagnostic.contains("IDENTITY_RANGE"), "{diagnostic}");
    assert!(
        !has_schema(&target.client, &schema).await,
        "identity-range refusal must happen before target schema creation"
    );
    let source_id: i16 = source
        .query_first(format!("SELECT id FROM `{table}`"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source_id, i16::MAX, "refusal leaves source data intact");

    source
        .query_drop(format!("DROP TABLE `{table}`"))
        .await
        .unwrap();
    target.close().await.unwrap();
    source.disconnect().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
