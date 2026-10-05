//! Native acceptance for unsigned BIGINT data bounds and source identity policy.
use my2pg::{config::MigrationConfig, postgres};
use mysql_async::prelude::Queryable;
use std::{env, path::Path, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}

fn source_config() -> my2pg::config::SourceConfig {
    serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_MYSQL_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "consistency": "frozen"
    }))
    .unwrap()
}

fn target_config(schema: &str) -> my2pg::config::TargetConfig {
    serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "schema": schema,
        "ca_file": required("MY2PG_TLS_CA")
    }))
    .unwrap()
}

fn migration_config(schema: &str, table: &str, report_dir: &Path) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {
            "url_env": "MY2PG_MYSQL_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "consistency": "frozen"
        },
        "target": {
            "url_env": "MY2PG_POSTGRES_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "schema": schema
        },
        "migration": {
            "batch_rows": 2,
            "batch_bytes": 4096,
            "max_row_bytes": 1024,
            "memory_bytes": 262144
        },
        "tables": {"include": [table]},
        "report": {
            "directory": report_dir,
            "console": "json",
            "progress": "never"
        }
    }))
    .unwrap()
}

async fn run(config: &MigrationConfig, directory: &Path, label: &str) -> std::process::Output {
    let path = directory.join(format!("{label}.toml"));
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
async fn native_runner_round_trips_unsigned_bigint_max_as_numeric() {
    let suffix = std::process::id();
    let schema = format!("t11_u64_{suffix}");
    let table = format!("T11UnsignedMaximum{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t11-u64-{suffix}"));
    std::fs::create_dir_all(&directory).unwrap();
    let mut source =
        my2pg::mysql::connect(&source_config(), &required("MY2PG_MYSQL_ROOT_URL"), 1024)
            .await
            .unwrap();
    let target = postgres::connect(&target_config(&schema), &required("MY2PG_POSTGRES_URL"))
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
            "CREATE TABLE `{table}` (value BIGINT UNSIGNED NOT NULL) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO `{table}` (value) VALUES (18446744073709551615)"
        ))
        .await
        .unwrap();

    let output = run(
        &migration_config(&schema, &table, &directory),
        &directory,
        "unsigned-maximum",
    )
    .await;
    assert!(
        output.status.success(),
        "migration failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let relation = postgres::qualified(&schema, &table);
    let observed_type: String = target
        .client
        .query_one(
            "SELECT pg_catalog.format_type(a.atttypid,a.atttypmod) \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid \
             WHERE n.nspname=$1 AND c.relname=$2 AND a.attname='value' \
               AND a.attnum>0 AND NOT a.attisdropped",
            &[&schema, &table],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(observed_type, "numeric(20,0)");
    let round_trip: String = target
        .client
        .query_one(&format!("SELECT value::text FROM {relation}"), &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(round_trip, "18446744073709551615");

    source
        .query_drop(format!("DROP TABLE `{table}`"))
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .unwrap();
    target.close().await.unwrap();
    source.disconnect().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn native_runner_refuses_nondefault_source_auto_increment_policy_before_ddl() {
    let suffix = std::process::id();
    let schema = format!("t11_policy_{suffix}");
    let table = format!("T11IdentityPolicy{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t11-policy-{suffix}"));
    std::fs::create_dir_all(&directory).unwrap();
    let mut admin =
        my2pg::mysql::connect(&source_config(), &required("MY2PG_MYSQL_ROOT_URL"), 1024)
            .await
            .unwrap();
    let target = postgres::connect(&target_config(&schema), &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();

    target
        .client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .await
        .unwrap();
    admin
        .query_drop(format!("DROP TABLE IF EXISTS `{table}`"))
        .await
        .unwrap();
    admin
        .query_drop(format!(
            "CREATE TABLE `{table}` (id INT NOT NULL AUTO_INCREMENT PRIMARY KEY) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!("INSERT INTO `{table}` (id) VALUES (5)"))
        .await
        .unwrap();

    let previous: (u64, u64) = admin
        .query_first("SELECT @@GLOBAL.auto_increment_increment, @@GLOBAL.auto_increment_offset")
        .await
        .unwrap()
        .unwrap();
    admin
        .query_drop("SET GLOBAL auto_increment_increment=2")
        .await
        .unwrap();
    admin
        .query_drop("SET GLOBAL auto_increment_offset=2")
        .await
        .unwrap();

    // The runner opens another MySQL connection, so verify its connection-level
    // metadata sees the changed server defaults before exercising the refusal.
    let mut probe =
        my2pg::mysql::connect(&source_config(), &required("MY2PG_MYSQL_ROOT_URL"), 1024)
            .await
            .unwrap();
    let catalog = my2pg::mysql::inspect(&mut probe).await.unwrap();
    probe.disconnect().await.unwrap();
    let migration = migration_config(&schema, &table, &directory);
    let output = run(&migration, &directory, "nondefault-source-policy").await;

    // Restore the disposable MySQL instance before making any test assertions.
    admin
        .query_drop(format!(
            "SET GLOBAL auto_increment_increment={}",
            previous.0
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!("SET GLOBAL auto_increment_offset={}", previous.1))
        .await
        .unwrap();
    let restored: (u64, u64) = admin
        .query_first("SELECT @@GLOBAL.auto_increment_increment, @@GLOBAL.auto_increment_offset")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored, previous, "restore disposable MySQL globals");

    assert_eq!(
        (
            catalog.auto_increment_increment,
            catalog.auto_increment_offset
        ),
        (2, 2),
        "fresh source session observes the non-default server generation policy"
    );
    assert!(
        !output.status.success(),
        "unsupported identity policy must refuse"
    );
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("IDENTITY_INCREMENT_OFFSET"),
        "{diagnostic}"
    );
    assert!(
        !has_schema(&target.client, &schema).await,
        "generation-policy refusal must happen before target schema creation"
    );
    let source_rows: Vec<(i32,)> = admin
        .query(format!("SELECT id FROM `{table}` ORDER BY id"))
        .await
        .unwrap();
    assert_eq!(source_rows, [(5,)], "refusal leaves source rows unchanged");

    admin
        .query_drop(format!("DROP TABLE `{table}`"))
        .await
        .unwrap();
    target.close().await.unwrap();
    admin.disconnect().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
