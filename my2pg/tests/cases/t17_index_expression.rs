//! Functional MySQL indexes require an explicit PostgreSQL expression override.
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
            object: format!("{source_database}.{source_table}.email_lower_unique"),
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
#[ignore = "requires owned MySQL 8/PostgreSQL TLS fixtures"]
async fn native_functional_index_requires_override_and_preserves_unique_behavior() {
    let suffix = std::process::id();
    let source_table = format!("T17FunctionalIndex{suffix}");
    let rejected_schema = format!("t17_index_rejected_{suffix}");
    let accepted_schema = format!("t17_index_accepted_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t17-index-{suffix}"));
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
            "CREATE TABLE {source_ident}(id INT NOT NULL PRIMARY KEY, email VARCHAR(100) NOT NULL) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {source_ident}(id,email) VALUES(1,'alice@example.com'),(2,'bob@example.com')"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE UNIQUE INDEX email_lower_unique ON {source_ident} ((LOWER(email)))"
        ))
        .await
        .unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let source_database = catalog.database.clone();
    let source_index = catalog
        .tables
        .iter()
        .find(|table| table.name == source_table)
        .unwrap()
        .indexes
        .iter()
        .find(|index| index.name == "email_lower_unique")
        .unwrap();
    assert!(source_index.unique);
    assert_eq!(source_index.parts.len(), 1);
    assert!(source_index.parts[0].expression.is_some());

    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "schema": accepted_schema
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    for schema in [&rejected_schema, &accepted_schema] {
        assert!(!schema_exists(&target.client, schema).await);
    }

    let missing = config(
        &source_database,
        &source_table,
        &rejected_schema,
        &directory.join("rejected-runs"),
        None,
    );
    let output = run(&missing, &directory, "missing-policy").await;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "missing index policy succeeded");
    assert!(
        diagnostics.contains("INDEX_EXPRESSION_POLICY"),
        "{diagnostics}"
    );
    assert!(!schema_exists(&target.client, &rejected_schema).await);

    let accepted = config(
        &source_database,
        &source_table,
        &accepted_schema,
        &directory.join("accepted-runs"),
        Some("lower((email)::text)"),
    );
    let output = run(&accepted, &directory, "explicit-expression").await;
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let index = target
        .client
        .query_one(
            "SELECT pg_catalog.pg_get_indexdef(i.indexrelid) FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class t ON t.oid=i.indrelid JOIN pg_catalog.pg_namespace n ON n.oid=t.relnamespace WHERE n.nspname=$1 AND t.relname='rows' AND pg_catalog.pg_get_indexdef(i.indexrelid) LIKE '%lower(%'",
            &[&accepted_schema],
        )
        .await
        .unwrap();
    let definition: String = index.get(0);
    assert!(
        definition.starts_with("CREATE UNIQUE INDEX"),
        "{definition}"
    );
    let error = target
        .client
        .execute(
            &format!(
                "INSERT INTO {}.rows(id,email) VALUES(3,'ALICE@EXAMPLE.COM')",
                my2pg::postgres::quote_ident(&accepted_schema)
            ),
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23505");

    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            my2pg::postgres::quote_ident(&accepted_schema)
        ))
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            my2pg::postgres::quote_ident(&rejected_schema)
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE IF EXISTS {source_ident}"))
        .await
        .unwrap();
    drop(source);
    drop(target);
    fs::remove_dir_all(directory).unwrap();
}
