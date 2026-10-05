//! Actual sent-COMMIT cancellation followed by physical report-write failure.
#![cfg(target_os = "macos")]

use my2pg::{
    config::MigrationConfig,
    model::{RunReport, RunStatus},
    mysql, postgres,
};
use mysql_async::prelude::Queryable;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing mandatory {name}"))
}

fn storage(operation: &str, root: Option<&Path>) -> serde_json::Value {
    let mut command = Command::new("python3");
    command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/durable_faults.py"))
        .arg(operation);
    if let Some(root) = root {
        command.arg(root);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "owned storage {operation} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

struct Volume {
    root: PathBuf,
    mount: PathBuf,
}

impl Drop for Volume {
    fn drop(&mut self) {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/durable_faults.py");
        let detached = Command::new("python3")
            .arg(&script)
            .arg("detach")
            .arg(&self.root)
            .output()
            .is_ok_and(|output| output.status.success());
        if !detached {
            assert!(
                Command::new("python3")
                    .arg(script)
                    .arg("force-detach")
                    .arg(&self.root)
                    .output()
                    .is_ok_and(|output| output.status.success()),
                "owned volume cleanup failed: {}",
                self.root.display()
            );
        }
    }
}

struct Process(Option<Child>);
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[tokio::test]
#[ignore = "requires owned native TLS databases and validated private macOS ENOSPC image"]
async fn sent_commit_uncertainty_survives_physical_failure_to_publish_new_report() {
    assert_eq!(required("MY2PG_INTEGRATION"), "1");
    let artifact_root = PathBuf::from(required("MY2PG_ARTIFACT_DIR"));
    assert_eq!(
        artifact_root.parent().unwrap().canonicalize().unwrap(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/integration")
            .canonicalize()
            .unwrap()
    );
    let owner: serde_json::Value =
        serde_json::from_slice(&fs::read(artifact_root.join("connections.json")).unwrap()).unwrap();
    assert_eq!(owner["state"], "ready");
    assert!(owner["project"].as_str().unwrap().starts_with("my2pg-"));
    for key in [
        "MY2PG_MYSQL_URL",
        "MY2PG_MYSQL_ROOT_URL",
        "MY2PG_POSTGRES_URL",
    ] {
        assert!(
            owner["env"][key].as_str() == Some(required(key).as_str()),
            "fixture endpoint ownership mismatch"
        );
    }
    let info = storage("create", None);
    let volume = Volume {
        root: info["root"].as_str().unwrap().into(),
        mount: info["mount"].as_str().unwrap().into(),
    };
    let name = format!(
        "t12_unknown_{}_{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let app = format!("{name}_cli");
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":name,"on_existing":"append","ca_file":required("MY2PG_TLS_CA")},
        "migration":{"mode":"data_only","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":2097152},
        "tables":{"include":[name]},
        "report":{"directory":volume.mount.join("runs"),"console":"json","progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let locker = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let source_table = mysql::quote_ident(&name);
    let table = postgres::qualified(&name, &name);
    source.query_drop(format!("CREATE TABLE {source_table}(id INT PRIMARY KEY,value TEXT) ENGINE=InnoDB CHARSET=utf8mb4")).await.unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {source_table} VALUES(1,'first'),(2,'second'),(3,'third'),(4,'fourth')"
        ))
        .await
        .unwrap();
    target.client.batch_execute(&format!("CREATE SCHEMA {};CREATE TABLE {table}(id integer PRIMARY KEY,value text,CONSTRAINT tail_unique UNIQUE(value) DEFERRABLE INITIALLY DEFERRED)",postgres::quote_ident(&name))).await.unwrap();
    locker.client.batch_execute("BEGIN").await.unwrap();
    locker
        .client
        .batch_execute(&format!("INSERT INTO {table} VALUES(999,'fourth')"))
        .await
        .unwrap();
    config
        .target
        .session
        .insert("application_name".into(), app.clone());
    let path = volume.root.join("migration.toml");
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
    let (report_path, prior, commit_pid) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let rows = target.client.query("SELECT pid FROM pg_stat_activity WHERE application_name=$1 AND state='active' AND query='COMMIT' AND wait_event_type='Lock'", &[&app]).await.unwrap();
            if rows.len() == 1
                && let Ok(entries) = fs::read_dir(&config.report.directory)
            {
                for entry in entries {
                    let path = entry.unwrap().path().join("report.json");
                    if let Ok(bytes) = fs::read(&path) {
                        let report: RunReport = serde_json::from_slice(&bytes).unwrap();
                        if report.status == RunStatus::Running && report.tables[0].committed_rows == 2 {
                            return (path, bytes, rows[0].get::<_, i32>(0));
                        }
                    }
                }
            }
            assert!(process.0.as_mut().unwrap().try_wait().unwrap().is_none(), "CLI exited before real blocked COMMIT and durable ACK2");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("real second COMMIT must reach deferred-key lock after durable first batch");
    let fill = storage("fill", Some(&volume.root));
    assert_eq!(fill["errno"], 28);
    assert_eq!(fill["failed_write_bytes"], 1);
    assert!(
        Command::new("kill")
            .args(["-TERM", &process.0.as_ref().unwrap().id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    tokio::time::timeout(Duration::from_secs(15), async {
        while process.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("signal and failed report publication must stop without replay");
    let output = process.0.take().unwrap().wait_with_output().unwrap();
    fs::write(volume.root.join("cli.stdout.jsonl"), &output.stdout).unwrap();
    fs::write(volume.root.join("cli.stderr.log"), &output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let final_report: RunReport =
        serde_json::from_value(events.last().unwrap()["report"].clone()).unwrap();
    assert_eq!(final_report.status, RunStatus::Indeterminate);
    let outcome = &final_report.tables[0];
    assert_eq!(
        (
            outcome.rows_read,
            outcome.committed_rows,
            outcome.rejected_rows,
            outcome.unresolved_rows,
            outcome.indeterminate_rows
        ),
        (4, 2, 0, 0, 2)
    );
    assert!(
        final_report
            .diagnostics
            .iter()
            .any(|d| d.code == "COMMIT_INDETERMINATE")
    );
    assert!(
        final_report
            .diagnostics
            .iter()
            .any(|d| d.code == "ARTIFACT_SHUTDOWN_UNKNOWN")
    );
    assert_eq!(
        fs::read(&report_path).unwrap(),
        prior,
        "last durable ACK2 report remains exact; uncertainty was not durably published"
    );
    let retained: RunReport = serde_json::from_slice(&prior).unwrap();
    assert_eq!(retained.status, RunStatus::Running);
    assert_eq!(
        (
            retained.tables[0].committed_rows,
            retained.tables[0].indeterminate_rows
        ),
        (2, 0)
    );
    let alive: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1)",
            &[&commit_pid],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !alive,
        "unacknowledged COMMIT backend must be closed before inspecting target"
    );
    locker.client.batch_execute("ROLLBACK").await.unwrap();
    let actual: Vec<(i32, String)> = target
        .client
        .query(&format!("SELECT id,value FROM {table} ORDER BY id"), &[])
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    assert_eq!(
        actual,
        vec![(1, "first".into()), (2, "second".into())],
        "sent suffix is not replayed after uncertainty"
    );
    let original: Vec<(i32, String)> = source
        .query(format!("SELECT id,value FROM {source_table} ORDER BY id"))
        .await
        .unwrap();
    assert_eq!(
        original,
        vec![
            (1, "first".into()),
            (2, "second".into()),
            (3, "third".into()),
            (4, "fourth".into())
        ]
    );
    fs::write(volume.root.join("combined-proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"physical_fill":fill,"commit_pid":commit_pid,"last_durable_report":retained,"unpersisted_final_report":final_report,"target_rows":actual,"source_rows":original,"cli_exit":1,"replayed_rows":0})).unwrap()).unwrap();
    storage("free", Some(&volume.root));
    assert_eq!(
        fs::read(&report_path).unwrap(),
        prior,
        "restoring space must not publish or replay implicitly"
    );
    target
        .client
        .batch_execute(&format!(
            "DROP TABLE {table};DROP SCHEMA {}",
            postgres::quote_ident(&name)
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE {source_table}"))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    locker.close().await.unwrap();
    target.close().await.unwrap();
    eprintln!(
        "combined actual COMMIT/ENOSPC proof retained at {}",
        volume.root.display()
    );
}
