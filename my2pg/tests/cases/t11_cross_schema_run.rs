use my2pg::{config::MigrationConfig, mysql, postgres};
use mysql_async::prelude::Queryable;
use sha2::{Digest, Sha256};
use std::{env, fs, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}

fn lock_key(schema: &str) -> i64 {
    let digest = Sha256::digest(format!("my2pg:v1:{schema}").as_bytes());
    i64::from_be_bytes(digest[..8].try_into().unwrap())
}

async fn run(config: &MigrationConfig, path: &std::path::Path) -> std::process::Output {
    fs::write(path, toml::to_string(config).unwrap()).unwrap();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .args([
                "run",
                path.to_str().unwrap(),
                "--output",
                "json",
                "--progress",
                "never",
            ])
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4/PostgreSQL16 TLS fixtures"]
async fn native_run_migrates_mapped_schemas_and_refuses_partial_lock_set_before_writes() {
    let suffix = std::process::id();
    let base_schema = format!("t11_cross_run_a_{suffix}");
    let mapped_schema = format!("t11_cross_run_z_{suffix}");
    let parent_source = format!("T11CrossRunParent{suffix}");
    let child_source = format!("T11CrossRunChild{suffix}");
    let foreign_key = format!("cross_run_fk_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t11-cross-run-{suffix}"));
    fs::create_dir(&directory).unwrap();
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":base_schema,"on_existing":"error"},
        "migration":{"mode":"full","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":[parent_source,child_source],"rename":[
            {"source":parent_source,"target":"same"},
            {"source":child_source,"target":"same","schema":mapped_schema}
        ]},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE IF EXISTS `{child_source}`"))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE IF EXISTS `{parent_source}`"))
        .await
        .unwrap();
    source.query_drop(format!(
        "CREATE TABLE `{parent_source}`(id INT NOT NULL PRIMARY KEY, value VARCHAR(32) NOT NULL)"
    )).await.unwrap();
    source.query_drop(format!(
        "CREATE TABLE `{child_source}`(id INT NOT NULL PRIMARY KEY, parent_id INT NOT NULL, CONSTRAINT `{foreign_key}` FOREIGN KEY(parent_id) REFERENCES `{parent_source}`(id))"
    )).await.unwrap();
    source
        .query_drop(format!("INSERT INTO `{parent_source}` VALUES(1,'parent')"))
        .await
        .unwrap();
    source
        .query_drop(format!("INSERT INTO `{child_source}` VALUES(2,1)"))
        .await
        .unwrap();

    let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let lock_value = lock_key(&mapped_schema);
    let acquired: bool = target
        .client
        .query_one("SELECT pg_try_advisory_lock($1)", &[&lock_value])
        .await
        .unwrap()
        .get(0);
    assert!(acquired);

    let blocked = run(&config, &directory.join("blocked.toml")).await;
    assert_ne!(
        blocked.status.code(),
        Some(0),
        "a lock conflict must refuse the run"
    );
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(
        stderr.contains("advisory lock"),
        "unexpected refusal: {stderr}"
    );
    for schema in [&base_schema, &mapped_schema] {
        let exists: bool = target
            .client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname=$1)",
                &[schema],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!exists, "lock refusal must not create schema {schema}");
    }
    // The runner acquired the lexicographically earlier base-schema lock before
    // discovering the blocked mapped lock. It must release that partial set.
    let base_lock = lock_key(&base_schema);
    let base_available: bool = target
        .client
        .query_one("SELECT pg_try_advisory_lock($1)", &[&base_lock])
        .await
        .unwrap()
        .get(0);
    assert!(
        base_available,
        "partial lock acquisition leaked after refusal"
    );
    let unlocked: bool = target
        .client
        .query_one("SELECT pg_advisory_unlock($1)", &[&base_lock])
        .await
        .unwrap()
        .get(0);
    assert!(unlocked);
    let unlocked: bool = target
        .client
        .query_one("SELECT pg_advisory_unlock($1)", &[&lock_value])
        .await
        .unwrap()
        .get(0);
    assert!(unlocked);

    config.report.directory = directory.join("successful-runs");
    let succeeded = run(&config, &directory.join("successful.toml")).await;
    assert_eq!(
        succeeded.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&succeeded.stderr)
    );
    assert_eq!(
        target
            .client
            .query_one(
                &format!(
                    "SELECT value FROM {}.same",
                    postgres::quote_ident(&base_schema)
                ),
                &[]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "parent"
    );
    assert_eq!(
        target
            .client
            .query_one(
                &format!(
                    "SELECT parent_id FROM {}.same",
                    postgres::quote_ident(&mapped_schema)
                ),
                &[]
            )
            .await
            .unwrap()
            .get::<_, i32>(0),
        1
    );
    assert!(
        target
            .client
            .batch_execute(&format!(
                "INSERT INTO {}.same VALUES(3,999)",
                postgres::quote_ident(&mapped_schema)
            ))
            .await
            .is_err(),
        "the migrated cross-schema FK must remain enforced"
    );
    assert_eq!(
        target
            .client
            .query_one(
                &format!(
                    "SELECT (SELECT count(*) FROM {}.same)+(SELECT count(*) FROM {}.same)",
                    postgres::quote_ident(&base_schema),
                    postgres::quote_ident(&mapped_schema)
                ),
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        2,
        "same-named relations in separate schemas remain distinct"
    );

    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE; DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&base_schema),
            postgres::quote_ident(&mapped_schema)
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE `{child_source}`"))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE `{parent_source}`"))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}
