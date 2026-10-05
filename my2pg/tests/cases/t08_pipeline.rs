use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, pipeline, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{env, path::PathBuf, time::Duration};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing fixture variable {name}"))
}
fn config(schema: &str, tables: &[&str]) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":required("MY2PG_TLS_CA")},
        "migration":{"batch_rows":1,"batch_bytes":1024,"max_row_bytes":1024,"memory_bytes":1048576},
        "tables":{"include":tables},
        "report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t08"),"progress":"never"}
    })).unwrap()
}
fn credentials(config: &MigrationConfig) -> ResolvedCredentials {
    resolve_credentials(config).unwrap()
}
fn console(config: &MigrationConfig) -> Console {
    Console::new(
        config,
        &Args::try_parse_from(["my2pg", "run", "synthetic.toml", "--quiet"]).unwrap(),
    )
}
async fn count(target: &postgres::TargetConnection, schema: &str, table: &str) -> i64 {
    target
        .client
        .query_one(
            &format!(
                "SELECT count(*) FROM {}",
                postgres::qualified(schema, table)
            ),
            &[],
        )
        .await
        .unwrap()
        .get(0)
}
fn persisted(report: &RunReport) -> RunReport {
    serde_json::from_slice(
        &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn readonly_plan_success_schema_only_and_preflight_gates() {
    let mut config = config("t08_success", &["all_bytes", "keyless", "CamelCase"]);
    let creds = credentials(&config);
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_success CASCADE")
        .await
        .unwrap();
    let inventory = pipeline::inspect(&config, &creds).await.unwrap();
    assert!(
        inventory
            .source
            .tables
            .iter()
            .any(|t| t.name == "all_bytes")
    );
    let plan = pipeline::plan(&config, &creds).await.unwrap();
    assert_eq!(plan.tables.len(), 3);
    let exists: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='t08_success')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists, "inspect/plan must not create a schema");
    let (_, receiver) = watch::channel(false);
    let report = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(report.exit_code(), 0, "{:?}", report.verification);
    assert_eq!(
        report.tables.iter().map(|t| t.committed_rows).sum::<u64>(),
        7
    );
    assert_eq!(persisted(&report).exit_code(), 0);
    let bytes: Vec<u8> = target
        .client
        .query_one("SELECT value FROM t08_success.all_bytes WHERE id=1", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(bytes, (0..=255).collect::<Vec<_>>());
    assert_eq!(count(&target, "t08_success", "keyless").await, 5);
    let sentinel: String = target
        .client
        .query_one("SELECT sentinel FROM public.users WHERE id=999", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sentinel, "must-survive");
    target
        .client
        .batch_execute("DROP SCHEMA t08_success CASCADE")
        .await
        .unwrap();
    config.migration.mode = MigrationMode::SchemaOnly;
    let (_, receiver) = watch::channel(false);
    let schema = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(schema.exit_code(), 0, "{:?}", schema.verification);
    assert!(
        schema
            .tables
            .iter()
            .all(|t| t.rows_read == 0 && t.committed_rows == 0)
    );
    target
        .client
        .batch_execute("DROP SCHEMA t08_success CASCADE")
        .await
        .unwrap();
    config.migration.table_workers = 2;
    config.source.consistency = Consistency::Frozen;
    config.migration.mode = MigrationMode::Full;
    let (_, receiver) = watch::channel(false);
    let parallel = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(parallel.exit_code(), 0, "{parallel:?}");
    assert_eq!(
        parallel
            .tables
            .iter()
            .map(|table| table.committed_rows)
            .sum::<u64>(),
        7
    );
    assert_eq!(persisted(&parallel).exit_code(), 0);
    assert_eq!(count(&target, "t08_success", "keyless").await, 5);
    target
        .client
        .batch_execute("DROP SCHEMA t08_success CASCADE")
        .await
        .unwrap();
    config.migration.table_workers = 1;
    config.migration.mode = MigrationMode::Full;
    config.migration.memory_bytes = 8192;
    let (_, receiver) = watch::channel(false);
    let error = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("memory_bytes"));
    let exists: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='t08_success')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists, "underfunded data run must fail before target DDL");
    config.migration.mode = MigrationMode::SchemaOnly;
    // Schema-only does not need row-pipeline bytes, but the durable artifact
    // worker still reserves its fixed buffers for the run lifetime.
    config.migration.memory_bytes = 1_048_576;
    let (_, receiver) = watch::channel(false);
    let schema = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(schema.exit_code(), 0);
    assert_eq!(persisted(&schema).exit_code(), 0);
    assert!(
        schema
            .tables
            .iter()
            .all(|table| table.rows_read == 0 && table.committed_rows == 0)
    );
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn conversion_failure_preserves_acknowledged_batches_and_artifacts() {
    let config = config("t08_failure", &["t08_a_prefix", "t08_z_failure"]);
    let creds = credentials(&config);
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t08_a_prefix,source.t08_z_failure")
        .await
        .unwrap();
    admin
        .query_drop(
            "CREATE TABLE source.t08_a_prefix(id INT PRIMARY KEY, value TEXT) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    admin
        .query_drop("INSERT INTO source.t08_a_prefix VALUES(1,'valid')")
        .await
        .unwrap();
    admin
        .query_drop(
            "CREATE TABLE source.t08_z_failure(id INT PRIMARY KEY, value TEXT) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    admin
        .query_drop("INSERT INTO source.t08_z_failure VALUES(2,CONCAT('invalid',CHAR(0)))")
        .await
        .unwrap();
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_failure CASCADE")
        .await
        .unwrap();
    let (_, receiver) = watch::channel(false);
    let report = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(report.exit_code(), 1);
    let table = |name: &str| {
        report
            .tables
            .iter()
            .find(|table| table.source_name == name)
            .unwrap()
    };
    let prefix = table("t08_a_prefix");
    assert_eq!(
        (
            prefix.rows_read,
            prefix.committed_rows,
            prefix.unresolved_rows
        ),
        (1, 1, 0)
    );
    assert_eq!(prefix.accounted_rows(), Some(1));
    let failed = table("t08_z_failure");
    assert_eq!(
        (
            failed.rows_read,
            failed.committed_rows,
            failed.unresolved_rows
        ),
        (1, 0, 1)
    );
    assert_eq!(failed.accounted_rows(), Some(1));
    assert_eq!(count(&target, "t08_failure", "t08_a_prefix").await, 1);
    assert_eq!(count(&target, "t08_failure", "t08_z_failure").await, 0);
    assert_eq!(
        persisted(&report)
            .tables
            .iter()
            .map(|table| table.committed_rows)
            .sum::<u64>(),
        1
    );
    assert!(
        !std::fs::read_to_string(PathBuf::from(&report.artifact_dir).join("report.json"))
            .unwrap()
            .contains("invalid\\u0000")
    );
    target
        .client
        .batch_execute("DROP SCHEMA t08_failure CASCADE")
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE source.t08_a_prefix,source.t08_z_failure")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn advisory_lock_and_artifact_failure_precede_writes() {
    let mut config = config("t08_lock", &["all_bytes"]);
    let creds = credentials(&config);
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_lock CASCADE")
        .await
        .unwrap();
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(b"my2pg:v1:t08_lock");
    let key = i64::from_be_bytes(digest[..8].try_into().unwrap());
    target
        .client
        .query_one("SELECT pg_advisory_lock($1)", &[&key])
        .await
        .unwrap();
    let (_, receiver) = watch::channel(false);
    let error = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap_err();
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("advisory lock"));
    target
        .client
        .query_one("SELECT pg_advisory_unlock($1)", &[&key])
        .await
        .unwrap();
    let path = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t08-artifact-file");
    std::fs::write(&path, b"synthetic non-directory").unwrap();
    config.report.directory = path.clone();
    let (_, receiver) = watch::channel(false);
    assert_eq!(
        crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
            .await
            .unwrap_err()
            .exit_code(),
        1
    );
    let exists: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='t08_lock')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists);
    std::fs::remove_file(path).unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn actual_copy_cancellation_rolls_back_pending_batch() {
    let mut config = config("t08_cancel", &["all_bytes"]);
    let creds = credentials(&config);
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_cancel CASCADE")
        .await
        .unwrap();
    config.migration.mode = MigrationMode::SchemaOnly;
    let (_, receiver) = watch::channel(false);
    let schema = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(schema.exit_code(), 0);
    target.client.batch_execute("CREATE FUNCTION t08_cancel.slow() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM pg_sleep(10); RETURN NEW; END$$; CREATE TRIGGER slow BEFORE INSERT ON t08_cancel.all_bytes FOR EACH ROW EXECUTE FUNCTION t08_cancel.slow()").await.unwrap();
    config.migration.mode = MigrationMode::DataOnly;
    config.target.on_existing = ExistingPolicy::Append;
    config
        .target
        .session
        .insert("application_name".into(), "t08-cancel-worker".into());
    let (sender, receiver) = watch::channel(false);
    let observer = async {
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let active:bool=target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name='t08-cancel-worker' AND state='active' AND query LIKE 'COPY %')",&[]).await.unwrap().get(0);
                if active { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("actual COPY must become active before cancellation");
        sender.send(true).unwrap();
    };
    let mut console = console(&config);
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&config, &creds, &mut console, receiver),
        observer
    );
    let report = report.unwrap();
    assert_eq!(report.exit_code(), 130, "{report:?}");
    assert_eq!(report.tables[0].rows_read, 1);
    assert_eq!(report.tables[0].committed_rows, 0);
    assert_eq!(report.tables[0].unresolved_rows, 1);
    assert_eq!(count(&target, "t08_cancel", "all_bytes").await, 0);
    assert_eq!(persisted(&report).status, RunStatus::Cancelled);
    target
        .client
        .batch_execute("DROP SCHEMA t08_cancel CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn pipeline_verification_keeps_the_original_snapshot() {
    let mut config = config("t08_snapshot", &["t08_snapshot"]);
    let creds = credentials(&config);
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t08_snapshot")
        .await
        .unwrap();
    admin
        .query_drop("CREATE TABLE source.t08_snapshot(id INT PRIMARY KEY,value TEXT) ENGINE=InnoDB")
        .await
        .unwrap();
    admin
        .query_drop("INSERT INTO source.t08_snapshot VALUES(1,'snapshot row')")
        .await
        .unwrap();
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_snapshot CASCADE")
        .await
        .unwrap();
    config.migration.mode = MigrationMode::SchemaOnly;
    let (_, receiver) = watch::channel(false);
    assert_eq!(
        crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
            .await
            .unwrap()
            .exit_code(),
        0
    );
    target.client.batch_execute("CREATE FUNCTION t08_snapshot.slow() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM pg_sleep(0.3); RETURN NEW; END$$; CREATE TRIGGER slow BEFORE INSERT ON t08_snapshot.t08_snapshot FOR EACH ROW EXECUTE FUNCTION t08_snapshot.slow()").await.unwrap();
    config.migration.mode = MigrationMode::DataOnly;
    config.target.on_existing = ExistingPolicy::Append;
    config
        .target
        .session
        .insert("application_name".into(), "t08-snapshot-worker".into());
    let observer = async {
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let active:bool=target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name='t08-snapshot-worker' AND state='active' AND query LIKE 'COPY %')",&[]).await.unwrap().get(0);
                if active { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("COPY is active before the concurrent source insert");
        admin
            .query_drop("INSERT INTO source.t08_snapshot VALUES(2,'later row')")
            .await
            .unwrap();
    };
    let (_, receiver) = watch::channel(false);
    let mut console = console(&config);
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&config, &creds, &mut console, receiver),
        observer
    );
    let report = report.unwrap();
    assert_eq!(report.exit_code(), 0, "{report:?}");
    assert_eq!(report.tables[0].rows_read, 1);
    assert_eq!(report.verification.status, VerificationStatus::Complete);
    assert_eq!(count(&target, "t08_snapshot", "t08_snapshot").await, 1);
    assert_eq!(
        admin
            .query_first::<u64, _>("SELECT count(*) FROM source.t08_snapshot")
            .await
            .unwrap(),
        Some(2)
    );
    target
        .client
        .batch_execute("DROP SCHEMA t08_snapshot CASCADE")
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE source.t08_snapshot")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn closed_json_output_fails_and_persists_the_run_report() {
    use std::process::{Command, Stdio};
    let mut config = config("t08_console", &["all_bytes"]);
    config.report.console = OutputFormat::Json;
    config.report.directory = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(format!(
        "t08-closed-console-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let creds = credentials(&config);
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_console CASCADE")
        .await
        .unwrap();
    let path = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t08-console.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .args(["run", path.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let result = tokio::task::spawn_blocking(move || child.wait().unwrap())
        .await
        .unwrap();
    assert_eq!(result.code(), Some(1));
    let directories: Vec<_> = std::fs::read_dir(&config.report.directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(directories.len(), 1);
    let report: RunReport =
        serde_json::from_slice(&std::fs::read(directories[0].join("report.json")).unwrap())
            .unwrap();
    assert_eq!(report.exit_code(), 1);
    assert_eq!(report.status, RunStatus::Failed);
    let exists: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='t08_console')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists, "closed initial event pipe aborts before DDL");
    std::fs::remove_file(path).unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn interrupted_commit_marks_only_submitted_rows_indeterminate() {
    let mut config = config("t08_uncertain", &["t08_uncertain"]);
    config.migration.batch_rows = 100;
    config.migration.batch_bytes = 400;
    config.migration.max_row_bytes = 400;
    let creds = credentials(&config);
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t08_uncertain")
        .await
        .unwrap();
    admin
        .query_drop(
            "CREATE TABLE source.t08_uncertain(id INT PRIMARY KEY,value TEXT) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    admin
        .query_drop(
            "INSERT INTO source.t08_uncertain VALUES(1,REPEAT('a',300)),(2,REPEAT('b',300))",
        )
        .await
        .unwrap();
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_uncertain CASCADE")
        .await
        .unwrap();
    config.migration.mode = MigrationMode::SchemaOnly;
    let (_, receiver) = watch::channel(false);
    assert_eq!(
        crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
            .await
            .unwrap()
            .exit_code(),
        0
    );
    target.client.batch_execute("CREATE FUNCTION t08_uncertain.slow_commit() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM pg_sleep(10); RETURN NEW; END$$; CREATE CONSTRAINT TRIGGER slow_commit AFTER INSERT ON t08_uncertain.t08_uncertain DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION t08_uncertain.slow_commit()").await.unwrap();
    config.migration.mode = MigrationMode::DataOnly;
    config.target.on_existing = ExistingPolicy::Append;
    config
        .target
        .session
        .insert("application_name".into(), "t08-uncertain-worker".into());
    let (sender, receiver) = watch::channel(false);
    let observer = async {
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                let committing:bool=target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name='t08-uncertain-worker' AND state='active' AND query='COMMIT')",&[]).await.unwrap().get(0);
                if committing { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("a deferred trigger must hold actual COMMIT before cancellation");
        sender.send(true).unwrap();
    };
    let mut console = console(&config);
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&config, &creds, &mut console, receiver),
        observer
    );
    let report = report.unwrap();
    assert_eq!(report.status, RunStatus::Indeterminate, "{report:?}");
    assert_eq!(report.exit_code(), 1);
    let table = &report.tables[0];
    assert_eq!(table.rows_read, 2);
    assert_eq!(table.committed_rows, 0);
    assert_eq!(
        table.indeterminate_rows, 1,
        "only the first row was submitted to COPY"
    );
    assert_eq!(
        table.unresolved_rows, 1,
        "the second row remains a lookahead workspace"
    );
    assert_eq!(table.accounted_rows(), Some(2));
    assert_eq!(persisted(&report).tables[0].indeterminate_rows, 1);
    assert_eq!(count(&target, "t08_uncertain", "t08_uncertain").await, 0);
    target
        .client
        .batch_execute("DROP SCHEMA t08_uncertain CASCADE")
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE source.t08_uncertain")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn cancelled_ddl_is_not_a_failed_execution_step() {
    let mut config = config("t08_ddl_cancel", &["all_bytes"]);
    let creds = credentials(&config);
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_ddl_cancel CASCADE")
        .await
        .unwrap();
    config.migration.mode = MigrationMode::SchemaOnly;
    let (_, receiver) = watch::channel(false);
    assert_eq!(
        crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
            .await
            .unwrap()
            .exit_code(),
        0
    );
    target
        .client
        .batch_execute("INSERT INTO t08_ddl_cancel.all_bytes VALUES(99,NULL)")
        .await
        .unwrap();
    // SHARE permits catalog inspection but conflicts with TRUNCATE's
    // ACCESS EXCLUSIVE lock, placing cancellation inside the DDL operation.
    target
        .client
        .batch_execute("BEGIN; LOCK TABLE t08_ddl_cancel.all_bytes IN SHARE MODE")
        .await
        .unwrap();
    config.migration.mode = MigrationMode::DataOnly;
    config.target.on_existing = ExistingPolicy::Truncate;
    config
        .target
        .session
        .insert("application_name".into(), "t08-ddl-cancel-worker".into());
    let (sender, receiver) = watch::channel(false);
    let observer = async {
        let observed=tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                // This observer holds a transaction-level fixture lock;
                // PostgreSQL otherwise reuses its first statistics snapshot.
                target.client.query_one("SELECT pg_stat_clear_snapshot()",&[]).await.unwrap();
                let truncating:bool=target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name='t08-ddl-cancel-worker' AND state='active' AND query LIKE 'TRUNCATE %')",&[]).await.unwrap().get(0);
                if truncating { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await;
        if observed.is_err() {
            let activity:Vec<_>=target.client.query("SELECT state,wait_event_type,wait_event,left(query,240) FROM pg_stat_activity WHERE application_name='t08-ddl-cancel-worker'",&[]).await.unwrap().into_iter().map(|row|(row.get::<_,String>(0),row.get::<_,Option<String>>(1),row.get::<_,Option<String>>(2),row.get::<_,String>(3))).collect();
            panic!("planned TRUNCATE was not observed; activity={activity:?}");
        }
        sender.send(true).unwrap();
    };
    let mut console = console(&config);
    let (report, ()) = tokio::join!(
        async {
            let result = crate::test_pipeline::run(&config, &creds, &mut console, receiver).await;
            if let Err(error) = &result {
                panic!("DDL cancellation preflight returned: {error}");
            }
            result
        },
        observer
    );
    target.client.batch_execute("ROLLBACK").await.unwrap();
    let report = report.unwrap();
    assert_eq!(report.status, RunStatus::Cancelled);
    assert_eq!(report.exit_code(), 130, "{report:?}");
    assert!(report.failed_steps.is_empty());
    assert_eq!(report.tables[0].rows_read, 0);
    assert_eq!(count(&target, "t08_ddl_cancel", "all_bytes").await, 1);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "CANCELLED" && d.object.is_some())
    );
    target
        .client
        .batch_execute("DROP SCHEMA t08_ddl_cancel CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL/PostgreSQL TLS fixture"]
async fn explicit_transform_counts_changed_rows_and_null_stays_unchanged() {
    let mut config = config("t08_transform", &["t08_transform"]);
    let creds = credentials(&config);
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t08_transform")
        .await
        .unwrap();
    admin
        .query_drop(
            "CREATE TABLE source.t08_transform(id INT PRIMARY KEY,value TEXT) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    admin
        .query_drop("INSERT INTO source.t08_transform VALUES(1,''),(2,NULL),(3,'kept')")
        .await
        .unwrap();
    config.cast.push(CastRule {
        source_table: Some("t08_transform".into()),
        source_column: Some("value".into()),
        transform: Some("empty-string-to-null".into()),
        ..Default::default()
    });
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t08_transform CASCADE")
        .await
        .unwrap();
    let (_, receiver) = watch::channel(false);
    let report = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.tables[0].transformations.get("value"), Some(&1));
    let nulls: i64 = target
        .client
        .query_one(
            "SELECT count(*) FROM t08_transform.t08_transform WHERE value IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(nulls, 2);
    target
        .client
        .batch_execute("DROP SCHEMA t08_transform CASCADE")
        .await
        .unwrap();
    config.cast[0].transform = Some("remove-null-characters".into());
    admin
        .query_drop("UPDATE source.t08_transform SET value=CONCAT('ke',CHAR(0),'pt') WHERE id=3")
        .await
        .unwrap();
    let (_, receiver) = watch::channel(false);
    let report = crate::test_pipeline::run(&config, &creds, &mut console(&config), receiver)
        .await
        .unwrap();
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.tables[0].transformations.get("value"), Some(&1));
    let actual: Vec<Option<String>> = target
        .client
        .query(
            "SELECT value FROM t08_transform.t08_transform ORDER BY id",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(actual, [Some(String::new()), None, Some("kept".into())]);
    target
        .client
        .batch_execute("DROP SCHEMA t08_transform CASCADE")
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE source.t08_transform")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    target.close().await.unwrap();
}
