//! Explicitly omitting MySQL ON UPDATE behavior keeps the copied value/default but adds no PG trigger.
use my2pg::{config::MigrationConfig, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::Path};
use tokio::process::Command;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

fn config(
    source_database: &str,
    source_table: &str,
    target_schema: &str,
    report_dir: &Path,
    omit_on_update: bool,
) -> MigrationConfig {
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {
            "url_env": "MY2PG_MYSQL_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "consistency": "frozen",
            "session": {"time_zone": "+00:00"}
        },
        "target": {
            "url_env": "MY2PG_POSTGRES_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "schema": target_schema
        },
        "migration": {
            "mode": "full",
            "reset_sequences": true,
            "batch_rows": 2,
            "batch_bytes": 4096,
            "max_row_bytes": 1024,
            "memory_bytes": 262144
        },
        "tables": {"include": [source_table]},
        "report": {
            "directory": report_dir,
            "console": "json",
            "progress": "never"
        }
    }))
    .unwrap();
    if omit_on_update {
        config.overrides.push(my2pg::config::ObjectOverride {
            object: format!("{source_database}.{source_table}.updated_at.on_update"),
            omit: true,
            target_expression: None,
            target_sql: None,
            materialize: None,
        });
    }
    config
}

async fn run(config: &MigrationConfig, directory: &Path, filename: &str) -> std::process::Output {
    fs::create_dir_all(directory).unwrap();
    let path = directory.join(filename);
    fs::write(&path, toml::to_string(config).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("run")
        .arg(path)
        .args(["--output", "json", "--progress", "never"])
        .output()
        .await
        .unwrap()
}

async fn schema_exists(client: &tokio_postgres::Client, schema: &str) -> bool {
    client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname=$1)",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn native_on_update_requires_explicit_omission_and_omission_does_not_create_trigger() {
    let suffix = std::process::id();
    let source_table = format!("T17OnUpdate{suffix}");
    let target_schema = format!("t17_on_update_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t17-on-update-{suffix}"));
    fs::create_dir_all(&directory).unwrap();

    let source_config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_MYSQL_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "consistency": "frozen",
        "session": {"time_zone": "+00:00"}
    }))
    .unwrap();
    let mut source = mysql::connect(&source_config, &required("MY2PG_MYSQL_ROOT_URL"), 4096)
        .await
        .unwrap();
    let source_ident = mysql::quote_ident(&source_table);
    source
        .query_drop(format!("DROP TABLE IF EXISTS {source_ident}"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE {source_ident}(id INT NOT NULL PRIMARY KEY, note VARCHAR(32) NOT NULL, updated_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6)) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {source_ident}(id,note) VALUES(1,'before')"
        ))
        .await
        .unwrap();
    let source_timestamp: String = source
        .query_first(format!(
            "SELECT CAST(updated_at AS CHAR) FROM {source_ident} WHERE id=1"
        ))
        .await
        .unwrap()
        .unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let source_column = catalog
        .tables
        .iter()
        .find(|table| table.name == source_table)
        .unwrap()
        .columns
        .iter()
        .find(|column| column.name == "updated_at")
        .unwrap();
    assert!(
        source_column
            .extra
            .to_ascii_lowercase()
            .contains("on update"),
        "the native MySQL fixture must expose ON UPDATE metadata: {:?}",
        source_column.extra
    );

    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "schema": target_schema
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    assert!(!schema_exists(&target.client, &target_schema).await);

    let rejected = config(
        &catalog.database,
        &source_table,
        &target_schema,
        &directory.join("rejected-runs"),
        false,
    );
    let output = run(&rejected, &directory, "rejected.toml").await;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "missing ON UPDATE policy succeeded"
    );
    assert!(diagnostics.contains("ON_UPDATE_POLICY"), "{diagnostics}");
    assert!(
        !schema_exists(&target.client, &target_schema).await,
        "planning failure created target schema"
    );

    let accepted = config(
        &catalog.database,
        &source_table,
        &target_schema,
        &directory.join("accepted-runs"),
        true,
    );
    let output = run(&accepted, &directory, "accepted.toml").await;
    assert!(
        output.status.success(),
        "explicit ON UPDATE omission failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let relation = postgres::qualified(&target_schema, &source_table);
    let copied_timestamp: String = target
        .client
        .query_one(
            &format!("SELECT to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US') FROM {relation} WHERE id=1"),
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        copied_timestamp.starts_with(&source_timestamp),
        "copied timestamp must retain the source's UTC wall time: source={source_timestamp:?}, target={copied_timestamp:?}"
    );
    let default: Option<String> = target
        .client
        .query_one(
            "SELECT column_default FROM information_schema.columns WHERE table_schema=$1 AND table_name=$2 AND column_name='updated_at'",
            &[&target_schema, &source_table],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        default
            .as_ref()
            .is_some_and(|value| value.to_ascii_uppercase().contains("CURRENT_TIMESTAMP")),
        "ordinary source default must remain available after omitting update behavior: {default:?}"
    );
    let before_update = copied_timestamp;
    let target_ns = postgres::quote_ident(&target_schema);
    let target_table = postgres::quote_ident(&source_table);
    target
        .client
        .execute(
            &format!("UPDATE {target_ns}.{target_table} SET note='after' WHERE id=1"),
            &[],
        )
        .await
        .unwrap();
    let after_update: String = target
        .client
        .query_one(
            &format!("SELECT to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US') FROM {relation} WHERE id=1"),
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        after_update, before_update,
        "the explicit omission must not install implicit target update behavior"
    );
    let triggers: i64 = target
        .client
        .query_one(
            "SELECT count(*) FROM pg_catalog.pg_trigger t WHERE t.tgrelid=to_regclass($1) AND NOT t.tgisinternal",
            &[&format!("{target_ns}.{target_table}")],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(triggers, 0, "omission must not create a target trigger");
    target
        .client
        .batch_execute(&format!("DROP SCHEMA {target_ns} CASCADE"))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE {source_ident}"))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}
