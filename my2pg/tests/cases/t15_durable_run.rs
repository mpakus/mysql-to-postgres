//! Independent production-worker and CLI durable-prefix oracles.
use my2pg::{
    config::*,
    model::*,
    mysql,
    pipeline::scheduler::{Admission, MemoryLayout, Resources},
    postgres,
    report::artifact_io::{ArtifactIdentity, ArtifactLimits, DurableArtifacts, WORKER_ARGUMENT},
};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::PathBuf, process::Command, time::Duration};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required native fixture {name}"))
}
fn directory(name: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "my2pg-t15-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[tokio::test]
async fn production_worker_uses_admitted_artifact_bytes_and_retains_confirmed_report() {
    let root = directory("admission");
    let identity = ArtifactIdentity::new(&root).unwrap();
    let plan: MigrationPlan = serde_json::from_str(include_str!("../contracts/plan.json")).unwrap();
    let mut report: RunReport =
        serde_json::from_str(include_str!("../contracts/report.json")).unwrap();
    report.run_id = identity.run_id.clone();
    report.artifact_dir = identity.directory.to_str().unwrap().into();
    let limits = ArtifactLimits::derive(&identity, &plan, &report, 1024).unwrap();
    let config:MigrationConfig = serde_json::from_value(serde_json::json!({"version":1,
        "source":{"url_env":"SRC","consistency":"single_snapshot"},
        "target":{"url_env":"DST","schema":"legacy"},
        "migration":{"batch_rows":1,"batch_bytes":1024,"max_row_bytes":1024,"memory_bytes":1048576}})).unwrap();
    let admission = Admission::new_with_artifact_reservation(
        &config.migration,
        Consistency::SingleSnapshot,
        plan.tables.len(),
        0,
        limits.reservation_bytes,
    )
    .unwrap();
    let resources = Resources::new(admission);
    assert_eq!(
        resources.available_bytes(),
        admission.memory_bytes - limits.reservation_bytes
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_my2pg"));
    command.arg(WORKER_ARGUMENT);
    let ids = plan
        .tables
        .iter()
        .map(|table| table.id.clone())
        .collect::<Vec<_>>();
    let writer = DurableArtifacts::start(
        command,
        identity.clone(),
        limits,
        &ids,
        resources.take_artifact_reservation().unwrap(),
    )
    .await
    .unwrap();
    writer
        .lease()
        .await
        .unwrap()
        .write_plan(&plan)
        .await
        .unwrap();
    writer
        .lease()
        .await
        .unwrap()
        .write_report(1, &report)
        .await
        .unwrap();
    let confirmed = writer
        .shutdown(tokio::time::Instant::now() + Duration::from_secs(3))
        .await;
    assert_eq!(confirmed.report_generation, 1);
    assert_eq!(confirmed.unreaped_pid, None);
    drop(writer);
    // The reaper may retain Life until the actor exits; boundedly observe release.
    tokio::time::timeout(Duration::from_secs(3), async {
        while resources.available_bytes() != admission.memory_bytes {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let disk: RunReport =
        serde_json::from_slice(&fs::read(identity.directory.join("report.json")).unwrap()).unwrap();
    assert_eq!(disk.run_id, report.run_id);
    assert!(
        identity
            .directory
            .join("report.00000000000000000001.json")
            .is_file()
    );
    fs::remove_dir_all(root).unwrap();
}

struct Native {
    config: MigrationConfig,
    source: mysql::SourceConnection,
    target: postgres::TargetConnection,
    name: String,
    directory: PathBuf,
}
impl Native {
    async fn new(label: &str) -> Self {
        let name = format!("t15_durable_{label}_{}", std::process::id());
        let directory = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&name);
        fs::create_dir_all(&directory).unwrap();
        let config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,
            "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},
            "target":{"url_env":"MY2PG_POSTGRES_URL","schema":name,"ca_file":required("MY2PG_TLS_CA")},
            "migration":{"batch_rows":4,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":1048576,"reset_sequences":false},
            "tables":{"include":[name]},"verification":{"mode":"none"},
            "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}})).unwrap();
        let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 4096)
            .await
            .unwrap();
        source
            .query_drop(format!(
                "DROP TABLE IF EXISTS {}",
                mysql::quote_ident(&name)
            ))
            .await
            .unwrap();
        source
            .query_drop(format!(
                "CREATE TABLE {}(id INT PRIMARY KEY,value TEXT) ENGINE=InnoDB",
                mysql::quote_ident(&name)
            ))
            .await
            .unwrap();
        for (id, value) in [
            (1, "good"),
            (2, "bad\0"),
            (3, "server"),
            (4, "slash\\\t\nΩ"),
        ] {
            source
                .exec_drop(
                    format!("INSERT INTO {} VALUES(?,?)", mysql::quote_ident(&name)),
                    (id, value),
                )
                .await
                .unwrap();
        }
        let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap();
        target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA IF EXISTS {} CASCADE",
                postgres::quote_ident(&name)
            ))
            .await
            .unwrap();
        Self {
            config,
            source,
            target,
            name,
            directory,
        }
    }
    async fn invoke(&self, config: &MigrationConfig) -> std::process::Output {
        let path = self.directory.join("migration.toml");
        fs::write(&path, toml::to_string(config).unwrap()).unwrap();
        let output = tokio::time::timeout(
            Duration::from_secs(40),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
                .args([
                    "run",
                    path.to_str().unwrap(),
                    "--output",
                    "json",
                    "--progress",
                    "never",
                ])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("finite CLI shutdown")
        .unwrap();
        for variable in ["MY2PG_MYSQL_URL", "MY2PG_POSTGRES_URL"] {
            let url = required(variable);
            let password = url::Url::parse(&url)
                .unwrap()
                .password()
                .unwrap()
                .to_owned();
            for bytes in [&output.stdout, &output.stderr] {
                let text = String::from_utf8_lossy(bytes);
                assert!(!text.contains(&url));
                assert!(!text.contains(&password));
            }
        }
        output
    }
    fn report(&self, output: &std::process::Output) -> RunReport {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|event| event["kind"] == "outcome")
            .map(|event| serde_json::from_value(event["report"].clone()).unwrap())
            .unwrap_or_else(|| {
                panic!(
                    "missing outcome: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
            })
    }
    async fn close(mut self) {
        self.source
            .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&self.name)))
            .await
            .unwrap();
        self.target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA {} CASCADE",
                postgres::quote_ident(&self.name)
            ))
            .await
            .unwrap();
        self.source.disconnect().await.unwrap();
        self.target.close().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PostgreSQL16 TLS fixture"]
async fn production_cli_persists_conversion_and_copy_rejects_before_final_accounting() {
    let case = Native::new("prefix").await;
    let mut schema = case.config.clone();
    schema.migration.mode = MigrationMode::SchemaOnly;
    let output = case.invoke(&schema).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let qualified = postgres::qualified(&case.name, &case.name);
    case.target
        .client
        .batch_execute(&format!(
            "ALTER TABLE {qualified} ADD CONSTRAINT reject_server CHECK(value <> 'server')"
        ))
        .await
        .unwrap();
    let mut data = case.config.clone();
    data.migration.mode = MigrationMode::DataOnly;
    data.target.on_existing = ExistingPolicy::Append;
    data.migration.on_row_error = RowErrorPolicy::Reject;
    data.migration.max_rejected_rows = 2;
    let output = case.invoke(&data).await;
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = case.report(&output);
    let table = &report.tables[0];
    assert_eq!(
        (
            table.rows_read,
            table.committed_rows,
            table.rejected_rows,
            table.unresolved_rows,
            table.indeterminate_rows
        ),
        (4, 2, 2, 0, 0)
    );
    let rows = case
        .target
        .client
        .query(
            &format!("SELECT id,value FROM {qualified} ORDER BY id"),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, i32>(0), 1);
    assert_eq!(rows[1].get::<_, String>(1), "slash\\\t\nΩ");
    let artifact = PathBuf::from(&report.artifact_dir);
    let disk: RunReport =
        serde_json::from_slice(&fs::read(artifact.join("report.json")).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(&disk).unwrap()
    );
    let records = fs::read_to_string(artifact.join(format!("{}.reject.jsonl", table.id)))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .any(|r| r["kind"] == "conversion" && r["locator"]["ordinal"] == 2)
    );
    assert!(
        records
            .iter()
            .any(|r| r["kind"] == "copy" && r["locator"]["ordinal"] == 3 && r["offset"] == 0)
    );
    assert_eq!(
        fs::read(artifact.join(format!("{}.reject.copy", table.id))).unwrap(),
        b"3\tserver\n"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "RESOURCE_ADMISSION" && d.message.contains("artifact_bytes="))
    );
    case.close().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PostgreSQL16 TLS fixture"]
async fn artifact_admission_and_creation_fail_before_target_mutation() {
    let mut case = Native::new("preflight").await;
    let mut small = case.config.clone();
    small.migration.memory_bytes = MemoryLayout::compute(&small.migration)
        .unwrap()
        .pipeline_reservation;
    validate(&small).unwrap();
    let output = case.invoke(&small).await;
    assert_eq!(output.status.code(), Some(2));
    let blocked = case.directory.join("ordinary-file");
    fs::write(&blocked, b"owned sentinel").unwrap();
    let mut bad = case.config.clone();
    bad.report.directory = blocked.clone();
    let output = case.invoke(&bad).await;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(blocked).unwrap(), b"owned sentinel");
    let exists: bool = case
        .target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname=$1)",
            &[&case.name],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!exists);
    let rows: u64 = case
        .source
        .query_first(format!(
            "SELECT COUNT(*) FROM {}",
            mysql::quote_ident(&case.name)
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rows, 4);
    // No target schema was created; clean only the owned source table.
    case.source
        .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&case.name)))
        .await
        .unwrap();
    case.source.disconnect().await.unwrap();
    case.target.close().await.unwrap();
}
