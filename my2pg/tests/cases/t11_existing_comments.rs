use my2pg::{config::MigrationConfig, mysql, plan, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}

async fn run(config: &MigrationConfig, path: &std::path::Path) -> std::process::Output {
    fs::write(path, toml::to_string(config).unwrap()).unwrap();
    let path = path.to_owned();
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
#[ignore = "requires owned MySQL8.4/PostgreSQL16 TLS fixtures"]
async fn native_existing_append_rejects_comment_drift_before_copy_or_mutation() {
    let directory = env::temp_dir().join(format!("my2pg-t11-comments-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":"t11_comment_drift","on_existing":"append"},
        "migration":{"mode":"data_only","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":["T11CommentDrift"],"rename":[{"source":"T11CommentDrift","target":"comment_drift"}]},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap();

    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE IF EXISTS T11CommentDrift")
        .await
        .unwrap();
    source
        .query_drop("CREATE TABLE T11CommentDrift(id INT NOT NULL PRIMARY KEY,note VARCHAR(24) COMMENT 'source column comment') COMMENT='source table comment'")
        .await
        .unwrap();
    source
        .query_drop("INSERT INTO T11CommentDrift VALUES(1,'incoming')")
        .await
        .unwrap();
    let source_catalog = mysql::inspect(&mut source).await.unwrap();

    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t11_comment_drift CASCADE;CREATE SCHEMA t11_comment_drift;CREATE TABLE t11_comment_drift.comment_drift(id integer NOT NULL PRIMARY KEY,note varchar(24));COMMENT ON TABLE t11_comment_drift.comment_drift IS 'target table comment';COMMENT ON COLUMN t11_comment_drift.comment_drift.note IS 'target column comment';INSERT INTO t11_comment_drift.comment_drift VALUES(99,'sentinel')")
        .await
        .unwrap();
    let before = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();

    let plan::PlanError::Blocked(diagnostics) = plan::build(&config, &source_catalog, &before)
        .expect_err("Append must refuse source/target comment drift")
    else {
        panic!("comment drift must be a positive planning rejection")
    };
    let comment_errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "TARGET_COMMENT_INCOMPATIBLE")
        .count();
    assert_eq!(
        comment_errors, 2,
        "both table and column comments are checked"
    );

    let output = run(&config, &directory.join("append.toml")).await;
    let diagnostic = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "Append must refuse comment drift: {diagnostic}"
    );
    assert!(
        diagnostic.contains("TARGET_COMMENT_INCOMPATIBLE"),
        "{diagnostic}"
    );

    let rows = target
        .client
        .query(
            "SELECT id,note FROM t11_comment_drift.comment_drift ORDER BY id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "blocked planning must not COPY source rows");
    assert_eq!(rows[0].get::<_, i32>(0), 99);
    assert_eq!(rows[0].get::<_, String>(1), "sentinel");
    let comments = target
        .client
        .query_one(
            "SELECT obj_description('t11_comment_drift.comment_drift'::regclass,'pg_class'),col_description('t11_comment_drift.comment_drift'::regclass,(SELECT attnum FROM pg_attribute WHERE attrelid='t11_comment_drift.comment_drift'::regclass AND attname='note'))",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        comments.get::<_, Option<String>>(0).as_deref(),
        Some("target table comment")
    );
    assert_eq!(
        comments.get::<_, Option<String>>(1).as_deref(),
        Some("target column comment")
    );

    target
        .client
        .batch_execute("DROP SCHEMA t11_comment_drift CASCADE")
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE T11CommentDrift")
        .await
        .unwrap();
    target.close().await.unwrap();
}
