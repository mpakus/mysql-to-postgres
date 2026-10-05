//! Native MySQL AUTO_INCREMENT integer mappings and unsupported-range refusal.
use my2pg::{config::MigrationConfig, postgres};
use mysql_async::prelude::Queryable;
use std::{env, path::Path, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}

fn config(schema: &str, source_tables: &[String], report_dir: &Path) -> MigrationConfig {
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
        "tables": {"include": source_tables},
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
async fn native_runner_preserves_supported_auto_increment_integer_mappings() {
    let suffix = std::process::id();
    let schema = format!("t11_identity_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t11-identity-{suffix}"));
    std::fs::create_dir_all(&directory).unwrap();
    let source_config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_MYSQL_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "consistency": "frozen"
    }))
    .unwrap();
    let mut source = my2pg::mysql::connect(&source_config, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "schema": schema,
        "ca_file": required("MY2PG_TLS_CA")
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();

    // Reset artifacts from an interrupted run of this same test process.
    target
        .client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .await
        .unwrap();

    struct Mapping {
        label: &'static str,
        mysql_type: &'static str,
        postgres_type: &'static str,
        values: &'static str,
        expected_ids: &'static [&'static str],
        next: &'static str,
    }
    let mappings = [
        Mapping {
            label: "tiny_signed",
            mysql_type: "TINYINT",
            postgres_type: "smallint",
            values: "(-128),(7)",
            expected_ids: &["-128", "7"],
            next: "42",
        },
        Mapping {
            label: "tiny_unsigned",
            mysql_type: "TINYINT UNSIGNED",
            postgres_type: "smallint",
            values: "(250)",
            expected_ids: &["250"],
            next: "251",
        },
        Mapping {
            label: "small_signed",
            mysql_type: "SMALLINT",
            postgres_type: "smallint",
            values: "(-32768),(12)",
            expected_ids: &["-32768", "12"],
            next: "42",
        },
        Mapping {
            label: "small_unsigned",
            mysql_type: "SMALLINT UNSIGNED",
            postgres_type: "integer",
            values: "(50000)",
            expected_ids: &["50000"],
            next: "50001",
        },
        Mapping {
            label: "medium_signed",
            mysql_type: "MEDIUMINT",
            postgres_type: "integer",
            values: "(-8388608),(100000)",
            expected_ids: &["-8388608", "100000"],
            next: "100001",
        },
        Mapping {
            label: "medium_unsigned",
            mysql_type: "MEDIUMINT UNSIGNED",
            postgres_type: "integer",
            values: "(10000000)",
            expected_ids: &["10000000"],
            next: "10000001",
        },
        Mapping {
            label: "int_signed",
            mysql_type: "INT",
            postgres_type: "integer",
            values: "(-2147483648),(1234567)",
            expected_ids: &["-2147483648", "1234567"],
            next: "1234568",
        },
        Mapping {
            label: "int_unsigned",
            mysql_type: "INT UNSIGNED",
            postgres_type: "bigint",
            values: "(3000000000)",
            expected_ids: &["3000000000"],
            next: "3000000001",
        },
        Mapping {
            label: "big_signed",
            mysql_type: "BIGINT",
            postgres_type: "bigint",
            values: "(-9223372036854775808),(7)",
            expected_ids: &["-9223372036854775808", "7"],
            next: "42",
        },
    ];
    let mut source_tables = Vec::new();
    let mut expected_values = Vec::new();
    for mapping in mappings {
        let name = format!("T11Identity{suffix}_{}", mapping.label);
        source
            .query_drop(format!("DROP TABLE IF EXISTS `{name}`"))
            .await
            .unwrap();
        source
            .query_drop(format!(
                "CREATE TABLE `{name}` (id {} NOT NULL AUTO_INCREMENT PRIMARY KEY)",
                mapping.mysql_type
            ))
            .await
            .unwrap();
        source
            .query_drop(format!(
                "INSERT INTO `{name}` (id) VALUES {}",
                mapping.values
            ))
            .await
            .unwrap();
        source
            .query_drop(format!(
                "ALTER TABLE `{name}` AUTO_INCREMENT={}",
                mapping.next
            ))
            .await
            .unwrap();
        // Values in this fixture are stable numeric literals; keep the reviewed
        // expectation independent of rows read back from either database.
        expected_values.push((
            name.clone(),
            mapping.postgres_type,
            mapping
                .expected_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        ));
        source_tables.push(name);
    }

    let migration = config(&schema, &source_tables, &directory);
    let output = run(&migration, &directory, "supported").await;
    assert!(
        output.status.success(),
        "native migration failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for ((table, expected_type, expected_ids), mapping) in expected_values.iter().zip([
        "42",
        "251",
        "42",
        "50001",
        "100001",
        "10000001",
        "1234568",
        "3000000001",
        "42",
    ]) {
        let relation = postgres::qualified(&schema, table);
        let observed = target
            .client
            .query_one(
                "SELECT pg_catalog.format_type(a.atttypid,a.atttypmod), a.attidentity = 'd' \
                   FROM pg_catalog.pg_class c \
                   JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                   JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid \
                   WHERE n.nspname=$1 AND c.relname=$2 AND a.attname='id' \
                     AND a.attnum>0 AND NOT a.attisdropped",
                &[&schema, table],
            )
            .await
            .unwrap();
        assert_eq!(observed.get::<_, String>(0), *expected_type, "{table}");
        assert!(
            observed.get::<_, bool>(1),
            "{table} must have BY DEFAULT identity"
        );
        let imported: Vec<String> = target
            .client
            .query(&format!("SELECT id::text FROM {relation} ORDER BY id"), &[])
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        let mut sorted_expected = expected_ids.clone();
        sorted_expected.sort_by(|left, right| {
            left.parse::<i128>()
                .unwrap()
                .cmp(&right.parse::<i128>().unwrap())
        });
        assert_eq!(imported, sorted_expected, "{table} imported IDs");
        let sequence: Option<String> = target
            .client
            .query_one(
                "SELECT pg_catalog.pg_get_serial_sequence($1,'id')",
                &[&format!("\"{schema}\".\"{table}\"")],
            )
            .await
            .unwrap()
            .get(0);
        let sequence = sequence.unwrap_or_else(|| panic!("{table} has no owned sequence"));
        let generated: String = target
            .client
            .query_one("SELECT nextval($1::text::regclass)::text", &[&sequence])
            .await
            .unwrap()
            .get(0);
        assert_eq!(generated, *mapping, "{table} next generated value");
    }

    for table in &source_tables {
        source
            .query_drop(format!("DROP TABLE `{table}`"))
            .await
            .unwrap();
    }
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
async fn native_runner_refuses_unsigned_bigint_auto_increment_before_target_ddl() {
    let suffix = std::process::id();
    let schema = format!("t11_identity_unsupported_{suffix}");
    let table = format!("T11IdentityUnsupported{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t11-identity-unsupported-{suffix}"));
    std::fs::create_dir_all(&directory).unwrap();
    let source_config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_MYSQL_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "consistency": "frozen"
    }))
    .unwrap();
    let mut source = my2pg::mysql::connect(&source_config, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "schema": schema,
        "ca_file": required("MY2PG_TLS_CA")
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
            "CREATE TABLE `{table}` (id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY)"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("INSERT INTO `{table}` VALUES (1)"))
        .await
        .unwrap();

    let migration = config(&schema, std::slice::from_ref(&table), &directory);
    let output = run(&migration, &directory, "unsigned-bigint-refusal").await;
    assert!(!output.status.success(), "unsupported identity must refuse");
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(diagnostic.contains("IDENTITY_RANGE"), "{diagnostic}");
    assert!(
        !has_schema(&target.client, &schema).await,
        "refusal must occur before target schema creation"
    );
    let rows: u64 = source
        .query_first(format!("SELECT COUNT(*) FROM `{table}`"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rows, 1, "refusal must not mutate the source");

    source
        .query_drop(format!("DROP TABLE `{table}`"))
        .await
        .unwrap();
    target.close().await.unwrap();
    source.disconnect().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
