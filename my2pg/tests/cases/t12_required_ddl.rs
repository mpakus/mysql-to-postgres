//! Required target structure failures remain visible after committed COPY.
use my2pg::{config::MigrationConfig, model::*, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::PathBuf};
use tokio::process::Command;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

#[tokio::test]
#[ignore = "requires owned MySQL 8+ and PostgreSQL TLS fixtures"]
async fn native_required_check_ddl_failure_persists_failed_report_after_copy() {
    let suffix = std::process::id();
    let source_table = format!("T12RequiredDdl{suffix}");
    let target_schema = format!("t12_required_ddl_{suffix}");
    let directory =
        PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(format!("t12_required_ddl_{suffix}"));
    fs::create_dir_all(&directory).unwrap();

    let source_config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_MYSQL_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "consistency": "frozen"
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
            "CREATE TABLE {source_ident}(id INT NOT NULL PRIMARY KEY, amount INT NOT NULL, CONSTRAINT amount_nonnegative CHECK (amount >= 0)) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("INSERT INTO {source_ident}(id,amount) VALUES(1,5)"))
        .await
        .unwrap();
    let source_catalog = mysql::inspect(&mut source).await.unwrap();
    assert!(
        source_catalog
            .tables
            .iter()
            .find(|table| table.name == source_table)
            .unwrap()
            .checks
            .iter()
            .any(|check| check.name == "amount_nonnegative" && check.enforced)
    );
    let source_database = source_catalog.database;

    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {
            "url_env": "MY2PG_MYSQL_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "consistency": "frozen"
        },
        "target": {
            "url_env": "MY2PG_POSTGRES_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "schema": target_schema
        },
        "migration": {
            "mode": "full",
            "reset_sequences": false,
            "batch_rows": 2,
            "batch_bytes": 4096,
            "max_row_bytes": 1024,
            "memory_bytes": 262144
        },
        "tables": {
            "include": [source_table],
            "rename": [{"source": source_table, "target": "rows"}]
        },
        "report": {
            "directory": directory.join("runs"),
            "console": "json",
            "progress": "never"
        }
    }))
    .unwrap();
    config.overrides.push(my2pg::config::ObjectOverride {
        object: format!("{source_database}.{source_table}.amount_nonnegative"),
        omit: false,
        target_expression: Some("(amount >".into()),
        target_sql: None,
        materialize: None,
    });

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

    let config_path = directory.join("migration.toml");
    fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("run")
        .arg(config_path)
        .args(["--output", "json", "--progress", "never"])
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    for secret in [required("MY2PG_MYSQL_URL"), required("MY2PG_POSTGRES_URL")] {
        assert!(!stdout.contains(&secret));
        assert!(!stderr.contains(&secret));
    }
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let outcome = events
        .iter()
        .find(|event| event["kind"] == "outcome")
        .unwrap();
    let report: RunReport = serde_json::from_value(outcome["report"].clone()).unwrap();
    assert_eq!(outcome["exit_code"], 1);
    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(report.tables.len(), 1);
    assert_eq!(report.tables[0].status, RunStatus::Failed);
    assert_eq!(report.tables[0].rows_read, 1);
    assert_eq!(report.tables[0].committed_rows, 1);
    assert!(
        report
            .failed_steps
            .iter()
            .any(|step| step == &format!("{source_database}.{source_table}.amount_nonnegative"))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "DDL" && diagnostic.message.contains("42601"))
    );
    assert!(!stdout.contains("syntax error at end of input"));

    let persisted: RunReport = serde_json::from_slice(
        &fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        serde_json::to_value(persisted).unwrap()
    );
    assert!(schema_exists(&target.client, &target_schema).await);
    let target_table = postgres::qualified(&target_schema, "rows");
    let target_rows: i64 = target
        .client
        .query_one(&format!("SELECT count(*) FROM {target_table}"), &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(target_rows, 1);
    let installed_check: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_constraint con JOIN pg_catalog.pg_class c ON c.oid=con.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname='rows' AND con.conname='amount_nonnegative' AND con.contype='c')",
            &[&target_schema],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!installed_check, "failed CHECK DDL must not be installed");

    source
        .query_drop(format!("DROP TABLE {source_ident}"))
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            my2pg::plan::quote_identifier(&target_schema)
        ))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
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
