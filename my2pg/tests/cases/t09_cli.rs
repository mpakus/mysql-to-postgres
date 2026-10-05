use my2pg::config::MigrationConfig;
use std::{
    env, fs,
    path::Path,
    process::{Command, Output},
};

fn configuration(schema: &str, tables: &[&str], directory: &Path) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":env::var("MY2PG_TLS_CA").expect("owned TLS fixture required")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":env::var("MY2PG_TLS_CA").unwrap()},
        "migration":{"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":tables},
        "overrides":[{"object":"source.orders.label.collation","omit":true}],
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap()
}

fn invoke(operation: &str, path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg(operation)
        .arg(path)
        .args(["--output", "json", "--progress", "never"])
        .output()
        .expect("run the actual my2pg binary")
}

fn private_workspace(name: &str) -> std::path::PathBuf {
    let directory = env::temp_dir().join(format!("my2pg-t09-{name}-{}", std::process::id()));
    fs::create_dir(&directory).expect("each test owns one fresh directory");
    directory
}

fn assert_redacted(output: &Output) {
    for variable in ["MY2PG_MYSQL_URL", "MY2PG_POSTGRES_URL"] {
        let secret = env::var(variable).unwrap();
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&secret));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(&secret));
        if let Some(password) = url::Url::parse(&secret).unwrap().password() {
            assert!(
                !password.is_empty(),
                "fixture must exercise a real password"
            );
            assert!(!String::from_utf8_lossy(&output.stdout).contains(password));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(password));
        }
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires the owned disposable MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_sigterm_rolls_back_active_copy_and_persists_cancellation() {
    use my2pg::{
        config::{ExistingPolicy, MigrationMode},
        model::{RunReport, RunStatus},
    };
    use std::{
        process::{Child, Stdio},
        time::Duration,
    };

    struct Process(Option<Child>);
    impl Drop for Process {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    let directory = private_workspace("sigterm");
    let mut config = configuration("t09_cli_sigterm", &["all_bytes"], &directory);
    let path = directory.join("migration.toml");
    config.migration.mode = MigrationMode::SchemaOnly;
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let setup = invoke("run", &path);
    assert_eq!(
        setup.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let target = target(&config).await;
    target.client.batch_execute("CREATE FUNCTION t09_cli_sigterm.slow() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM pg_sleep(30); RETURN NEW; END$$; CREATE TRIGGER slow BEFORE INSERT ON t09_cli_sigterm.all_bytes FOR EACH ROW EXECUTE FUNCTION t09_cli_sigterm.slow()").await.unwrap();
    config.migration.mode = MigrationMode::DataOnly;
    config.target.on_existing = ExistingPolicy::Append;
    config
        .target
        .session
        .insert("application_name".into(), "t09-cli-sigterm".into());
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let mut process = Process(Some(
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(&path)
            .args(["--output", "json", "--progress", "never"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let active: bool = target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name='t09-cli-sigterm' AND state='active' AND query LIKE 'COPY %')", &[]).await.unwrap().get(0);
            if active { break; }
            assert!(process.0.as_mut().unwrap().try_wait().unwrap().is_none(), "CLI exited before actual COPY became active");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("observe active COPY before sending the process signal");
    assert!(
        Command::new("kill")
            .args(["-TERM", &process.0.as_ref().unwrap().id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while process.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("SIGTERM must terminate the CLI within the shutdown deadline");
    let output = process.0.take().unwrap().wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(130),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_redacted(&output);
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "outcome")
            .count(),
        1
    );
    assert_eq!(events.last().unwrap()["exit_code"], 130);
    let reports: Vec<RunReport> = fs::read_dir(&config.report.directory)
        .unwrap()
        .map(|entry| {
            serde_json::from_slice(&fs::read(entry.unwrap().path().join("report.json")).unwrap())
                .unwrap()
        })
        .collect();
    let cancelled: Vec<_> = reports
        .iter()
        .filter(|report| report.status == RunStatus::Cancelled)
        .collect();
    assert_eq!(cancelled.len(), 1);
    let report = cancelled[0];
    assert_eq!(report.exit_code(), 130);
    assert_eq!(report.tables[0].rows_read, 1);
    assert_eq!(report.tables[0].committed_rows, 0);
    assert_eq!(report.tables[0].unresolved_rows, 1);
    assert_eq!(report.tables[0].indeterminate_rows, 0);
    assert_eq!(report.tables[0].accounted_rows(), Some(1));
    let count: i64 = target
        .client
        .query_one("SELECT COUNT(*) FROM t09_cli_sigterm.all_bytes", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let exists: bool = target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name='t09-cli-sigterm')", &[]).await.unwrap().get(0);
            if !exists { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("the CLI must not leave its target backend alive");
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

async fn target(config: &MigrationConfig) -> my2pg::postgres::TargetConnection {
    my2pg::postgres::connect(&config.target, &env::var("MY2PG_POSTGRES_URL").unwrap())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the owned disposable MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_plan_is_read_only_and_run_preserves_relational_values() {
    let directory = private_workspace("success");
    let config = configuration(
        "t09_cli_success",
        &[
            "users",
            "orders",
            "empty_table",
            "keyless",
            "CamelCase",
            "numeric_edges",
            "all_bytes",
        ],
        &directory,
    );
    let path = directory.join("migration.toml");
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let target = target(&config).await;
    assert!(
        target
            .client
            .query_opt(
                "SELECT oid FROM pg_namespace WHERE nspname='t09_cli_success'",
                &[]
            )
            .await
            .unwrap()
            .is_none()
    );
    for operation in ["inspect", "plan"] {
        let output = invoke(operation, &path);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_redacted(&output);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["operation"], operation);
        assert_eq!(value["config"]["migration"]["batch_rows"], 2);
        if operation == "plan" {
            assert_eq!(value["plan"]["tables"].as_array().unwrap().len(), 7);
            assert!(!value["plan"]["ddl"].as_array().unwrap().is_empty());
        }
        assert!(
            target
                .client
                .query_opt(
                    "SELECT oid FROM pg_namespace WHERE nspname='t09_cli_success'",
                    &[]
                )
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            !config.report.directory.exists(),
            "read-only operations must not create run artifacts"
        );
    }
    let output = invoke("run", &path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert_redacted(&output);
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "outcome")
            .count(),
        1
    );
    assert_eq!(events.last().unwrap()["exit_code"], 0);
    let amount: String = target
        .client
        .query_one(
            "SELECT amount::text FROM t09_cli_success.users WHERE id=1",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(amount, "1234567890123456789012.12345678");
    let bytes: Vec<u8> = target
        .client
        .query_one(
            "SELECT value FROM t09_cli_success.all_bytes WHERE id=1",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(bytes, (0..=255u8).collect::<Vec<_>>());
    let unsigned: String = target
        .client
        .query_one(
            "SELECT unsigned_big::text FROM t09_cli_success.numeric_edges WHERE id=1",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(unsigned, u64::MAX.to_string());
    let duplicates: i64 = target
        .client
        .query_one(
            "SELECT COUNT(*) FROM t09_cli_success.keyless WHERE value='same' AND qty=2",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(duplicates, 2);
    let next: i64 = target
        .client
        .query_one(
            "SELECT nextval(pg_get_serial_sequence('t09_cli_success.users','id'))",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(next, 42);
    for (sql, state) in [
        (
            "INSERT INTO t09_cli_success.users SELECT * FROM t09_cli_success.users WHERE id=1",
            "23505",
        ),
        (
            "INSERT INTO t09_cli_success.orders VALUES (999,99,'foreign')",
            "23503",
        ),
    ] {
        let error = target.client.batch_execute(sql).await.unwrap_err();
        assert_eq!(error.code().unwrap().code(), state);
    }
    let sentinel: String = target
        .client
        .query_one("SELECT sentinel FROM public.users WHERE id=999", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sentinel, "must-survive");
    let run_directory = fs::read_dir(&config.report.directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let report: my2pg::model::RunReport =
        serde_json::from_slice(&fs::read(run_directory.join("report.json")).unwrap()).unwrap();
    assert_eq!(report.exit_code(), 0);
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.committed_rows)
            .sum::<u64>(),
        14
    );
    assert_eq!(report.verification.tables_checked, 7);
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires the owned disposable MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_conversion_failure_persists_accurate_partial_report() {
    let directory = private_workspace("failure");
    let config = configuration("t09_cli_failure", &["invalid_values"], &directory);
    let path = directory.join("migration.toml");
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let output = invoke("run", &path);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_redacted(&output);
    let run_directory = fs::read_dir(&config.report.directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let report: my2pg::model::RunReport =
        serde_json::from_slice(&fs::read(run_directory.join("report.json")).unwrap()).unwrap();
    assert_eq!(report.status, my2pg::model::RunStatus::Failed);
    assert_eq!(report.tables[0].rows_read, 1);
    assert_eq!(report.tables[0].committed_rows, 0);
    assert_eq!(report.tables[0].unresolved_rows, 1);
    assert_eq!(report.tables[0].accounted_rows(), Some(1));
    assert_eq!(report.exit_code(), 1);
    let target = target(&config).await;
    let count: i64 = target
        .client
        .query_one("SELECT COUNT(*) FROM t09_cli_failure.invalid_values", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}
