use my2pg::{config::*, model::*, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture {
    config: MigrationConfig,
    source: mysql::SourceConnection,
    target: postgres::TargetConnection,
    path: PathBuf,
    a: String,
    b: String,
}
#[cfg(unix)]
struct Process(Option<std::process::Child>);
#[cfg(unix)]
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
impl Fixture {
    async fn new(name: &str) -> Self {
        let schema = format!("t12_runner_{name}");
        let a = format!("{schema}_a");
        let b = format!("{schema}_b");
        let directory = PathBuf::from(env::var("MY2PG_ARTIFACT_DIR").unwrap()).join(&schema);
        fs::create_dir_all(&directory).unwrap();
        let config: MigrationConfig = serde_json::from_value(serde_json::json!({
            "version":1,
            "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":env::var("MY2PG_TLS_CA").unwrap()},
            "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":env::var("MY2PG_TLS_CA").unwrap()},
            "migration":{"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
            "tables":{"include":[a,b]},
            "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
        })).unwrap();
        let mut source = mysql::connect(
            &config.source,
            &env::var("MY2PG_MYSQL_ROOT_URL").unwrap(),
            1024,
        )
        .await
        .unwrap();
        for table in [&a, &b] {
            source
                .query_drop(format!("DROP TABLE IF EXISTS source.{table}"))
                .await
                .unwrap();
            source.query_drop(format!("CREATE TABLE source.{table}(id INT PRIMARY KEY,value TEXT) ENGINE=InnoDB CHARACTER SET utf8mb4")).await.unwrap();
            for (id, value) in [(1, "good"), (2, "bad\0")] {
                source
                    .exec_drop(
                        format!("INSERT INTO source.{table} VALUES(?,?)"),
                        (id, value),
                    )
                    .await
                    .unwrap();
            }
        }
        for (id, value) in [(3, "server"), (4, "slash\\\t\nΩ")] {
            source
                .exec_drop(format!("INSERT INTO source.{a} VALUES(?,?)"), (id, value))
                .await
                .unwrap();
        }
        let target = postgres::connect(&config.target, &env::var("MY2PG_POSTGRES_URL").unwrap())
            .await
            .unwrap();
        target
            .client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await
            .unwrap();
        Self {
            config,
            source,
            target,
            path: directory.join("migration.toml"),
            a,
            b,
        }
    }
    fn invoke(&self) -> Output {
        fs::write(&self.path, toml::to_string(&self.config).unwrap()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(&self.path)
            .args(["--output", "json", "--progress", "never"])
            .output()
            .unwrap();
        for variable in ["MY2PG_MYSQL_URL", "MY2PG_POSTGRES_URL"] {
            let secret = env::var(variable).unwrap();
            for bytes in [&output.stdout, &output.stderr] {
                let text = String::from_utf8_lossy(bytes);
                assert!(!text.contains(&secret));
                assert!(!text.contains(url::Url::parse(&secret).unwrap().password().unwrap()));
            }
        }
        output
    }
    fn run(&self, exit: i32) -> RunReport {
        self.result(self.invoke(), exit)
    }
    fn result(&self, output: Output, exit: i32) -> RunReport {
        assert_eq!(
            output.status.code(),
            Some(exit),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
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
        assert_eq!(events.last().unwrap()["exit_code"], exit);
        let report: RunReport =
            serde_json::from_value(events.last().unwrap()["report"].clone()).unwrap();
        for event in events
            .iter()
            .filter(|event| event["kind"] == "table_complete")
        {
            let table = report
                .tables
                .iter()
                .find(|table| event["table"] == table.source_name)
                .unwrap();
            assert_eq!(event["committed_rows"], table.committed_rows);
            assert_eq!(event["committed_bytes"], table.committed_bytes);
            assert_eq!(event["rejected_rows"], table.rejected_rows);
            assert_eq!(event["elapsed_millis"], table.copy_elapsed_millis);
        }
        self.validate_report(&report);
        report
    }
    fn validate_report(&self, report: &RunReport) {
        let stored: RunReport = serde_json::from_slice(
            &fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(report).unwrap(),
            serde_json::to_value(&stored).unwrap()
        );
        for table in &report.tables {
            assert_eq!(table.accounted_rows(), Some(table.rows_read));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&report.artifact_dir)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            for file in fs::read_dir(&report.artifact_dir).unwrap() {
                assert_eq!(
                    file.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
    async fn setup(&mut self) {
        self.config.migration.mode = MigrationMode::SchemaOnly;
        self.run(0);
        self.target
            .client
            .batch_execute(&format!(
                "ALTER TABLE {}.{} ADD CONSTRAINT reject_three CHECK(id<>3)",
                self.config.target.schema, self.a
            ))
            .await
            .unwrap();
        self.config.migration.mode = MigrationMode::DataOnly;
        self.config.target.on_existing = ExistingPolicy::Append;
    }
    async fn rows(&self, table: &str) -> Vec<i32> {
        self.target
            .client
            .query(
                &format!(
                    "SELECT id FROM {}.{table} ORDER BY id",
                    self.config.target.schema
                ),
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }
    async fn close(mut self) {
        self.target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA {} CASCADE",
                self.config.target.schema
            ))
            .await
            .unwrap();
        for table in [&self.a, &self.b] {
            self.source
                .query_drop(format!("DROP TABLE source.{table}"))
                .await
                .unwrap();
        }
        self.source.disconnect().await.unwrap();
        self.target.close().await.unwrap();
    }
}

#[cfg(unix)]
async fn cancel_recovered_tail(name: &str, during_commit: bool, closed_output: bool) {
    use std::{process::Stdio, time::Duration};
    let mut fixture = Fixture::new(name).await;
    fixture.setup().await;
    fixture
        .source
        .query_drop(format!(
            "UPDATE source.{} SET value='second' WHERE id=2",
            fixture.a
        ))
        .await
        .unwrap();
    fixture.config.tables.include = vec![fixture.a.clone()];
    fixture.config.migration.batch_rows = 4;
    fixture.config.migration.on_row_error = RowErrorPolicy::Reject;
    fixture.config.migration.max_rejected_rows = 3;
    let locker = postgres::connect(
        &fixture.config.target,
        &env::var("MY2PG_POSTGRES_URL").unwrap(),
    )
    .await
    .unwrap();
    if during_commit {
        fixture.target.client.batch_execute(&format!("ALTER TABLE {}.{} ADD CONSTRAINT tail_unique UNIQUE(value) DEFERRABLE INITIALLY DEFERRED",fixture.config.target.schema,fixture.a)).await.unwrap();
    }
    locker.client.batch_execute("BEGIN").await.unwrap();
    locker
        .client
        .execute(
            &format!(
                "INSERT INTO {}.{} VALUES($1,$2)",
                fixture.config.target.schema, fixture.a
            ),
            &[
                &if during_commit { 999i32 } else { 4i32 },
                &if during_commit {
                    "slash\\\t\nΩ"
                } else {
                    "locker"
                },
            ],
        )
        .await
        .unwrap();
    fixture.config.target.session.insert(
        "application_name".into(),
        fixture.config.target.schema.clone(),
    );
    fs::write(&fixture.path, toml::to_string(&fixture.config).unwrap()).unwrap();
    let mut process = Process(Some(
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(&fixture.path)
            .args(["--output", "json", "--progress", "never"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let running = tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let query=if during_commit {"COMMIT"} else {"COPY %"};
            let waiting:bool=fixture.target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND state='active' AND query LIKE $2 AND wait_event_type='Lock')",&[&fixture.config.target.schema,&query]).await.unwrap().get(0);
            let progress=fs::read_dir(&fixture.config.report.directory).unwrap().find_map(|entry| {
                let path=entry.unwrap().path().join("report.json");
                let Ok(bytes)=fs::read(path) else { return None; };
                let report:RunReport=serde_json::from_slice(&bytes).unwrap();
                (report.mode==MigrationMode::DataOnly && report.status==RunStatus::Running && report.tables[0].committed_rows==2 && report.tables[0].rejected_rows==1).then_some(report)
            });
            if waiting && let Some(report)=progress { break report; }
            assert!(process.0.as_mut().unwrap().try_wait().unwrap().is_none(),"CLI exited before the locked recovery tail");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("observe actual recovery progress and blocked tail before signaling");
    if closed_output {
        drop(process.0.as_mut().unwrap().stdout.take());
    }
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
    .expect("actual CLI signal shutdown must be bounded");
    let output = process.0.take().unwrap().wait_with_output().unwrap();
    let report = if closed_output {
        assert!(during_commit);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let report: RunReport = serde_json::from_slice(
            &fs::read(PathBuf::from(&running.artifact_dir).join("report.json")).unwrap(),
        )
        .unwrap();
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "CONSOLE_IO")
        );
        fixture.validate_report(&report);
        report
    } else {
        fixture.result(output, if during_commit { 1 } else { 130 })
    };
    let table = &report.tables[0];
    assert_eq!(
        (table.rows_read, table.committed_rows, table.rejected_rows),
        (4, 2, 1)
    );
    assert_eq!(table.committed_bytes, 16);
    assert_eq!(
        (table.unresolved_rows, table.indeterminate_rows),
        if during_commit { (0, 1) } else { (1, 0) }
    );
    assert_eq!(
        report.status,
        if during_commit {
            RunStatus::Indeterminate
        } else {
            RunStatus::Cancelled
        }
    );
    assert_eq!(rejects(&report, table).len(), 1);
    assert_eq!(fixture.rows(&fixture.a).await, [1, 2]);
    assert!(report.failed_steps.is_empty());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let live: bool = fixture
                .target
                .client
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1)",
                    &[&fixture.config.target.schema],
                )
                .await
                .unwrap()
                .get(0);
            if !live {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cancelled runner connection must be gone");
    locker.client.batch_execute("ROLLBACK").await.unwrap();
    locker.close().await.unwrap();
    fixture.close().await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_sigterm_preserves_committed_recovery_before_blocked_copy_tail() {
    cancel_recovered_tail("cancel_copy", false, false).await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_sigterm_at_recovery_commit_marks_only_active_tail_indeterminate() {
    cancel_recovered_tail("cancel_commit", true, false).await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_broken_final_output_preserves_indeterminate_commit_report() {
    cancel_recovered_tail("commit_console", true, true).await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_reject_file_collision_preserves_final_failed_report_and_prior_commits() {
    use std::{io::Write, os::unix::fs::OpenOptionsExt, process::Stdio, time::Duration};
    let mut fixture = Fixture::new("reject_io").await;
    fixture.setup().await;
    fixture
        .source
        .query_drop(format!(
            "UPDATE source.{} SET value='second' WHERE id=2",
            fixture.a
        ))
        .await
        .unwrap();
    fixture.config.tables.include = vec![fixture.a.clone()];
    fixture.config.migration.batch_rows = 4;
    fixture.config.migration.on_row_error = RowErrorPolicy::Reject;
    fixture.config.migration.max_rejected_rows = 3;
    fixture.config.target.session.insert(
        "application_name".into(),
        fixture.config.target.schema.clone(),
    );
    let locker = postgres::connect(
        &fixture.config.target,
        &env::var("MY2PG_POSTGRES_URL").unwrap(),
    )
    .await
    .unwrap();
    // Use a separate application name so observing COPY identifies only the CLI.
    locker
        .client
        .batch_execute("SET application_name='t12_reject_io_locker'; BEGIN")
        .await
        .unwrap();
    locker
        .client
        .execute(
            &format!(
                "INSERT INTO {}.{} VALUES(1,'locker')",
                fixture.config.target.schema, fixture.a
            ),
            &[],
        )
        .await
        .unwrap();
    fs::write(&fixture.path, toml::to_string(&fixture.config).unwrap()).unwrap();
    let mut process = Process(Some(
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(&fixture.path)
            .args(["--output", "json", "--progress", "never"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let running = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting:bool=fixture.target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND state='active' AND query LIKE 'COPY %' AND wait_event_type='Lock')",&[&fixture.config.target.schema]).await.unwrap().get(0);
            if waiting {
                for entry in fs::read_dir(&fixture.config.report.directory).unwrap() {
                    let Ok(bytes)=fs::read(entry.unwrap().path().join("report.json")) else {continue;};
                    let report:RunReport=serde_json::from_slice(&bytes).unwrap();
                    if report.mode==MigrationMode::DataOnly && report.status==RunStatus::Running {
                        return report;
                    }
                }
            }
            assert!(process.0.as_mut().unwrap().try_wait().unwrap().is_none(), "CLI exited before initial locked COPY");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("observe actual initial COPY before injecting metadata collision");
    let metadata =
        PathBuf::from(&running.artifact_dir).join(format!("{}.reject.jsonl", running.tables[0].id));
    let sentinel = b"reserved fixture metadata";
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&metadata)
        .unwrap()
        .write_all(sentinel)
        .unwrap();
    locker.client.batch_execute("ROLLBACK").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while process.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("reject I/O failure must stop the executable promptly");
    let report = fixture.result(process.0.take().unwrap().wait_with_output().unwrap(), 1);
    let table = &report.tables[0];
    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(table.status, RunStatus::Failed);
    assert_eq!(
        (
            table.rows_read,
            table.committed_rows,
            table.rejected_rows,
            table.unresolved_rows,
            table.indeterminate_rows
        ),
        (4, 2, 0, 2, 0)
    );
    assert_eq!(table.committed_bytes, 16);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "REJECT_ARTIFACT_IO")
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "ARTIFACT_SHUTDOWN_UNKNOWN")
    );
    assert_eq!(fs::read(&metadata).unwrap(), sentinel);
    assert!(
        fs::read(PathBuf::from(&report.artifact_dir).join(format!("{}.reject.copy", table.id)))
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.rows(&fixture.a).await, [1, 2]);
    locker.close().await.unwrap();
    fixture.close().await;
}

fn rejects(report: &RunReport, table: &TableReport) -> Vec<serde_json::Value> {
    fs::read_to_string(
        PathBuf::from(&report.artifact_dir).join(format!("{}.reject.jsonl", table.id)),
    )
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect()
}

#[tokio::test]
async fn production_artifact_reject_collision_still_publishes_final_failed_report() {
    use my2pg::report::artifact_io::{
        ArtifactIdentity, ArtifactLimits, DurableArtifacts, OwnedRejectBudget, WORKER_ARGUMENT,
    };
    use std::{sync::Arc, time::Duration};
    use tokio::{sync::Semaphore, time::Instant};
    let root = env::temp_dir().join(format!(
        "my2pg-reject-collision-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let identity = ArtifactIdentity::new(&root).unwrap();
    let plan: MigrationPlan = serde_json::from_str(include_str!("../contracts/plan.json")).unwrap();
    let mut report: RunReport =
        serde_json::from_str(include_str!("../contracts/report.json")).unwrap();
    report.run_id = identity.run_id.clone();
    report.artifact_dir = identity.directory.to_str().unwrap().into();
    report.status = RunStatus::Running;
    for table in &mut report.tables {
        table.status = RunStatus::Running;
        table.rows_read = 4;
        table.committed_rows = 2;
        table.committed_bytes = 16;
        table.rejected_rows = 0;
        table.unresolved_rows = 0;
        table.indeterminate_rows = 0;
    }
    let limits = ArtifactLimits::derive(&identity, &plan, &report, 1024).unwrap();
    let ids = plan
        .tables
        .iter()
        .map(|table| table.id.clone())
        .collect::<Vec<_>>();
    let deadline = Instant::now() + Duration::from_secs(3);
    let writer = loop {
        let mut command = Command::new(env!("CARGO_BIN_EXE_my2pg"));
        command.arg(WORKER_ARGUMENT);
        let memory = Arc::new(Semaphore::new(limits.reservation_bytes))
            .acquire_many_owned(limits.reservation_bytes as u32)
            .await
            .unwrap();
        match DurableArtifacts::start(command, identity.clone(), limits, &ids, memory).await {
            Ok(writer) => break writer,
            Err(error)
                if error.kind == my2pg::report::artifact_io::ArtifactFailure::Busy
                    && Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(error) => panic!("production worker failed: {error}"),
        }
    };
    writer
        .lease()
        .await
        .unwrap()
        .write_report(1, &report)
        .await
        .unwrap();
    let metadata = identity.directory.join(format!("{}.reject.jsonl", ids[0]));
    fs::write(&metadata, b"owned collision sentinel").unwrap();
    let bytes = bytes::Bytes::from_static(b"3\tserver\n");
    let storage = Arc::new(BatchStorage {
        permit: Arc::new(Semaphore::new(bytes.len()))
            .acquire_many_owned(bytes.len() as u32)
            .await
            .unwrap(),
        bytes,
    });
    let budget = OwnedRejectBudget::new(1);
    let reason = Diagnostic {
        code: "COPY_ROW_REJECTED".into(),
        stage: "copy".into(),
        object: Some(ids[0].clone()),
        severity: Severity::Warning,
        message: "confirmed rollback".into(),
    };
    let error = writer
        .lease()
        .await
        .unwrap()
        .reject_copy(
            0,
            &RowLocator {
                ordinal: 3,
                key: None,
            },
            storage.clone(),
            0..storage.bytes.len(),
            &reason,
            budget.reserve().unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.kind,
        my2pg::report::artifact_io::ArtifactFailure::Refused
    );
    assert!(error.errno.is_some());
    assert_eq!(
        Arc::strong_count(&storage),
        1,
        "definitively refused bytes remain retained"
    );
    let refused = writer
        .lease()
        .await
        .unwrap()
        .reject_copy(
            0,
            &RowLocator {
                ordinal: 4,
                key: None,
            },
            storage.clone(),
            0..storage.bytes.len(),
            &reason,
            budget.reserve().unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        refused.kind,
        my2pg::report::artifact_io::ArtifactFailure::Refused
    );
    writer.flush().await.unwrap();
    assert_eq!(budget.used(), 0);
    let ledger = writer.confirmed();
    assert_eq!(ledger.report_generation, 1);
    assert_eq!(ledger.pending_sequence, None);
    assert_eq!(ledger.tables[0].rejected_rows, 0);
    report.status = RunStatus::Failed;
    for table in &mut report.tables {
        table.status = RunStatus::Failed;
        table.unresolved_rows = 2;
    }
    writer
        .lease()
        .await
        .unwrap()
        .write_report(2, &report)
        .await
        .unwrap();
    let confirmed = writer
        .shutdown(Instant::now() + Duration::from_secs(3))
        .await;
    assert_eq!(confirmed.report_generation, 2);
    assert_eq!(confirmed.unreaped_pid, None);
    assert_eq!(fs::read(&metadata).unwrap(), b"owned collision sentinel");
    assert!(
        fs::read(identity.directory.join(format!("{}.reject.copy", ids[0])))
            .unwrap()
            .is_empty()
    );
    let disk: RunReport =
        serde_json::from_slice(&fs::read(identity.directory.join("report.json")).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(disk).unwrap(),
        serde_json::to_value(report).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_conversion_and_copy_rejects_share_exact_global_limit() {
    let mut fixture = Fixture::new("exact").await;
    fixture.setup().await;
    fixture.config.migration.on_row_error = RowErrorPolicy::Reject;
    fixture.config.migration.max_rejected_rows = 3;
    let report = fixture.run(3);
    assert_eq!(report.status, RunStatus::Partial);
    assert_eq!(report.verification.status, VerificationStatus::Complete);
    assert_eq!(fixture.rows(&fixture.a).await, [1, 4]);
    assert_eq!(fixture.rows(&fixture.b).await, [1]);
    let a = report
        .tables
        .iter()
        .find(|table| table.source_name == fixture.a)
        .unwrap();
    let b = report
        .tables
        .iter()
        .find(|table| table.source_name == fixture.b)
        .unwrap();
    assert_eq!(
        (
            a.rows_read,
            a.committed_rows,
            a.rejected_rows,
            a.unresolved_rows
        ),
        (4, 2, 2, 0)
    );
    assert_eq!(
        (
            b.rows_read,
            b.committed_rows,
            b.rejected_rows,
            b.unresolved_rows
        ),
        (2, 1, 1, 0)
    );
    assert_eq!(a.status, RunStatus::Partial);
    assert_eq!((a.committed_bytes, b.committed_bytes), (23, 7));
    let records = rejects(&report, a);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["kind"], "conversion");
    assert_eq!(records[0]["locator"]["ordinal"], 2);
    assert_eq!(records[0]["values"][1]["encoding"], "hex");
    assert_eq!(records[0]["values"][1]["value"], "62616400");
    assert_eq!(records[1]["kind"], "copy");
    assert_eq!(records[1]["locator"]["ordinal"], 3);
    assert_eq!(records[1]["offset"], 0);
    assert_eq!(
        fs::read(PathBuf::from(&report.artifact_dir).join(format!("{}.reject.copy", a.id)))
            .unwrap(),
        b"3\tserver\n"
    );
    assert_eq!(rejects(&report, b).len(), 1);
    let exact: String = fixture
        .target
        .client
        .query_one(
            &format!(
                "SELECT value FROM {}.{} WHERE id=4",
                fixture.config.target.schema, fixture.a
            ),
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(exact, "slash\\\t\nΩ");
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_next_reject_after_cap_fails_without_losing_earlier_commits() {
    let mut fixture = Fixture::new("limit").await;
    fixture.setup().await;
    // A completes before B in the original single snapshot; B fails on its first row.
    fixture
        .source
        .query_drop(format!("DELETE FROM source.{} WHERE id=1", fixture.b))
        .await
        .unwrap();
    fixture.config.migration.batch_rows = 1;
    fixture.config.migration.on_row_error = RowErrorPolicy::Reject;
    fixture.config.migration.max_rejected_rows = 2;
    let report = fixture.run(1);
    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "REJECT_LIMIT")
    );
    assert_eq!(fixture.rows(&fixture.a).await, [1, 4]);
    assert!(fixture.rows(&fixture.b).await.is_empty());
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.rejected_rows)
            .sum::<u64>(),
        2
    );
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.committed_rows)
            .sum::<u64>(),
        2
    );
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.unresolved_rows)
            .sum::<u64>(),
        1
    );
    let b = report
        .tables
        .iter()
        .find(|table| table.source_name == fixture.b)
        .unwrap();
    assert!(
        !PathBuf::from(&report.artifact_dir)
            .join(format!("{}.reject.jsonl", b.id))
            .exists()
    );
    assert_eq!(
        (
            b.rows_read,
            b.committed_rows,
            b.rejected_rows,
            b.unresolved_rows,
            b.indeterminate_rows
        ),
        (1, 0, 0, 1, 0)
    );
    let a = report
        .tables
        .iter()
        .find(|table| table.source_name == fixture.a)
        .unwrap();
    assert_eq!(
        (
            a.rows_read,
            a.committed_rows,
            a.rejected_rows,
            a.unresolved_rows,
            a.indeterminate_rows
        ),
        (4, 2, 2, 0, 0)
    );
    assert_eq!(a.committed_bytes, 23);
    let records = rejects(&report, a);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["kind"], "conversion");
    assert_eq!(records[0]["locator"]["ordinal"], 2);
    assert_eq!(records[0]["values"][1]["value"], "62616400");
    assert_eq!(records[1]["kind"], "copy");
    assert_eq!(records[1]["locator"]["ordinal"], 3);
    assert_eq!(
        fs::read(PathBuf::from(&report.artifact_dir).join(format!("{}.reject.copy", a.id)))
            .unwrap(),
        b"3\tserver\n"
    );
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_stop_conversion_never_creates_rejects() {
    let mut fixture = Fixture::new("stop").await;
    fixture.setup().await;
    // Establish an ACK in A, then fail B without depending on within-table read-ahead.
    fixture
        .source
        .query_drop(format!("DELETE FROM source.{} WHERE id<>1", fixture.a))
        .await
        .unwrap();
    fixture
        .source
        .query_drop(format!("DELETE FROM source.{} WHERE id=1", fixture.b))
        .await
        .unwrap();
    fixture.config.migration.batch_rows = 1;
    let report = fixture.run(1);
    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(fixture.rows(&fixture.a).await, [1]);
    assert!(fixture.rows(&fixture.b).await.is_empty());
    let a = report
        .tables
        .iter()
        .find(|table| table.source_name == fixture.a)
        .unwrap();
    let b = report
        .tables
        .iter()
        .find(|table| table.source_name == fixture.b)
        .unwrap();
    assert_eq!(
        (a.rows_read, a.committed_rows, a.unresolved_rows),
        (1, 1, 0)
    );
    assert_eq!(
        (b.rows_read, b.committed_rows, b.unresolved_rows),
        (1, 0, 1)
    );
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.rejected_rows)
            .sum::<u64>(),
        0
    );
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.unresolved_rows)
            .sum::<u64>(),
        1
    );
    assert!(fs::read_dir(&report.artifact_dir).unwrap().all(|file| {
        !file
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("reject")
    }));
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_unsafe_recovery_target_is_rejected_before_truncate() {
    let mut fixture = Fixture::new("unsafe").await;
    fixture.setup().await;
    fixture.target.client.batch_execute(&format!("INSERT INTO {0}.{1} VALUES(999,'sentinel');CREATE SEQUENCE {0}.side_effect;CREATE FUNCTION {0}.unsafe() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM nextval('{0}.side_effect'); RAISE EXCEPTION 'failure' USING ERRCODE='23514'; END$$;CREATE TRIGGER unsafe BEFORE INSERT ON {0}.{1} FOR EACH ROW EXECUTE FUNCTION {0}.unsafe()",fixture.config.target.schema,fixture.a)).await.unwrap();
    fixture.config.target.on_existing = ExistingPolicy::Truncate;
    fixture.config.migration.on_row_error = RowErrorPolicy::Reject;
    fixture.config.migration.max_rejected_rows = 3;
    let prior = fs::read_dir(&fixture.config.report.directory)
        .unwrap()
        .count();
    let output = fixture.invoke();
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fixture.rows(&fixture.a).await, [999]);
    let called: bool = fixture
        .target
        .client
        .query_one(
            &format!(
                "SELECT is_called FROM {}.side_effect",
                fixture.config.target.schema
            ),
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!called);
    assert_eq!(
        fs::read_dir(&fixture.config.report.directory)
            .unwrap()
            .count(),
        prior
    );
    fixture.close().await;
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_existing_hierarchies_fail_before_any_policy_mutates_sentinels() {
    for topology in ["parent", "child", "partition_parent", "partition_leaf"] {
        let mut fixture = Fixture::new(&format!("topology_{topology}")).await;
        fixture.config.tables.include = vec![fixture.a.clone()];
        fixture.setup().await;
        let schema = &fixture.config.target.schema;
        let a = &fixture.a;
        let before = postgres::inspect(&mut fixture.target.client, schema)
            .await
            .unwrap();
        assert!(
            before
                .tables
                .iter()
                .find(|table| table.name == *a)
                .unwrap()
                .ordinary_standalone
        );
        let ddl = match topology {
            "parent" => format!(
                "CREATE TABLE {schema}.related() INHERITS({schema}.{a});INSERT INTO {schema}.{a} VALUES(999,'selected');INSERT INTO {schema}.related VALUES(998,'unselected')"
            ),
            "child" => format!(
                "CREATE TABLE {schema}.related(id INT NOT NULL,value TEXT);ALTER TABLE {schema}.{a} INHERIT {schema}.related;INSERT INTO {schema}.{a} VALUES(999,'selected');INSERT INTO {schema}.related VALUES(998,'unselected')"
            ),
            "partition_parent" => format!(
                "DROP TABLE {schema}.{a};CREATE TABLE {schema}.{a}(id INT NOT NULL,value TEXT) PARTITION BY RANGE(id);CREATE TABLE {schema}.related PARTITION OF {schema}.{a} DEFAULT;INSERT INTO {schema}.{a} VALUES(999,'selected');INSERT INTO {schema}.related VALUES(998,'unselected')"
            ),
            "partition_leaf" => format!(
                "DROP TABLE {schema}.{a};CREATE TABLE {schema}.related(id INT NOT NULL,value TEXT) PARTITION BY RANGE(id);CREATE TABLE {schema}.{a} PARTITION OF {schema}.related DEFAULT;CREATE TABLE {schema}.sibling PARTITION OF {schema}.related FOR VALUES FROM (0) TO (500);INSERT INTO {schema}.{a} VALUES(999,'selected');INSERT INTO {schema}.sibling VALUES(42,'unselected')"
            ),
            _ => unreachable!(),
        };
        fixture.target.client.batch_execute(&ddl).await.unwrap();
        let catalog = postgres::inspect(&mut fixture.target.client, schema)
            .await
            .unwrap();
        assert!(
            !catalog
                .tables
                .iter()
                .find(|table| table.name == *a)
                .unwrap()
                .ordinary_standalone
        );
        let selected_sql = format!("SELECT id FROM ONLY {schema}.{a} ORDER BY id");
        let related_sql = format!("SELECT id FROM {schema}.related ORDER BY id");
        let ids = |rows: Vec<tokio_postgres::Row>| {
            rows.into_iter()
                .map(|row| row.get::<_, i32>(0))
                .collect::<Vec<_>>()
        };
        let selected = ids(fixture
            .target
            .client
            .query(&selected_sql, &[])
            .await
            .unwrap());
        let related = ids(fixture
            .target
            .client
            .query(&related_sql, &[])
            .await
            .unwrap());
        assert_eq!(
            selected,
            if topology == "partition_parent" {
                vec![]
            } else {
                vec![999]
            }
        );
        assert_eq!(
            related,
            match topology {
                "parent" => vec![998],
                "child" | "partition_parent" => vec![998, 999],
                "partition_leaf" => vec![42, 999],
                _ => unreachable!(),
            }
        );
        let prior = fs::read_dir(&fixture.config.report.directory)
            .unwrap()
            .count();
        for policy in [
            ExistingPolicy::Append,
            ExistingPolicy::Truncate,
            ExistingPolicy::Recreate,
        ] {
            fixture.config.target.on_existing = policy;
            fixture.config.migration.mode = if policy == ExistingPolicy::Append {
                MigrationMode::DataOnly
            } else {
                MigrationMode::Full
            };
            // Stop mode must also block before a broad TRUNCATE or successful COPY.
            assert_eq!(fixture.config.migration.on_row_error, RowErrorPolicy::Stop);
            let output = fixture.invoke();
            assert_eq!(
                output.status.code(),
                Some(2),
                "{topology}/{policy:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("TARGET_TABLE_TOPOLOGY"));
            assert_eq!(
                ids(fixture
                    .target
                    .client
                    .query(&selected_sql, &[])
                    .await
                    .unwrap()),
                selected
            );
            assert_eq!(
                ids(fixture
                    .target
                    .client
                    .query(&related_sql, &[])
                    .await
                    .unwrap()),
                related
            );
            assert_eq!(
                fs::read_dir(&fixture.config.report.directory)
                    .unwrap()
                    .count(),
                prior
            );
        }
        fixture.close().await;
    }
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn actual_cli_named_transform_counts_only_changed_nonnull_values() {
    let mut fixture = Fixture::new("transform").await;
    fixture.config.tables.include = vec![fixture.a.clone()];
    fixture
        .source
        .query_drop(format!("DELETE FROM source.{}", fixture.a))
        .await
        .unwrap();
    for (id, value) in [
        (1, Some("tail ")),
        (2, Some("plain")),
        (3, None),
        (4, Some("Ω \t")),
        (5, Some("")),
    ] {
        fixture
            .source
            .exec_drop(
                format!("INSERT INTO source.{} VALUES(?,?)", fixture.a),
                (id, value),
            )
            .await
            .unwrap();
    }
    fixture.config.cast.push(CastRule {
        source_table: Some(fixture.a.clone()),
        source_column: Some("value".into()),
        target_type: Some("text".into()),
        transform: Some("right-trim".into()),
        ..Default::default()
    });
    let report = fixture.run(0);
    assert_eq!(report.status, RunStatus::Complete);
    assert_eq!(report.tables[0].transformations.get("value"), Some(&2));
    assert_eq!(report.tables[0].committed_bytes, 28);
    let actual: Vec<Option<String>> = fixture
        .target
        .client
        .query(
            &format!(
                "SELECT value FROM {}.{} ORDER BY id",
                fixture.config.target.schema, fixture.a
            ),
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        actual,
        [
            Some("tail".into()),
            Some("plain".into()),
            None,
            Some("Ω".into()),
            Some("".into())
        ]
    );
    fixture.close().await;
}
