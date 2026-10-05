//! Schema-only recreate replaces a selected target without copying rows or touching a sentinel.
use my2pg::{config::MigrationConfig, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::Path, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

fn config(schema: &str, source_table: &str, report_dir: &Path) -> MigrationConfig {
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
            "schema": schema,
            "on_existing": "recreate"
        },
        "migration": {
            "mode": "schema_only",
            "reset_sequences": true,
            "batch_rows": 2,
            "batch_bytes": 4096,
            "max_row_bytes": 1024,
            "memory_bytes": 262144
        },
        "tables": {
            "include": [source_table],
            "rename": [{"source": source_table, "target": "selected"}]
        },
        "report": {
            "directory": report_dir,
            "console": "json",
            "progress": "never"
        }
    }))
    .unwrap()
}

async fn run(config: &MigrationConfig, directory: &Path) -> std::process::Output {
    let path = directory.join("schema-only-recreate.toml");
    fs::write(&path, toml::to_string(config).unwrap()).unwrap();
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

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn native_schema_only_recreate_replaces_selected_table_without_copying_or_touching_sentinel()
{
    let suffix = std::process::id();
    let schema = format!("t11_schema_recreate_{suffix}");
    let outside_schema = format!("t11_schema_recreate_outside_{suffix}");
    let source_table = format!("T11SchemaRecreate{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t11-schema-recreate-{suffix}"));
    fs::create_dir_all(&directory).unwrap();

    let source_config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_MYSQL_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "consistency": "frozen"
    }))
    .unwrap();
    let mut source = mysql::connect(&source_config, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    let source_ident = mysql::quote_ident(&source_table);
    source
        .query_drop(format!("DROP TABLE IF EXISTS {source_ident}"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE {source_ident}(id INT NOT NULL AUTO_INCREMENT PRIMARY KEY, value INT NOT NULL, UNIQUE KEY source_value(value)) ENGINE=InnoDB AUTO_INCREMENT=42"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {source_ident}(id,value) VALUES(1,11),(2,22)"
        ))
        .await
        .unwrap();
    let source_catalog = mysql::inspect(&mut source).await.unwrap();
    assert_eq!(
        source_catalog
            .tables
            .iter()
            .find(|table| table.name == source_table)
            .unwrap()
            .next_auto_increment,
        Some(42),
        "the source fixture must expose its chosen next identity"
    );

    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "schema": schema
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let target_ns = postgres::quote_ident(&schema);
    let outside_ns = postgres::quote_ident(&outside_schema);
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {target_ns} CASCADE; DROP SCHEMA IF EXISTS {outside_ns} CASCADE; CREATE SCHEMA {target_ns}; CREATE SCHEMA {outside_ns}; CREATE TABLE {target_ns}.selected(id text, value text, obsolete text, CONSTRAINT obsolete_check CHECK(obsolete IS NOT NULL)); CREATE INDEX obsolete_index ON {target_ns}.selected(value); INSERT INTO {target_ns}.selected VALUES('old-id','old-value','old'); CREATE TABLE {outside_ns}.sentinel(id integer PRIMARY KEY, payload text NOT NULL); INSERT INTO {outside_ns}.sentinel VALUES(7,'keep me')"
        ))
        .await
        .unwrap();
    let old_oid: u32 = target
        .client
        .query_one(
            "SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname='selected'",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0);
    let sentinel_oid: u32 = target
        .client
        .query_one(
            "SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname='sentinel'",
            &[&outside_schema],
        )
        .await
        .unwrap()
        .get(0);

    let migration = config(&schema, &source_table, &directory.join("runs"));
    let output = run(&migration, &directory).await;
    assert!(
        output.status.success(),
        "schema-only recreate failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let selected_oid: u32 = target
        .client
        .query_one(
            "SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname='selected'",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0);
    assert_ne!(selected_oid, old_oid, "selected relation must be recreated");
    let relation = postgres::qualified(&schema, "selected");
    let columns = target
        .client
        .query(
            "SELECT a.attname::text, pg_catalog.format_type(a.atttypid,a.atttypmod), a.attidentity::text, a.attnotnull FROM pg_catalog.pg_attribute a WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",
            &[&selected_oid],
        )
        .await
        .unwrap();
    assert_eq!(columns.len(), 2, "obsolete target column must be gone");
    assert_eq!(
        (
            columns[0].get::<_, String>(0),
            columns[0].get::<_, String>(1),
            columns[0].get::<_, String>(2),
            columns[0].get::<_, bool>(3)
        ),
        ("id".into(), "integer".into(), "d".into(), true)
    );
    assert_eq!(
        (
            columns[1].get::<_, String>(0),
            columns[1].get::<_, String>(1),
            columns[1].get::<_, String>(2),
            columns[1].get::<_, bool>(3)
        ),
        ("value".into(), "integer".into(), "".into(), true)
    );
    let unique_indexes = target
        .client
        .query(
            "SELECT i.indisprimary, i.indisunique, ARRAY(SELECT a.attname::text FROM unnest(i.indkey) WITH ORDINALITY k(attnum,ord) JOIN pg_catalog.pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.attnum ORDER BY k.ord) FROM pg_catalog.pg_index i WHERE i.indrelid=$1 AND i.indisunique ORDER BY i.indisprimary DESC",
            &[&selected_oid],
        )
        .await
        .unwrap();
    assert_eq!(
        unique_indexes.len(),
        2,
        "expected recreated primary and source unique indexes"
    );
    assert!(unique_indexes[0].get::<_, bool>(0));
    assert!(unique_indexes[0].get::<_, bool>(1));
    assert_eq!(unique_indexes[0].get::<_, Vec<String>>(2), ["id"]);
    assert!(!unique_indexes[1].get::<_, bool>(0));
    assert!(unique_indexes[1].get::<_, bool>(1));
    assert_eq!(unique_indexes[1].get::<_, Vec<String>>(2), ["value"]);
    assert_eq!(
        target
            .client
            .query_one(&format!("SELECT count(*) FROM {relation}"), &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0,
        "schema-only recreate must not copy source rows"
    );
    let relation_name = format!("{schema}.selected");
    let sequence: Option<String> = target
        .client
        .query_one(
            "SELECT pg_catalog.pg_get_serial_sequence($1,'id')",
            &[&relation_name],
        )
        .await
        .unwrap()
        .get(0);
    let sequence = sequence.expect("recreated identity must own a sequence");
    let next_value: i64 = target
        .client
        .query_one("SELECT nextval($1::text::regclass)", &[&sequence])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        next_value, 42,
        "schema-only must initialize identity from the source next value"
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT count(*) FROM pg_catalog.pg_constraint WHERE conrelid=$1 AND conname='obsolete_check'",
                &[&selected_oid],
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0,
        "old target constraint must not survive recreation"
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname='obsolete_index'",
                &[&schema],
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0,
        "old target index must not survive recreation"
    );
    let sentinel = target
        .client
        .query_one(
            &format!("SELECT c.oid, s.id, s.payload FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN {outside_ns}.sentinel s ON true WHERE n.nspname=$1 AND c.relname='sentinel' AND s.id=7"),
            &[&outside_schema],
        )
        .await
        .unwrap();
    assert_eq!(sentinel.get::<_, u32>(0), sentinel_oid);
    assert_eq!(sentinel.get::<_, i32>(1), 7);
    assert_eq!(sentinel.get::<_, String>(2), "keep me");

    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {target_ns} CASCADE; DROP SCHEMA {outside_ns} CASCADE"
        ))
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
