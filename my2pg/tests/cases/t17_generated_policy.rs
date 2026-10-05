//! Generated MySQL columns require a PostgreSQL expression or explicit materialization.
use my2pg::{config::MigrationConfig, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::Path};
use tokio::process::Command;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

#[derive(Clone, Copy)]
enum GeneratedPolicy<'a> {
    Missing,
    TargetExpression(&'a str),
    Materialize,
}

fn config(
    source_database: &str,
    source_table: &str,
    target_schema: &str,
    report_dir: &Path,
    mode: &str,
    policy: &str,
    generated_policy: GeneratedPolicy<'_>,
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
            "schema": target_schema,
            "on_existing": policy
        },
        "migration": {
            "mode": mode,
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
    match generated_policy {
        GeneratedPolicy::Missing => {}
        GeneratedPolicy::TargetExpression(target_expression) => {
            config.overrides.push(my2pg::config::ObjectOverride {
                object: format!("{source_database}.{source_table}.derived"),
                omit: false,
                target_expression: Some(target_expression.into()),
                target_sql: None,
                materialize: None,
            });
        }
        GeneratedPolicy::Materialize => {
            config.overrides.push(my2pg::config::ObjectOverride {
                object: format!("{source_database}.{source_table}.derived"),
                omit: false,
                target_expression: None,
                target_sql: None,
                materialize: Some(true),
            });
        }
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

async fn generated_kind(client: &tokio_postgres::Client, schema: &str) -> String {
    client
        .query_one(
            "SELECT a.attgenerated::text FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname='rows' AND a.attname='derived'",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn native_generated_columns_require_expression_or_materialization_and_match_existing_targets()
{
    let suffix = std::process::id();
    let source_table = format!("T17Generated{suffix}");
    let expression_schema = format!("t17_generated_expr_{suffix}");
    let materialized_schema = format!("t17_generated_values_{suffix}");
    let rejected_schema = format!("t17_generated_rejected_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t17-generated-{suffix}"));
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
            "CREATE TABLE {source_ident}(id INT NOT NULL, source_text VARCHAR(32) NOT NULL, derived VARCHAR(32) GENERATED ALWAYS AS (LOWER(source_text)) STORED) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {source_ident}(id,source_text) VALUES(1,'MySQL Value')"
        ))
        .await
        .unwrap();
    let expected_source_value: String = source
        .query_first(format!("SELECT derived FROM {source_ident} WHERE id=1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expected_source_value, "mysql value");
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let source_database = catalog.database.clone();
    let generated = catalog
        .tables
        .iter()
        .find(|table| table.name == source_table)
        .unwrap()
        .columns
        .iter()
        .find(|column| column.name == "derived")
        .unwrap();
    assert!(
        generated
            .generation_expression
            .as_deref()
            .is_some_and(|expression| expression.to_ascii_lowercase().contains("source_text"))
    );

    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "schema": rejected_schema
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    for schema in [&expression_schema, &materialized_schema, &rejected_schema] {
        assert!(!schema_exists(&target.client, schema).await);
    }

    let rejected = config(
        &source_database,
        &source_table,
        &rejected_schema,
        &directory.join("rejected-runs"),
        "full",
        "error",
        GeneratedPolicy::Missing,
    );
    let output = run(&rejected, &directory, "missing-policy").await;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "missing generated policy succeeded"
    );
    assert!(diagnostics.contains("GENERATED_POLICY"), "{diagnostics}");
    assert!(
        !schema_exists(&target.client, &rejected_schema).await,
        "missing policy must block before target schema creation"
    );

    // pg_get_expr renders the varchar-to-text coercion inserted for lower(text).
    let expression = "lower((source_text)::text)";
    let accepted = config(
        &source_database,
        &source_table,
        &expression_schema,
        &directory.join("expression-runs"),
        "full",
        "error",
        GeneratedPolicy::TargetExpression(expression),
    );
    let output = run(&accepted, &directory, "target-expression").await;
    assert!(
        output.status.success(),
        "target-expression run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        generated_kind(&target.client, &expression_schema).await,
        "s"
    );
    let expression_relation = postgres::qualified(&expression_schema, "rows");
    let computed_row = target
        .client
        .query_one(
            &format!(
                "SELECT source_text,derived,derived=lower(source_text) FROM {expression_relation}"
            ),
            &[],
        )
        .await
        .unwrap();
    let computed = (
        computed_row.get::<_, String>(0),
        computed_row.get::<_, String>(1),
        computed_row.get::<_, bool>(2),
    );
    assert_eq!(computed, ("MySQL Value".into(), "mysql value".into(), true));

    for (label, policy, expected_count) in
        [("append", "append", 2_i64), ("truncate", "truncate", 1_i64)]
    {
        let data_only = config(
            &source_database,
            &source_table,
            &expression_schema,
            &directory.join(format!("data-only-{label}-runs")),
            "data_only",
            policy,
            GeneratedPolicy::TargetExpression(expression),
        );
        let output = run(&data_only, &directory, &format!("data-only-{label}")).await;
        assert!(
            output.status.success(),
            "data-only {label} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            generated_kind(&target.client, &expression_schema).await,
            "s"
        );
        let rows = target
            .client
            .query_one(
                &format!(
                    "SELECT count(*) FROM {expression_relation} WHERE derived=lower(source_text)"
                ),
                &[],
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert_eq!(
            rows, expected_count,
            "DataOnly {label} independently recomputes expression"
        );
    }

    let materialized = config(
        &source_database,
        &source_table,
        &materialized_schema,
        &directory.join("materialized-runs"),
        "full",
        "error",
        GeneratedPolicy::Materialize,
    );
    let output = run(&materialized, &directory, "materialize").await;
    assert!(
        output.status.success(),
        "materialization run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        generated_kind(&target.client, &materialized_schema).await,
        ""
    );
    let materialized_relation = postgres::qualified(&materialized_schema, "rows");
    let copied_row = target
        .client
        .query_one(
            &format!(
                "SELECT source_text,derived,derived=lower(source_text) FROM {materialized_relation}"
            ),
            &[],
        )
        .await
        .unwrap();
    let copied = (
        copied_row.get::<_, String>(0),
        copied_row.get::<_, String>(1),
        copied_row.get::<_, bool>(2),
    );
    assert_eq!(copied, ("MySQL Value".into(), expected_source_value, true));

    let schemas = [expression_schema, materialized_schema, rejected_schema];
    let drop_schemas = schemas
        .iter()
        .map(|schema| {
            format!(
                "DROP SCHEMA IF EXISTS {} CASCADE",
                postgres::quote_ident(schema)
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    target.client.batch_execute(&drop_schemas).await.unwrap();
    source
        .query_drop(format!("DROP TABLE {source_ident}"))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
