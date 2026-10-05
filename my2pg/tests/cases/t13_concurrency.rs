//! Native resource/queue/teardown oracles; no successful prerequisite skips.
use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, pipeline, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{
    env,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
struct Case {
    config: MigrationConfig,
    creds: ResolvedCredentials,
    target: postgres::TargetConnection,
    source: mysql::SourceConnection,
    names: Vec<String>,
    app: String,
}
impl Case {
    async fn new(label: &str, tables: usize, rows: usize) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let schema = format!(
            "t13_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let app = format!("{schema}_run");
        let names: Vec<_> = (0..tables)
            .map(|index| format!("{schema}_{index}"))
            .collect();
        let config: MigrationConfig = serde_json::from_value(serde_json::json!({
            "version":1,
            "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},
            "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":required("MY2PG_TLS_CA"),"session":{"application_name":app}},
            "migration":{"table_workers":2,"index_workers":2,"queue_batches":1,"batch_rows":1,"batch_bytes":1024,"max_row_bytes":1024,"memory_bytes":1048576,"reset_sequences":false},
            "tables":{"include":names},
            "report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&schema),"progress":"never"}
        })).unwrap();
        let creds = resolve_credentials(&config).unwrap();
        let mut admin = config.target.clone();
        admin
            .session
            .insert("application_name".into(), format!("{schema}_admin"));
        let target = postgres::connect(&admin, creds.target.expose())
            .await
            .unwrap();
        let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1048576)
            .await
            .unwrap();
        for name in &names {
            let table = mysql::quote_ident(name);
            source.query_drop(format!("CREATE TABLE {table}(id INT NOT NULL PRIMARY KEY, value TEXT NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4")).await.unwrap();
            if rows > 0 {
                source
                    .query_drop("SET SESSION cte_max_recursion_depth=1000000")
                    .await
                    .unwrap();
                source.query_drop(format!("INSERT INTO {table} WITH RECURSIVE n AS (SELECT 1 AS id UNION ALL SELECT id+1 FROM n WHERE id<{rows}) SELECT id, REPEAT('x',300) FROM n")).await.unwrap();
            }
        }
        Self {
            config,
            creds,
            target,
            source,
            names,
            app,
        }
    }
    fn console(&self) -> Console {
        Console::new(
            &self.config,
            &Args::try_parse_from(["my2pg", "run", "fixture.toml", "--quiet"]).unwrap(),
        )
    }
    fn table(&self, index: usize) -> String {
        postgres::qualified(&self.config.target.schema, &self.names[index])
    }
    async fn data_only(&mut self) {
        self.config.migration.mode = MigrationMode::SchemaOnly;
        let (_, receiver) = watch::channel(false);
        let report =
            crate::test_pipeline::run(&self.config, &self.creds, &mut self.console(), receiver)
                .await
                .unwrap();
        assert_eq!(report.exit_code(), 0, "schema fixture: {report:?}");
        self.config.migration.mode = MigrationMode::DataOnly;
        self.config.target.on_existing = ExistingPolicy::Append;
    }
    fn exact_memory(&mut self, pipelines: usize) {
        // Current native artifact reservations top out below 48 KiB; this
        // leaves headroom without admitting another ~76 KiB pipeline.
        const ARTIFACT_HEADROOM_BYTES: usize = 64 * 1024;
        self.config.migration.memory_bytes =
            pipeline::scheduler::MemoryLayout::compute(&self.config.migration)
                .unwrap()
                .pipeline_reservation
                .checked_mul(pipelines)
                .and_then(|bytes| bytes.checked_add(ARTIFACT_HEADROOM_BYTES))
                .expect("test pipeline and artifact budgets fit usize");
    }
    async fn active(&self, predicate: &str) -> i64 {
        self.target
            .client
            .batch_execute("SELECT pg_stat_clear_snapshot()")
            .await
            .unwrap();
        self.target.client.query_one(&format!("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND backend_type='client backend' AND {predicate}"), &[&self.app]).await.unwrap().get(0)
    }
    async fn wait_active(&self, predicate: &str, count: i64) {
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            while self.active(predicate).await < count {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if result.is_err() {
            let states: Vec<(String, String, Option<String>, Option<String>)> = self.target.client.query("SELECT state,query,wait_event_type,wait_event FROM pg_stat_activity WHERE application_name=$1", &[&self.app]).await.unwrap().into_iter().map(|row| (row.get(0),row.get(1),row.get(2),row.get(3))).collect();
            panic!("missing real server activity: {predicate}, expected {count}: {states:?}");
        }
    }
    async fn assert_closed(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.active("true").await != 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("every owned runner PostgreSQL backend must disappear");
    }
    async fn count(&self, index: usize) -> i64 {
        self.target
            .client
            .query_one(&format!("SELECT count(*) FROM {}", self.table(index)), &[])
            .await
            .unwrap()
            .get(0)
    }
    async fn cleanup(mut self) {
        self.target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA IF EXISTS {} CASCADE",
                postgres::quote_ident(&self.config.target.schema)
            ))
            .await
            .unwrap();
        for name in &self.names {
            self.source
                .query_drop(format!("DROP TABLE {}", mysql::quote_ident(name)))
                .await
                .unwrap();
        }
        self.source.disconnect().await.unwrap();
        self.target.close().await.unwrap();
    }
}
fn check_persisted(report: &RunReport) {
    let disk: RunReport = serde_json::from_slice(
        &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(disk.status, report.status);
    for (actual, expected) in disk.tables.iter().zip(&report.tables) {
        assert_eq!(
            (
                actual.rows_read,
                actual.committed_rows,
                actual.committed_bytes,
                actual.rejected_rows,
                actual.indeterminate_rows,
                actual.unresolved_rows
            ),
            (
                expected.rows_read,
                expected.committed_rows,
                expected.committed_bytes,
                expected.rejected_rows,
                expected.indeterminate_rows,
                expected.unresolved_rows
            )
        );
    }
}

async fn source_count(monitor: &mut mysql::SourceConnection, predicate: &str) -> u64 {
    let user = mysql_async::Opts::from_url(&required("MY2PG_MYSQL_URL"))
        .unwrap()
        .user()
        .unwrap()
        .to_owned();
    monitor.exec_first(format!("SELECT count(*) FROM information_schema.processlist WHERE USER=? AND DB=DATABASE() AND {predicate}"), (user,)).await.unwrap().unwrap()
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn empty_queues_cancel_blocked_source_and_close_reader_writer_pairs() {
    empty_queues(false).await;
}
#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn source_socket_fault_wakes_empty_consumers_and_stops_sibling() {
    empty_queues(true).await;
}
async fn empty_queues(source_fault: bool) {
    let case = Case::new(if source_fault { "emptyfault" } else { "empty" }, 2, 5).await;
    let key = 81310000i64 + i64::from(std::process::id());
    let trigger = format!("{}_gate", case.config.target.schema);
    let function = format!("public.{}", postgres::quote_ident(&trigger));
    case.target.client.batch_execute(&format!("CREATE FUNCTION {function}() RETURNS event_trigger LANGUAGE plpgsql AS $$BEGIN IF current_setting('application_name')={} AND TG_TAG='CREATE TABLE' THEN PERFORM pg_advisory_xact_lock({key}); END IF; END$$; CREATE EVENT TRIGGER {} ON ddl_command_start EXECUTE FUNCTION {function}(); SELECT pg_advisory_lock({key})", literal(&case.app), postgres::quote_ident(&trigger))).await.unwrap();
    let mut monitor = mysql::connect(
        &case.config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        1048576,
    )
    .await
    .unwrap();
    let (sender, receiver) = watch::channel(false);
    let observe = async {
        case.wait_active("wait_event='advisory'", 1).await;
        monitor
            .query_drop(format!(
                "LOCK TABLES {} WRITE, {} WRITE",
                mysql::quote_ident(&case.names[0]),
                mysql::quote_ident(&case.names[1])
            ))
            .await
            .unwrap();
        case.target
            .client
            .batch_execute(&format!("SELECT pg_advisory_unlock({key})"))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while source_count(
                &mut monitor,
                "INFO LIKE 'SELECT %' AND STATE LIKE '%lock%' ",
            )
            .await
                != 2
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("both source SELECTs must block before any row is received");
        assert_eq!(
            source_count(&mut monitor, "true").await,
            3,
            "one original + two admitted source connections"
        );
        assert_eq!(case.active("true").await, 3);
        assert_eq!(
            case.active("query LIKE 'COPY %'").await,
            0,
            "consumers have empty queues"
        );
        if source_fault {
            let pattern = format!("%{}%", case.names[0]);
            let id: u64 = monitor.exec_first("SELECT ID FROM information_schema.processlist WHERE INFO LIKE ? AND ID<>CONNECTION_ID()",(pattern,)).await.unwrap().expect("owned source table worker");
            monitor
                .query_drop(format!("KILL CONNECTION {id}"))
                .await
                .unwrap();
        } else {
            sender.send(true).unwrap();
        }
    };
    let mut console = case.console();
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&case.config, &case.creds, &mut console, receiver),
        observe
    );
    let report = report.unwrap();
    assert_eq!(
        report.exit_code(),
        if source_fault { 1 } else { 130 },
        "{report:?}"
    );
    if source_fault {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "SOURCE_READ")
        );
    }
    assert!(
        report
            .tables
            .iter()
            .all(|table| table.rows_read == 0 && table.committed_rows == 0)
    );
    check_persisted(&report);
    case.assert_closed().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while source_count(&mut monitor, "true").await != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("all source readers close without draining blocked result");
    monitor.query_drop("UNLOCK TABLES").await.unwrap();
    monitor.disconnect().await.unwrap();
    case.target
        .client
        .batch_execute(&format!(
            "DROP EVENT TRIGGER {}; DROP FUNCTION {function}()",
            postgres::quote_ident(&trigger)
        ))
        .await
        .unwrap();
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn mixed_conversion_copy_rejects_share_one_global_durable_limit() {
    let mut case = Case::new("rejects", 2, 4).await;
    case.data_only().await;
    case.exact_memory(2);
    case.config.migration.on_row_error = RowErrorPolicy::Reject;
    case.config.migration.max_rejected_rows = 3;
    for index in 0..2 {
        case.source.query_drop(format!("UPDATE {} SET value=CASE id WHEN 2 THEN CHAR(0) WHEN 3 THEN 'server-reject' ELSE value END", mysql::quote_ident(&case.names[index]))).await.unwrap();
        case.target
            .client
            .batch_execute(&format!(
                "ALTER TABLE {} ADD CONSTRAINT positive CHECK(id<>3)",
                case.table(index)
            ))
            .await
            .unwrap();
    }
    let (_, receiver) = watch::channel(false);
    let report =
        crate::test_pipeline::run(&case.config, &case.creds, &mut case.console(), receiver)
            .await
            .unwrap();
    assert_eq!(
        report.exit_code(),
        1,
        "a fatal global cap must never be hidden by a cancelled sibling: {report:?}"
    );
    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "REJECT_LIMIT")
    );
    let rejects = report
        .tables
        .iter()
        .map(|table| table.rejected_rows)
        .sum::<u64>();
    assert_eq!(rejects, 3);
    let mut metadata_rows = 0;
    for entry in std::fs::read_dir(&report.artifact_dir).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            metadata_rows += std::fs::read_to_string(path).unwrap().lines().count() as u64;
        }
    }
    assert_eq!(
        metadata_rows, rejects,
        "only durable metadata advances the shared cap/report"
    );
    for (index, table) in report.tables.iter().enumerate() {
        assert_eq!(case.count(index).await as u64, table.committed_rows);
        assert_eq!(table.indeterminate_rows, 0);
        assert_eq!(
            table.rows_read,
            table.committed_rows + table.rejected_rows + table.unresolved_rows
        );
    }
    check_persisted(&report);
    case.assert_closed().await;
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn control_ddl_signal_before_commit_is_cancelled_not_transport_failure() {
    let mut case = Case::new("control", 1, 0).await;
    case.config.migration.mode = MigrationMode::SchemaOnly;
    let key = 81320000i64 + i64::from(std::process::id());
    let trigger = format!("{}_gate", case.config.target.schema);
    let function = format!("public.{}", postgres::quote_ident(&trigger));
    case.target.client.batch_execute(&format!("CREATE FUNCTION {function}() RETURNS event_trigger LANGUAGE plpgsql AS $$BEGIN IF current_setting('application_name')={} AND TG_TAG='CREATE TABLE' THEN PERFORM pg_advisory_xact_lock({key}); END IF; END$$; CREATE EVENT TRIGGER {} ON ddl_command_start EXECUTE FUNCTION {function}(); SELECT pg_advisory_lock({key})", literal(&case.app), postgres::quote_ident(&trigger))).await.unwrap();
    let (sender, receiver) = watch::channel(false);
    let observe = async {
        case.wait_active("wait_event='advisory'", 1).await;
        sender.send(true).unwrap();
    };
    let mut console = case.console();
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&case.config, &case.creds, &mut console, receiver),
        observe
    );
    let report = report.unwrap();
    assert_eq!(report.exit_code(), 130, "{report:?}");
    assert!(report.failed_steps.is_empty());
    assert_eq!(report.tables[0].indeterminate_rows, 0);
    check_persisted(&report);
    case.assert_closed().await;
    case.target
        .client
        .batch_execute(&format!(
            "SELECT pg_advisory_unlock({key}); DROP EVENT TRIGGER {}; DROP FUNCTION {function}()",
            postgres::quote_ident(&trigger)
        ))
        .await
        .unwrap();
    case.cleanup().await;
}

#[cfg(unix)]
struct Process(Option<std::process::Child>);
#[cfg(unix)]
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[cfg(unix)]
fn child(case: &Case) -> Process {
    use std::process::{Command, Stdio};
    std::fs::create_dir_all(&case.config.report.directory).unwrap();
    let path = case.config.report.directory.join("migration.toml");
    std::fs::write(&path, toml::to_string(&case.config).unwrap()).unwrap();
    Process(Some(
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(path)
            .args(["--output", "json", "--progress", "never"])
            .stdout(Stdio::from(
                std::fs::File::create(case.config.report.directory.join("stdout.jsonl")).unwrap(),
            ))
            .stderr(Stdio::from(
                std::fs::File::create(case.config.report.directory.join("stderr.log")).unwrap(),
            ))
            .spawn()
            .unwrap(),
    ))
}
fn last_report(case: &Case) -> RunReport {
    let mut reports: Vec<_> = std::fs::read_dir(&case.config.report.directory)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path().join("report.json");
            if path.is_file() { Some(path) } else { None }
        })
        .collect();
    reports.sort();
    serde_json::from_slice(&std::fs::read(reports.last().expect("durable runner report")).unwrap())
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 TLS fixture and ps RSS oracle"]
async fn actual_cli_fixed_budget_rss_does_not_scale_with_tenfold_source_rows() {
    let mut measurements = vec![];
    for rows in [20000usize, 200000] {
        let mut case = Case::new("rss", 2, rows).await;
        case.config.migration.batch_rows = 100;
        case.config.migration.batch_bytes = 65536;
        case.config.migration.queue_batches = 2;
        case.exact_memory(2);
        let mut monitor = mysql::connect(
            &case.config.source,
            &required("MY2PG_MYSQL_ROOT_URL"),
            1048576,
        )
        .await
        .unwrap();
        let mut process = child(&case);
        let pid = process.0.as_ref().unwrap().id().to_string();
        let mut peak_kib = 0u64;
        let mut samples = 0usize;
        let mut source_peak = 0u64;
        let mut target_peak = 0i64;
        let status = tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                if let Some(status) = process.0.as_mut().unwrap().try_wait().unwrap() {
                    break status;
                }
                let output = std::process::Command::new("ps")
                    .args(["-o", "rss=", "-p", &pid])
                    .output()
                    .expect("native ps RSS oracle");
                if let Ok(rss) = String::from_utf8(output.stdout)
                    .unwrap()
                    .trim()
                    .parse::<u64>()
                {
                    peak_kib = peak_kib.max(rss);
                    samples += 1;
                }
                source_peak = source_peak.max(source_count(&mut monitor, "true").await);
                target_peak = target_peak.max(case.active("true").await);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("owned CLI must complete under fixed memory budget without deadlock");
        assert_eq!(
            status.code(),
            Some(0),
            "inspect retained stdout/stderr at {}",
            case.config.report.directory.display()
        );
        process.0.take().unwrap().wait().unwrap();
        assert!(samples > 2 && peak_kib > 0);
        assert!(
            source_peak <= 3 && target_peak <= 3,
            "source={source_peak} target={target_peak}"
        );
        assert_eq!(
            target_peak, 3,
            "native concurrent target activity must be observed"
        );
        let report = last_report(&case);
        assert_eq!(
            report
                .tables
                .iter()
                .map(|table| table.committed_rows)
                .sum::<u64>(),
            (rows * 2) as u64
        );
        assert_eq!(
            report
                .tables
                .iter()
                .map(|table| table.committed_bytes)
                .sum::<u64>(),
            (0..2)
                .map(|_| (1..=rows)
                    .map(|id| id.to_string().len() as u64 + 302)
                    .sum::<u64>())
                .sum::<u64>()
        );
        check_persisted(&report);
        case.assert_closed().await;
        let record = serde_json::json!({"rows_per_table":rows,"application_bytes":case.config.migration.memory_bytes,"peak_rss_kib":peak_kib,"samples":samples,"source_peak":source_peak,"target_peak":target_peak,"driver_tls_allocator_catalog_overhead_included_in_rss":true});
        std::fs::write(
            case.config.report.directory.join("rss.json"),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
        eprintln!("T13 RSS {record}");
        measurements.push(peak_kib);
        monitor.disconnect().await.unwrap();
        case.cleanup().await;
    }
    assert!(
        measurements[1] <= measurements[0] + 32768,
        "tenfold source rows must not cause unbounded retained row growth; measured KiB={measurements:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 TLS fixture and real SIGTERM"]
async fn actual_cli_sigterm_stops_two_blocked_full_queues() {
    let mut case = Case::new("sigterm", 2, 100).await;
    case.data_only().await;
    case.exact_memory(2);
    case.target
        .client
        .batch_execute(&format!(
            "BEGIN; INSERT INTO {} VALUES(1,'locker'); INSERT INTO {} VALUES(1,'locker')",
            case.table(0),
            case.table(1)
        ))
        .await
        .unwrap();
    let mut process = child(&case);
    case.wait_active("query LIKE 'COPY %' AND wait_event_type='Lock'", 2)
        .await;
    assert_eq!(case.active("true").await, 3);
    let start = std::time::Instant::now();
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &process.0.as_ref().unwrap().id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = process.0.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SIGTERM joins all admitted pipelines within the shutdown deadline");
    assert!(start.elapsed() < Duration::from_secs(10));
    assert_eq!(status.code(), Some(130));
    process.0.take().unwrap().wait().unwrap();
    let report = last_report(&case);
    assert_eq!(report.status, RunStatus::Cancelled);
    assert!(report.tables.iter().all(|table| table.committed_rows == 0
        && table.indeterminate_rows == 0
        && table.rows_read <= 3));
    check_persisted(&report);
    case.assert_closed().await;
    case.target.client.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(case.count(0).await + case.count(1).await, 0);
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn full_queues_cancel_two_copy_workers_and_release_all_connections() {
    let mut case = Case::new("full", 2, 100).await;
    case.data_only().await;
    case.exact_memory(2);
    case.target
        .client
        .batch_execute(&format!(
            "BEGIN; INSERT INTO {} VALUES(1,'locker'); INSERT INTO {} VALUES(1,'locker')",
            case.table(0),
            case.table(1)
        ))
        .await
        .unwrap();
    let (sender, receiver) = watch::channel(false);
    let observe = async {
        case.wait_active("query LIKE 'COPY %' AND wait_event_type='Lock'", 2)
            .await;
        assert_eq!(
            case.active("true").await,
            3,
            "control + exactly two target workers"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        let start = std::time::Instant::now();
        sender.send(true).unwrap();
        start
    };
    let mut console = case.console();
    let (report, start) = tokio::join!(
        crate::test_pipeline::run(&case.config, &case.creds, &mut console, receiver),
        observe
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    let report = report.unwrap();
    assert_eq!(report.exit_code(), 130, "{report:?}");
    for table in &report.tables {
        assert_eq!(
            table.committed_rows + table.rejected_rows + table.indeterminate_rows,
            0
        );
        assert!(
            table.rows_read > 0 && table.rows_read <= 3,
            "Q + active + filling: {table:?}"
        );
        assert_eq!(table.unresolved_rows, table.rows_read);
    }
    check_persisted(&report);
    case.assert_closed().await;
    case.target.client.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(case.count(0).await + case.count(1).await, 0);
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn actual_writer_socket_failure_stops_full_queue_sibling_without_replay() {
    let mut case = Case::new("writerfault", 2, 100).await;
    case.data_only().await;
    case.exact_memory(2);
    case.target
        .client
        .batch_execute(&format!(
            "BEGIN; INSERT INTO {} VALUES(1,'locker'); INSERT INTO {} VALUES(1,'locker')",
            case.table(0),
            case.table(1)
        ))
        .await
        .unwrap();
    let (_, receiver) = watch::channel(false);
    let observe = async {
        case.wait_active("query LIKE 'COPY %' AND wait_event_type='Lock'", 2)
            .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let pattern = format!("%{}%", case.names[0]);
        let killed:bool=case.target.client.query_one("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name=$1 AND query LIKE $2 AND query LIKE 'COPY %'",&[&case.app,&pattern]).await.unwrap().get(0);
        assert!(killed, "terminate only this owned worker backend");
    };
    let mut console = case.console();
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&case.config, &case.creds, &mut console, receiver),
        observe
    );
    let report = report.unwrap();
    assert_eq!(report.exit_code(), 1, "{report:?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "COPY")
    );
    assert!(report.tables.iter().all(|table| table.committed_rows == 0
        && table.indeterminate_rows == 0
        && table.rows_read <= 3));
    check_persisted(&report);
    case.assert_closed().await;
    case.target.client.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(case.count(0).await + case.count(1).await, 0);
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn exact_one_pipeline_budget_reduces_admission_and_finishes_all_tables() {
    let mut case = Case::new("minimum", 3, 5).await;
    case.data_only().await;
    case.config.migration.table_workers = 3;
    case.exact_memory(1);
    case.target
        .client
        .batch_execute(&format!(
            "BEGIN; INSERT INTO {} VALUES(1,'locker')",
            case.table(0)
        ))
        .await
        .unwrap();
    let (_, receiver) = watch::channel(false);
    let observe = async {
        case.wait_active("query LIKE 'COPY %' AND wait_event_type='Lock'", 1)
            .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            case.active("true").await,
            2,
            "control + one admitted target"
        );
        case.target.client.batch_execute("ROLLBACK").await.unwrap();
    };
    let mut console = case.console();
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&case.config, &case.creds, &mut console, receiver),
        observe
    );
    let report = report.unwrap();
    assert_eq!(report.exit_code(), 0, "{report:?}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RESOURCE_ADMISSION"
                && diagnostic.message.contains("table_workers=1/3"))
    );
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.committed_rows)
            .sum::<u64>(),
        15
    );
    for index in 0..3 {
        assert_eq!(case.count(index).await, 5);
    }
    check_persisted(&report);
    case.assert_closed().await;
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn schema_only_index_workers_are_effective_and_separately_bounded() {
    let mut case = Case::new("indexes", 3, 0).await;
    case.config.migration.mode = MigrationMode::SchemaOnly;
    case.config.migration.table_workers = 3;
    case.config.migration.memory_bytes = 1_048_576;
    let key = 81300000i64 + i64::from(std::process::id());
    let trigger = format!("{}_gate", case.config.target.schema);
    let function = format!("public.{}", postgres::quote_ident(&trigger));
    case.target.client.batch_execute(&format!("CREATE FUNCTION {function}() RETURNS event_trigger LANGUAGE plpgsql AS $$BEGIN IF current_setting('application_name')={} AND TG_TAG IN ('ALTER TABLE','CREATE INDEX') THEN PERFORM pg_advisory_xact_lock({key}); END IF; END$$; CREATE EVENT TRIGGER {} ON ddl_command_start EXECUTE FUNCTION {function}(); SELECT pg_advisory_lock({key})", literal(&case.app), postgres::quote_ident(&trigger))).await.unwrap();
    let (_, receiver) = watch::channel(false);
    let observe = async {
        case.wait_active("wait_event='advisory'", 2).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            case.active("true").await,
            3,
            "control + exactly two index workers"
        );
        case.target
            .client
            .batch_execute(&format!("SELECT pg_advisory_unlock({key})"))
            .await
            .unwrap();
    };
    let mut console = case.console();
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&case.config, &case.creds, &mut console, receiver),
        observe
    );
    let report = report.unwrap();
    assert_eq!(report.exit_code(), 0, "{report:?}");
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "RESOURCE_ADMISSION"
            && diagnostic
                .message
                .contains("table_workers=0/3 index_workers=2/2")
    }));
    assert!(report.tables.iter().all(|table| table.rows_read == 0));
    check_persisted(&report);
    case.assert_closed().await;
    case.target
        .client
        .batch_execute(&format!(
            "DROP EVENT TRIGGER {}; DROP FUNCTION {function}()",
            postgres::quote_ident(&trigger)
        ))
        .await
        .unwrap();
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn concurrent_commit_cancellation_retains_sibling_ack_prefix_without_replay() {
    let mut case = Case::new("commit", 2, 2).await;
    case.data_only().await;
    case.exact_memory(2);
    case.source
        .query_drop(format!(
            "UPDATE {} SET value=CONCAT('commit-',id)",
            mysql::quote_ident(&case.names[0])
        ))
        .await
        .unwrap();
    case.target
        .client
        .batch_execute(&format!(
            "ALTER TABLE {} ADD CONSTRAINT t13_commit_unique UNIQUE(value) DEFERRABLE INITIALLY DEFERRED",
            case.table(0)
        ))
        .await
        .unwrap();
    case.target.client.batch_execute("BEGIN").await.unwrap();
    case.target
        .client
        .batch_execute(&format!(
            "INSERT INTO {} (id,value) VALUES (3,'commit-1')",
            case.table(0)
        ))
        .await
        .unwrap();
    let (sender, receiver) = watch::channel(false);
    let observe = async {
        case.wait_active(
            "query='COMMIT' AND state='active' AND cardinality(pg_blocking_pids(pid)) > 0",
            1,
        )
        .await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while case.count(1).await != 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("sibling must ACK both batches while another COMMIT is blocked");
        sender.send(true).unwrap();
    };
    let mut console = case.console();
    let (report, ()) = tokio::join!(
        crate::test_pipeline::run(&case.config, &case.creds, &mut console, receiver),
        observe
    );
    let report = report.unwrap();
    assert_eq!(report.status, RunStatus::Indeterminate, "{report:?}");
    assert_eq!(
        (
            report.tables[0].committed_rows,
            report.tables[0].indeterminate_rows,
            report.tables[0].unresolved_rows
        ),
        (0, 1, 1)
    );
    assert_eq!(
        (
            report.tables[1].committed_rows,
            report.tables[1].indeterminate_rows,
            report.tables[1].unresolved_rows
        ),
        (2, 0, 0)
    );
    assert_eq!(case.count(1).await, 2);
    check_persisted(&report);
    case.assert_closed().await;
    let conflicted: Vec<(i32, String)> = case
        .target
        .client
        .query(
            &format!("SELECT id,value FROM {} ORDER BY id", case.table(0)),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(conflicted, vec![(3, "commit-1".to_owned())]);
    case.target.client.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(
        case.count(0).await,
        0,
        "server-side cancellation rolled back; report conservatively retains unknown attempted COMMIT"
    );
    case.cleanup().await;
}
