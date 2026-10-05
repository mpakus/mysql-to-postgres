use clap::Parser;
use my2pg::{
    cli::Args,
    config::{MigrationConfig, ResolvedCredentials},
    model::{RunStatus, VerificationStatus},
    mysql, postgres,
    report::Console,
};
use mysql_async::prelude::Queryable;
use std::{env, path::PathBuf, process::Command};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing fixture variable {name}"))
}

fn config(schema: &str, table: &str, append: bool) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {"url_env":"MY2PG_MYSQL_URL", "consistency":"single_snapshot", "ca_file":required("MY2PG_TLS_CA")},
        "target": {"url_env":"MY2PG_POSTGRES_URL", "schema":schema, "ca_file":required("MY2PG_TLS_CA"), "on_existing":if append {"append"} else {"error"}},
        "migration": {"mode":if append {"data_only"} else {"full"}, "batch_rows":2, "batch_bytes":4096, "max_row_bytes":1024, "memory_bytes":1048576, "reset_sequences":false},
        "tables": {"include":[table]},
        "verification": {"mode":"content"},
        "report": {"directory":env::temp_dir().join(format!("my2pg-{schema}-{}", std::process::id())), "console":"json", "progress":"never"}
    }))
    .unwrap()
}

fn credentials(config: &MigrationConfig) -> ResolvedCredentials {
    my2pg::config::resolve_credentials(config).unwrap()
}

fn console(config: &MigrationConfig) -> Console {
    Console::new(
        config,
        &Args::try_parse_from(["my2pg", "run", "synthetic.toml", "--quiet"]).unwrap(),
    )
}

fn persisted_status(report: &my2pg::model::RunReport) -> RunStatus {
    serde_json::from_slice::<my2pg::model::RunReport>(
        &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap()
    .status
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn content_verification_is_reported_by_the_real_pipeline_and_append_includes_baseline() {
    let full = config("t18_runner_full", "t18_runner_full", false);
    let full_creds = credentials(&full);
    let mut source_admin = mysql::connect(
        &full.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        full.migration.max_row_bytes,
    )
    .await
    .unwrap();
    source_admin
        .query_drop("DROP TABLE IF EXISTS source.t18_runner_full")
        .await
        .unwrap();
    source_admin
        .query_drop("CREATE TABLE source.t18_runner_full(value VARCHAR(64) NOT NULL) ENGINE=InnoDB")
        .await
        .unwrap();
    source_admin
        .query_drop(
            "INSERT INTO source.t18_runner_full VALUES ('tail   '),('duplicate'),('duplicate')",
        )
        .await
        .unwrap();

    let target = postgres::connect(&full.target, full_creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t18_runner_full CASCADE")
        .await
        .unwrap();
    let (_, cancel) = watch::channel(false);
    let report = crate::test_pipeline::run(&full, &full_creds, &mut console(&full), cancel)
        .await
        .unwrap();
    assert_eq!(report.exit_code(), 0, "{report:?}");
    assert_eq!(
        report.verification.mode,
        my2pg::config::VerificationMode::Content
    );
    assert_eq!(report.verification.status, VerificationStatus::Complete);
    assert_eq!(report.tables[0].committed_rows, 3);
    assert_eq!(persisted_status(&report), RunStatus::Complete);
    let target_before: i64 = target
        .client
        .query_one("SELECT count(*) FROM t18_runner_full.t18_runner_full", &[])
        .await
        .unwrap()
        .get(0);
    let config_path = env::temp_dir().join(format!(
        "my2pg-t18-verify-{}-{}.toml",
        std::process::id(),
        report.run_id
    ));
    std::fs::write(&config_path, toml::to_string(&full).unwrap()).unwrap();
    let verified = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("verify")
        .arg(&config_path)
        .arg("--run-dir")
        .arg(&report.artifact_dir)
        .args(["--mode", "counts-and-schema", "--output", "json"])
        .output()
        .unwrap();
    assert_eq!(
        verified.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(document["operation"], "verify");
    assert_eq!(document["run_id"], report.run_id);
    assert_eq!(document["verification"]["status"], "complete");
    let wrong_endpoint = required("MY2PG_MYSQL_URL").replace("127.0.0.1", "127.0.0.2");
    let refused = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("verify")
        .arg(&config_path)
        .arg("--run-dir")
        .arg(&report.artifact_dir)
        .env("MY2PG_MYSQL_URL", wrong_endpoint)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("do not match this configuration"));

    let plan_path = PathBuf::from(&report.artifact_dir).join("plan.json");
    let saved_plan = std::fs::read(&plan_path).unwrap();
    let mut changed_plan: serde_json::Value = serde_json::from_slice(&saved_plan).unwrap();
    changed_plan["target_schema"] = "another_schema".into();
    std::fs::write(&plan_path, serde_json::to_vec(&changed_plan).unwrap()).unwrap();
    let mismatched = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("verify")
        .arg(&config_path)
        .arg("--run-dir")
        .arg(&report.artifact_dir)
        .args(["--mode", "counts_and_schema"])
        .output()
        .unwrap();
    assert_eq!(mismatched.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&mismatched.stderr).contains("do not match this configuration")
    );
    std::fs::write(&plan_path, saved_plan).unwrap();
    let target_after: i64 = target
        .client
        .query_one("SELECT count(*) FROM t18_runner_full.t18_runner_full", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        target_after, target_before,
        "verify must not write to PostgreSQL"
    );
    std::fs::remove_file(config_path).unwrap();
    let copied: Vec<String> = target
        .client
        .query(
            "SELECT value FROM t18_runner_full.t18_runner_full ORDER BY value",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(copied, ["duplicate", "duplicate", "tail   "]);
    target
        .client
        .batch_execute("DROP SCHEMA t18_runner_full CASCADE")
        .await
        .unwrap();
    source_admin
        .query_drop("DROP TABLE source.t18_runner_full")
        .await
        .unwrap();

    let append = config("t18_runner_append", "t18_runner_append", true);
    let append_creds = credentials(&append);
    source_admin
        .query_drop("DROP TABLE IF EXISTS source.t18_runner_append")
        .await
        .unwrap();
    source_admin
        .query_drop(
            "CREATE TABLE source.t18_runner_append(value VARCHAR(64) NOT NULL) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    source_admin
        .query_drop("INSERT INTO source.t18_runner_append VALUES ('incoming'),('incoming')")
        .await
        .unwrap();
    let append_target = postgres::connect(&append.target, append_creds.target.expose())
        .await
        .unwrap();
    append_target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t18_runner_append CASCADE; CREATE SCHEMA t18_runner_append; CREATE TABLE t18_runner_append.t18_runner_append(value varchar(64) NOT NULL); INSERT INTO t18_runner_append.t18_runner_append VALUES ('existing'),('existing')")
        .await
        .unwrap();
    let (_, cancel) = watch::channel(false);
    let appended = crate::test_pipeline::run(&append, &append_creds, &mut console(&append), cancel)
        .await
        .unwrap();
    assert_eq!(appended.exit_code(), 0, "{appended:?}");
    assert_eq!(appended.verification.status, VerificationStatus::Complete);
    assert_eq!(appended.tables[0].committed_rows, 2);
    assert_eq!(persisted_status(&appended), RunStatus::Complete);
    let append_config_path = env::temp_dir().join(format!(
        "my2pg-t18-verify-append-{}-{}.toml",
        std::process::id(),
        appended.run_id
    ));
    std::fs::write(&append_config_path, toml::to_string(&append).unwrap()).unwrap();
    let append_verification = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("verify")
        .arg(&append_config_path)
        .arg("--run-dir")
        .arg(&appended.artifact_dir)
        .args(["--mode", "counts_and_schema", "--output", "json"])
        .output()
        .unwrap();
    assert_eq!(append_verification.status.code(), Some(4));
    let append_document: serde_json::Value =
        serde_json::from_slice(&append_verification.stdout).unwrap();
    assert_eq!(append_document["verification"]["status"], "unsupported");
    std::fs::remove_file(append_config_path).unwrap();
    let all_rows: Vec<String> = append_target
        .client
        .query(
            "SELECT value FROM t18_runner_append.t18_runner_append ORDER BY value",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(all_rows, ["existing", "existing", "incoming", "incoming"]);
    append_target
        .client
        .batch_execute("DROP SCHEMA t18_runner_append CASCADE")
        .await
        .unwrap();
    source_admin
        .query_drop("DROP TABLE source.t18_runner_append")
        .await
        .unwrap();
    source_admin.disconnect().await.unwrap();
    target.close().await.unwrap();
    append_target.close().await.unwrap();
}
