//! Enforced MySQL CHECK constraints need an explicit PostgreSQL expression.
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
    expression: Option<&str>,
) -> MigrationConfig {
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
            "directory": report_dir,
            "console": "json",
            "progress": "never"
        }
    }))
    .unwrap();
    if let Some(expression) = expression {
        config.overrides.push(my2pg::config::ObjectOverride {
            object: format!("{source_database}.{source_table}.amount_nonnegative"),
            omit: false,
            target_expression: Some(expression.into()),
            target_sql: None,
            materialize: None,
        });
    }
    config
}

async fn run(config: &MigrationConfig, directory: &Path, label: &str) -> std::process::Output {
    fs::create_dir_all(directory).unwrap();
    let path = directory.join(format!("{label}.toml"));
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
async fn native_check_requires_explicit_postgres_expression_and_verifies_it_exactly() {
    let suffix = std::process::id();
    let source_table = format!("T17Check{suffix}");
    let accepted_schema = format!("t17_check_accepted_{suffix}");
    let rejected_schema = format!("t17_check_rejected_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t17-check-{suffix}"));
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
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let source_database = catalog.database.clone();
    let source_check = catalog
        .tables
        .iter()
        .find(|table| table.name == source_table)
        .unwrap()
        .checks
        .iter()
        .find(|check| check.name == "amount_nonnegative")
        .unwrap();
    assert!(source_check.enforced);

    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "schema": accepted_schema
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    for schema in [&accepted_schema, &rejected_schema] {
        assert!(!schema_exists(&target.client, schema).await);
    }

    let rejected = config(
        &source_database,
        &source_table,
        &rejected_schema,
        &directory.join("rejected-runs"),
        None,
    );
    let output = run(&rejected, &directory, "missing-policy").await;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "missing CHECK policy succeeded");
    assert!(
        diagnostics.contains("CHECK_NOT_IMPLEMENTED"),
        "{diagnostics}"
    );
    assert!(
        !schema_exists(&target.client, &rejected_schema).await,
        "missing CHECK policy must block before target schema creation"
    );

    let accepted = config(
        &source_database,
        &source_table,
        &accepted_schema,
        &directory.join("accepted-runs"),
        Some("(amount >= 0)"),
    );
    let output = run(&accepted, &directory, "target-expression").await;
    assert!(
        output.status.success(),
        "explicit CHECK expression failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let expression: String = target
        .client
        .query_one(
            "SELECT pg_catalog.pg_get_expr(con.conbin,con.conrelid) FROM pg_catalog.pg_constraint con JOIN pg_catalog.pg_class c ON c.oid=con.conrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname='rows' AND con.conname='amount_nonnegative' AND con.contype='c' AND con.convalidated",
            &[&accepted_schema],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(expression, "(amount >= 0)");
    let relation = postgres::qualified(&accepted_schema, "rows");
    let count: i64 = target
        .client
        .query_one(&format!("SELECT count(*) FROM {relation}"), &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
    let error = target
        .client
        .execute(&format!("INSERT INTO {relation} VALUES(2,-1)"), &[])
        .await
        .unwrap_err();
    assert_eq!(error.as_db_error().unwrap().code().code(), "23514");

    source
        .query_drop(format!("DROP TABLE {source_ident}"))
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE; DROP SCHEMA IF EXISTS {} CASCADE",
            my2pg::plan::quote_identifier(&accepted_schema),
            my2pg::plan::quote_identifier(&rejected_schema)
        ))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
