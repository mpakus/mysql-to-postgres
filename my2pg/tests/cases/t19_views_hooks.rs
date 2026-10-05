use my2pg::config::MigrationConfig;
use mysql_async::prelude::Queryable;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture variable {name}"))
}

fn workspace(label: &str) -> PathBuf {
    let path = env::temp_dir().join(format!("my2pg-t19-{label}-{}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn config(schema: &str, view: &str, directory: &Path) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":schema},
        "migration":{"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":[view],"views":[view]},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    }))
    .unwrap()
}

fn invoke(operation: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg(operation)
        .arg(path)
        .args(["--output", "json", "--progress", "never"])
        .output()
        .expect("run actual my2pg binary")
}

async fn target(schema: &str) -> my2pg::postgres::TargetConnection {
    let config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":schema
    }))
    .unwrap();
    my2pg::postgres::connect(&config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap()
}

async fn source() -> my2pg::mysql::SourceConnection {
    let config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"
    }))
    .unwrap();
    my2pg::mysql::connect(&config, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap()
}

async fn source_cleanup(source: &mut my2pg::mysql::SourceConnection, base: &str, view: &str) {
    source
        .query_drop(format!("DROP VIEW IF EXISTS `{view}`"))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE IF EXISTS `{base}`"))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn selected_view_is_snapshotted_and_hooks_bracket_only_the_run() {
    let suffix = std::process::id();
    let schema = format!("t19_hooks_{suffix}");
    let base = format!("t19_hook_base_{suffix}");
    let view = format!("t19_hook_view_{suffix}");
    let directory = workspace("success");
    let mut source = source().await;
    let target = target(&schema).await;
    source_cleanup(&mut source, &base, &view).await;
    target
        .client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE `{base}` (id INT PRIMARY KEY, value VARCHAR(30)) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO `{base}` VALUES (1,'alpha'),(2,'beta')"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE VIEW `{view}` AS SELECT id, UPPER(value) AS value FROM `{base}`"
        ))
        .await
        .unwrap();

    let before = directory.join("before.sql");
    let after = directory.join("after.sql");
    fs::write(
        &before,
        format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"; CREATE TABLE \"{schema}\".hook_order (position text); INSERT INTO \"{schema}\".hook_order VALUES ('before');"),
    )
    .unwrap();
    fs::write(
        &after,
        format!("INSERT INTO \"{schema}\".hook_order VALUES ('after');"),
    )
    .unwrap();
    let mut migration = config(&schema, &view, &directory);
    migration.hooks.before.push(before);
    migration.hooks.after.push(after);
    let path = directory.join("migration.toml");
    fs::write(&path, toml::to_string(&migration).unwrap()).unwrap();

    let planned = invoke("plan", &path);
    assert!(
        planned.status.success(),
        "plan failed: {}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let exists: bool = target
        .client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname=$1)",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists, "planning must not execute target SQL hooks");

    let output = invoke("run", &path);
    assert!(
        output.status.success(),
        "run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let values: Vec<(i32, String)> = target
        .client
        .query(
            &format!("SELECT id,value FROM \"{schema}\".\"{view}\" ORDER BY id"),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(values, [(1, "ALPHA".into()), (2, "BETA".into())]);
    let order: Vec<String> = target
        .client
        .query(
            &format!("SELECT position FROM \"{schema}\".hook_order ORDER BY ctid"),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(order, ["before", "after"]);
    let source_view_exists: bool = source
        .exec_first(
            "SELECT EXISTS (SELECT 1 FROM information_schema.views WHERE table_schema=DATABASE() AND table_name=?)",
            (&view,),
        )
        .await
        .unwrap()
        .unwrap();
    let source_rows: u64 = source
        .exec_first(format!("SELECT COUNT(*) FROM `{base}`"), ())
        .await
        .unwrap()
        .unwrap();
    assert!(
        source_view_exists,
        "migration must leave the source view intact"
    );
    assert_eq!(source_rows, 2, "migration must not mutate source rows");

    source_cleanup(&mut source, &base, &view).await;
    target
        .client
        .batch_execute(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .unwrap();
    target.close().await.unwrap();
    source.disconnect().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn after_hooks_are_suppressed_when_target_preparation_fails() {
    let suffix = std::process::id();
    let schema = format!("t19_failed_{suffix}");
    let base = format!("t19_failed_base_{suffix}");
    let view = format!("t19_failed_view_{suffix}");
    let directory = workspace("failure");
    let mut source = source().await;
    let target = target(&schema).await;
    source_cleanup(&mut source, &base, &view).await;
    target
        .client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE `{base}` (id INT PRIMARY KEY, value VARCHAR(30)) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("INSERT INTO `{base}` VALUES (1,'alpha')"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE VIEW `{view}` AS SELECT id, value FROM `{base}`"
        ))
        .await
        .unwrap();

    let before = directory.join("before.sql");
    let after = directory.join("after.sql");
    fs::write(
        &before,
        format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"; CREATE TABLE \"{schema}\".hook_order (position text); INSERT INTO \"{schema}\".hook_order VALUES ('before'); CREATE TABLE \"{schema}\".\"{view}\" (id integer, value text);"),
    )
    .unwrap();
    fs::write(
        &after,
        format!("INSERT INTO \"{schema}\".hook_order VALUES ('after');"),
    )
    .unwrap();
    let mut migration = config(&schema, &view, &directory);
    migration.hooks.before.push(before);
    migration.hooks.after.push(after);
    let path = directory.join("migration.toml");
    fs::write(&path, toml::to_string(&migration).unwrap()).unwrap();

    let output = invoke("run", &path);
    assert!(
        !output.status.success(),
        "preexisting relation must make target preparation fail"
    );
    let order: Vec<String> = target
        .client
        .query(
            &format!("SELECT position FROM \"{schema}\".hook_order ORDER BY ctid"),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(order, ["before"]);

    source_cleanup(&mut source, &base, &view).await;
    target
        .client
        .batch_execute(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .unwrap();
    target.close().await.unwrap();
    source.disconnect().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn after_hook_sql_failure_fails_an_otherwise_completed_load() {
    let suffix = std::process::id();
    let schema = format!("t19_hook_error_{suffix}");
    let base = format!("t19_error_base_{suffix}");
    let view = format!("t19_error_view_{suffix}");
    let directory = workspace("hook-error");
    let mut source = source().await;
    let target = target(&schema).await;
    source_cleanup(&mut source, &base, &view).await;
    target
        .client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE `{base}` (id INT PRIMARY KEY) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("INSERT INTO `{base}` VALUES (42)"))
        .await
        .unwrap();
    source
        .query_drop(format!("CREATE VIEW `{view}` AS SELECT id FROM `{base}`"))
        .await
        .unwrap();
    let after = directory.join("after.sql");
    fs::write(&after, "SELECT 1 / 0;").unwrap();
    let mut migration = config(&schema, &view, &directory);
    migration.hooks.after.push(after);
    let path = directory.join("migration.toml");
    fs::write(&path, toml::to_string(&migration).unwrap()).unwrap();

    let output = invoke("run", &path);
    assert!(
        !output.status.success(),
        "failing SQL hook must fail the run"
    );
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(diagnostic.contains("HOOK_EXECUTION"), "{diagnostic}");
    let copied: Option<i32> = target
        .client
        .query_opt(&format!("SELECT id FROM \"{schema}\".\"{view}\""), &[])
        .await
        .unwrap()
        .map(|row| row.get(0));
    assert_eq!(copied, Some(42), "hook failure follows the completed load");

    source_cleanup(&mut source, &base, &view).await;
    target
        .client
        .batch_execute(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .await
        .unwrap();
    target.close().await.unwrap();
    source.disconnect().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}
