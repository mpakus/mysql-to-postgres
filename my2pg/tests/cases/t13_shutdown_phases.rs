use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{
    env,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required native fixture variable {name}"))
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
    async fn new(label: &str, tables: usize) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let schema = format!(
            "t13ph_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let app = format!("{schema}_run");
        let names: Vec<_> = (0..tables)
            .map(|index| format!("{schema}_{index}"))
            .collect();
        let config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,
            "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},
            "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":required("MY2PG_TLS_CA"),"session":{"application_name":app}},
            "migration":{"table_workers":2,"index_workers":2,"queue_batches":1,"batch_rows":1,"batch_bytes":1024,"max_row_bytes":1024,"memory_bytes":1048576,"reset_sequences":false},
            "tables":{"include":names},"report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&schema),"progress":"never"}})).unwrap();
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
            source.query_drop(format!("CREATE TABLE {}(id INT NOT NULL PRIMARY KEY,value TEXT NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",mysql::quote_ident(name))).await.unwrap();
            source
                .query_drop(format!(
                    "INSERT INTO {} VALUES(1,'first'),(2,'second')",
                    mysql::quote_ident(name)
                ))
                .await
                .unwrap();
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
    fn table(&self, index: usize) -> String {
        postgres::qualified(&self.config.target.schema, &self.names[index])
    }
    fn console(&self) -> Console {
        Console::new(
            &self.config,
            &Args::try_parse_from(["my2pg", "run", "native.toml", "--quiet"]).unwrap(),
        )
    }
    async fn activity(&self, predicate: &str) -> i64 {
        self.target
            .client
            .batch_execute("SELECT pg_stat_clear_snapshot()")
            .await
            .unwrap();
        self.target.client.query_one(&format!("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND backend_type='client backend' AND {predicate}"),&[&self.app]).await.unwrap().get(0)
    }
    async fn wait(&self, predicate: &str, count: i64) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while self.activity(predicate).await < count {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("missing actual phase {predicate}, expected {count}"));
    }
    async fn closed(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.activity("true").await != 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("all owned target client backends must close");
    }
    async fn gate(&self, key: i64, predicate: &str) -> (String, String) {
        let name = format!("{}_gate", self.config.target.schema);
        let function = format!("public.{}", postgres::quote_ident(&name));
        self.target.client.batch_execute(&format!("CREATE FUNCTION {function}() RETURNS event_trigger LANGUAGE plpgsql AS $$BEGIN IF current_setting('application_name')={} AND ({predicate}) THEN PERFORM pg_advisory_xact_lock({key}); END IF; END$$; CREATE EVENT TRIGGER {} ON ddl_command_start EXECUTE FUNCTION {function}(); SELECT pg_advisory_lock({key})",literal(&self.app),postgres::quote_ident(&name))).await.unwrap();
        (name, function)
    }
    async fn ungate(&self, key: i64, name: &str, function: &str) {
        self.target.client.batch_execute(&format!("SELECT pg_advisory_unlock({key}); DROP EVENT TRIGGER {}; DROP FUNCTION {function}()",postgres::quote_ident(name))).await.unwrap();
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
        // Drop referring source tables before their referenced parent.
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
fn persisted(report: &RunReport) -> RunReport {
    serde_json::from_slice(
        &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PostgreSQL16 TLS fixture"]
async fn concurrent_index_cancellation_before_commit_clears_active_ddl_ledger() {
    let mut case = Case::new("indexes", 3).await;
    case.config.migration.mode = MigrationMode::SchemaOnly;
    case.config.migration.memory_bytes = 1_048_576;
    let key = 81350000i64 + i64::from(std::process::id());
    let (name, function) = case
        .gate(key, "TG_TAG IN ('ALTER TABLE','CREATE INDEX')")
        .await;
    let (sender, receiver) = watch::channel(false);
    let observe = async {
        case.wait("wait_event='advisory'", 2).await;
        assert_eq!(
            case.activity("true").await,
            3,
            "control + two independently active index workers"
        );
        let start = Instant::now();
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
    assert!(
        report.failed_steps.is_empty(),
        "cancelled pre-COMMIT jobs are not failed statements"
    );
    assert!(
        report
            .tables
            .iter()
            .all(|table| table.rows_read == 0 && table.indeterminate_rows == 0)
    );
    assert_eq!(persisted(&report).exit_code(), 130);
    case.closed().await;
    case.ungate(key, &name, &function).await;
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PostgreSQL16 TLS fixture"]
async fn actual_control_verification_cancellation_retains_all_acknowledged_tables() {
    let mut case = Case::new("verify", 3).await;
    case.source
        .query_drop(format!(
            "ALTER TABLE {} ADD CONSTRAINT phase_fk FOREIGN KEY(id) REFERENCES {}(id)",
            mysql::quote_ident(&case.names[0]),
            mysql::quote_ident(&case.names[1])
        ))
        .await
        .unwrap();
    let key = 81360000i64 + i64::from(std::process::id());
    let (name, function) = case
        .gate(
            key,
            "TG_TAG='ALTER TABLE' AND current_query() LIKE '%FOREIGN KEY%'",
        )
        .await;
    let (sender, receiver) = watch::channel(false);
    let observe = async {
        case.wait("query LIKE '%FOREIGN KEY%' AND wait_event='advisory'", 1)
            .await;
        // The FK references tables0/1 only. All COPY/index ACKs precede this
        // barrier; table2 can be locked without preventing the FK transaction.
        case.target
            .client
            .batch_execute(&format!(
                "BEGIN; LOCK TABLE {} IN ACCESS EXCLUSIVE MODE; SELECT pg_advisory_unlock({key})",
                case.table(2)
            ))
            .await
            .unwrap();
        case.wait(
            "query LIKE 'SELECT COUNT(*) FROM %' AND wait_event_type='Lock'",
            1,
        )
        .await;
        let start = Instant::now();
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
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "VERIFICATION_EXECUTION")
    );
    assert_eq!(
        report
            .tables
            .iter()
            .map(|table| table.committed_rows)
            .sum::<u64>(),
        6
    );
    assert!(report.tables.iter().all(|table| table.rejected_rows == 0
        && table.indeterminate_rows == 0
        && table.unresolved_rows == 0));
    assert!(report.failed_steps.is_empty());
    let disk = persisted(&report);
    assert_eq!(disk.exit_code(), 130);
    assert_eq!(
        disk.tables
            .iter()
            .map(|table| table.committed_rows)
            .sum::<u64>(),
        6
    );
    case.closed().await;
    case.target.client.batch_execute("ROLLBACK").await.unwrap();
    for index in 0..3 {
        let count: i64 = case
            .target
            .client
            .query_one(&format!("SELECT count(*) FROM {}", case.table(index)), &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 2);
    }
    case.ungate(key, &name, &function).await;
    case.cleanup().await;
}
