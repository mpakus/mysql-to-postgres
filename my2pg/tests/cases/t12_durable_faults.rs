//! Native filesystem/descriptor faults; physical and descriptor evidence differ.
#![cfg(target_os = "macos")]
use my2pg::{
    config::*,
    model::*,
    mysql,
    pipeline::recovery::{self, RecoveryContext, RecoveryFailure, RejectBudget},
    postgres::{self, TargetConnection},
    report::RunArtifacts,
};
use mysql_async::prelude::Queryable;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing mandatory {name}"))
}
fn helper(operation: &str, root: Option<&Path>) -> serde_json::Value {
    let mut command = Command::new("python3");
    command.arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/durable_faults.py"));
    command.arg(operation);
    if let Some(root) = root {
        command.arg(root);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "native helper {operation} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
struct Volume {
    root: PathBuf,
    mount: PathBuf,
}
impl Volume {
    fn new() -> Self {
        let info = helper("create", None);
        Self {
            root: info["root"].as_str().unwrap().into(),
            mount: info["mount"].as_str().unwrap().into(),
        }
    }
    fn fill(&self) {
        let result = helper("fill", Some(&self.root));
        assert_eq!(result["errno"], 28);
        assert_eq!(result["failed_write_bytes"], 1);
        fs::write(
            self.root.join("enospc.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
    }
}
impl Drop for Volume {
    fn drop(&mut self) {
        // Every command validates image, device and mount identity before detach.
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/durable_faults.py");
        let result = Command::new("python3")
            .arg(&script)
            .arg("detach")
            .arg(&self.root)
            .output();
        if !result.is_ok_and(|result| result.status.success()) {
            let result = Command::new("python3")
                .arg(script)
                .arg("force-detach")
                .arg(&self.root)
                .output();
            assert!(
                result.is_ok_and(|result| result.status.success()),
                "owned image cleanup failed: {}",
                self.root.display()
            );
        }
    }
}
fn reason() -> Diagnostic {
    Diagnostic {
        code: "COPY_ROW_REJECTED".into(),
        stage: "copy".into(),
        object: None,
        severity: Severity::Warning,
        message: "native durability fixture".into(),
    }
}
fn report(artifacts: &RunArtifacts) -> RunReport {
    let mut report: RunReport =
        serde_json::from_str(include_str!("../contracts/report.json")).unwrap();
    report.run_id = artifacts.run_id.clone();
    report.artifact_dir = artifacts.directory.to_string_lossy().into_owned();
    report
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires approved owned native macOS disk-image fault lane"]
fn physical_enospc_poisoned_reject_and_atomic_report_preserve_prior_durable_evidence() {
    const TABLE: &str = "t_0000000000000021";
    let volume = Volume::new();
    let artifacts = RunArtifacts::create(&volume.mount.join("runs")).unwrap();
    let mut initial = report(&artifacts);
    initial.status = RunStatus::Indeterminate;
    artifacts.write_report(&initial).unwrap();
    artifacts
        .reject_copy(
            TABLE,
            &RowLocator {
                ordinal: 1,
                key: None,
            },
            b"-1\n",
            &reason(),
        )
        .unwrap();
    let path = artifacts.directory.join("report.json");
    let metadata = artifacts.directory.join(format!("{TABLE}.reject.jsonl"));
    let prior_report = fs::read(&path).unwrap();
    let prior_metadata = fs::read(&metadata).unwrap();
    volume.fill();
    initial.elapsed_millis += 1;
    let failure = artifacts.write_report(&initial).unwrap_err();
    assert_eq!(
        failure.raw_os_error(),
        Some(28),
        "actual report write must fail ENOSPC"
    );
    assert_eq!(fs::read(&path).unwrap(), prior_report);
    assert!(!artifacts.directory.join("report.json.tmp").exists());
    let row = format!("-2\t{}\n", "x".repeat(16_384));
    let failure = artifacts
        .reject_copy(
            TABLE,
            &RowLocator {
                ordinal: 2,
                key: None,
            },
            row.as_bytes(),
            &reason(),
        )
        .unwrap_err();
    assert_eq!(
        failure.raw_os_error(),
        Some(28),
        "actual reject write must fail ENOSPC"
    );
    assert_eq!(fs::read(&metadata).unwrap(), prior_metadata);
    assert!(
        artifacts
            .flush()
            .unwrap_err()
            .to_string()
            .contains("previously failed")
    );
    let retained: RunReport = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(retained.status, RunStatus::Indeterminate);
    helper("free", Some(&volume.root));
    artifacts.write_report(&initial).unwrap();
    assert_eq!(
        serde_json::from_slice::<RunReport>(&fs::read(path).unwrap())
            .unwrap()
            .status,
        RunStatus::Indeterminate
    );
}

async fn target() -> TargetConnection {
    let config: TargetConfig = serde_json::from_value(serde_json::json!({"url_env":"MY2PG_POSTGRES_URL","schema":"legacy","ca_file":required("MY2PG_TLS_CA")})).unwrap();
    postgres::connect(&config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap()
}
fn table(schema: &str) -> TablePlan {
    TablePlan {
        id: "t_0000000000000021".into(),
        source_name: "source".into(),
        target_schema: schema.into(),
        target_name: "rows".into(),
        engine: "InnoDB".into(),
        is_view: false,
        estimated_rows: None,
        next_auto_increment: None,
        primary_key: vec![],
        structure: Default::default(),
        columns: vec![ColumnPlan {
            source_name: "n".into(),
            target_name: "n".into(),
            source_type: "int".into(),
            target_type: "integer".into(),
            kind: ValueKind::Text,
            nullable: true,
            default_sql: None,
            identity: false,
            generated_expression: None,
            copy: true,
            transform: None,
            charset: None,
            comment: None,
            enum_labels: vec![],
            set_labels: vec![],
        }],
    }
}
async fn batch(lines: &[&[u8]], first: u64) -> EncodedBatch {
    let mut bytes = Vec::new();
    let mut rows = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let start = bytes.len();
        bytes.extend_from_slice(line);
        rows.push(RowPosition {
            start,
            end: bytes.len(),
            locator: RowLocator {
                ordinal: first + index as u64,
                key: None,
            },
        });
    }
    let permit = Arc::new(tokio::sync::Semaphore::new(bytes.len()))
        .acquire_many_owned(bytes.len() as u32)
        .await
        .unwrap();
    EncodedBatch {
        storage: Arc::new(BatchStorage {
            bytes: bytes.into(),
            permit,
        }),
        rows,
    }
}
fn unique() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "t12_df_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires PostgreSQL TLS fixture and isolated child descriptor fault"]
async fn metadata_descriptor_sync_fault_never_advances_second_durable_reject() {
    const CHILD: &str = "MY2PG_DURABLE_METADATA_CHILD";
    if env::var_os(CHILD).is_none() {
        let output=Command::new(env::current_exe().unwrap()).args(["--ignored","--exact","t12_durable_faults::metadata_descriptor_sync_fault_never_advances_second_durable_reject","--nocapture"]).env(CHILD,"1").output().unwrap();
        assert!(
            output.status.success(),
            "isolated descriptor child failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use std::{
        ffi::{CStr, c_int},
        io::{BufRead, BufReader},
        os::{
            fd::{AsRawFd, BorrowedFd},
            unix::{fs::MetadataExt, net::UnixStream},
        },
    };
    unsafe extern "C" {
        fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
        fn dup2(old: c_int, new: c_int) -> c_int;
        fn fsync(fd: c_int) -> c_int;
    }
    const F_GETPATH: c_int = 50;
    let mut conn = target().await;
    let planned = table(&unique());
    conn.client
        .batch_execute(&format!(
            "CREATE SCHEMA {};CREATE TABLE {}(n integer CHECK(n>=0))",
            postgres::quote_ident(&planned.target_schema),
            postgres::qualified(&planned.target_schema, &planned.target_name)
        ))
        .await
        .unwrap();
    let artifacts = RunArtifacts::create(
        &PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t12-durable-descriptor"),
    )
    .unwrap();
    let budget = RejectBudget::new(3);
    let mut observer = |_| Ok(());
    let initial = recovery::copy_with_recovery(
        &mut conn,
        &planned,
        &batch(&[b"1\n", b"-1\n"], 1).await,
        &mut RecoveryContext {
            policy: RowErrorPolicy::Reject,
            artifacts: &artifacts,
            reject_budget: &budget,
            observer: &mut observer,
        },
    )
    .await
    .unwrap();
    assert_eq!((initial.committed_rows, initial.rejected_rows), (1, 1));
    let mut baseline = report(&artifacts);
    baseline.status = RunStatus::Partial;
    baseline.tables[0].id = planned.id.clone();
    baseline.tables[0].source_name = planned.source_name.clone();
    baseline.tables[0].target_schema = planned.target_schema.clone();
    baseline.tables[0].target_name = planned.target_name.clone();
    baseline.tables[0].status = RunStatus::Partial;
    baseline.tables[0].committed_rows = initial.committed_rows;
    baseline.tables[0].committed_bytes = 2; // The acknowledged exact COPY row is b"1\n".
    baseline.tables[0].rejected_rows = initial.rejected_rows;
    artifacts.write_report(&baseline).unwrap();
    let report_path = artifacts.directory.join("report.json");
    let metadata = artifacts
        .directory
        .join(format!("{}.reject.jsonl", planned.id));
    let data = artifacts
        .directory
        .join(format!("{}.reject.copy", planned.id));
    let before_report = fs::read(&report_path).unwrap();
    let before_metadata = fs::read(&metadata).unwrap();
    let identity = fs::metadata(&metadata).unwrap();
    let mut descriptors = Vec::new();
    for fd in 3..1024 {
        let mut path = [0u8; 1024];
        // F_GETPATH observes only this disposable child's descriptor table.
        if unsafe { fcntl(fd, F_GETPATH, path.as_mut_ptr()) } == 0 {
            let path = unsafe { CStr::from_ptr(path.as_ptr().cast()) }.to_bytes();
            if path == metadata.as_os_str().as_encoded_bytes() {
                descriptors.push(fd);
            }
        }
    }
    assert_eq!(
        descriptors.len(),
        1,
        "identify exactly the production metadata File"
    );
    let fd = descriptors[0];
    // Darwin /dev/fd is fdescfs; its stat device is not the backing file device.
    // Borrow only our verified child descriptor and fstat a temporary duplicate.
    let observed = fs::File::from(
        unsafe { BorrowedFd::borrow_raw(fd) }
            .try_clone_to_owned()
            .unwrap(),
    )
    .metadata()
    .unwrap();
    assert_eq!(
        (observed.dev(), observed.ino()),
        (identity.dev(), identity.ino())
    );
    let (socket, peer) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(unsafe { dup2(socket.as_raw_fd(), fd) }, fd);
    assert_eq!(unsafe { fsync(fd) }, -1);
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap();
    assert_eq!(
        errno, 22,
        "real fsync on replaced socket must return EINVAL"
    );
    fs::write(artifacts.directory.join("descriptor-proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"metadata_path":metadata,"fd":fd,"original_device":identity.dev(),"original_inode":identity.ino(),"actual_fsync_errno":errno,"kind":"descriptor fault, not physical sync fault"})).unwrap()).unwrap();
    let failure = recovery::copy_with_recovery(
        &mut conn,
        &planned,
        &batch(&[b"-2\n"], 3).await,
        &mut RecoveryContext {
            policy: RowErrorPolicy::Reject,
            artifacts: &artifacts,
            reject_budget: &budget,
            observer: &mut observer,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(failure.kind, RecoveryFailure::RejectIo);
    assert_eq!(failure.progress.rejected_rows, 0);
    assert_eq!(budget.used(), 1);
    let mut delivered = String::new();
    BufReader::new(peer).read_line(&mut delivered).unwrap();
    let delivered: serde_json::Value = serde_json::from_str(&delivered).unwrap();
    assert_eq!(delivered["locator"]["ordinal"], 3);
    assert_eq!(
        fs::read(&data).unwrap(),
        b"-1\n-2\n",
        "data append/sync precedes successful metadata socket write"
    );
    assert_eq!(fs::read(&metadata).unwrap(), before_metadata);
    assert_eq!(fs::read(&report_path).unwrap(), before_report);
    let retained: RunReport = serde_json::from_slice(&before_report).unwrap();
    assert_eq!(
        (
            retained.tables[0].committed_rows,
            retained.tables[0].rejected_rows
        ),
        (1, 1)
    );
    assert!(
        artifacts
            .flush()
            .unwrap_err()
            .to_string()
            .contains("previously failed")
    );
    let count: i64 = conn
        .client
        .query_one(
            &format!(
                "SELECT count(*) FROM {}",
                postgres::qualified(&planned.target_schema, &planned.target_name)
            ),
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
    conn.client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&planned.target_schema)
        ))
        .await
        .unwrap();
    conn.close().await.unwrap();
}

struct Process(Option<std::process::Child>);
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
struct Fixture {
    config: MigrationConfig,
    source: mysql::SourceConnection,
    target: TargetConnection,
    name: String,
    path: PathBuf,
}
impl Fixture {
    async fn new(volume: &Volume) -> Self {
        let name = unique();
        let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,"source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},"target":{"url_env":"MY2PG_POSTGRES_URL","schema":name,"on_existing":"append","ca_file":required("MY2PG_TLS_CA")},"migration":{"mode":"data_only","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},"tables":{"include":[name]},"report":{"directory":volume.mount.join("runs"),"console":"json","progress":"never"}})).unwrap();
        let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
            .await
            .unwrap();
        source.query_drop(format!("CREATE TABLE source.{}(id INT PRIMARY KEY,value TEXT) ENGINE=InnoDB CHARACTER SET utf8mb4",mysql::quote_ident(&name))).await.unwrap();
        for (id, value) in [(1, "first"), (2, "second"), (3, "third"), (4, "fourth")] {
            source
                .exec_drop(
                    format!(
                        "INSERT INTO source.{} VALUES(?,?)",
                        mysql::quote_ident(&name)
                    ),
                    (id, value),
                )
                .await
                .unwrap();
        }
        let target = target().await;
        target
            .client
            .batch_execute(&format!(
                "CREATE SCHEMA {};CREATE TABLE {}(id integer PRIMARY KEY,value text)",
                postgres::quote_ident(&name),
                postgres::qualified(&name, &name)
            ))
            .await
            .unwrap();
        config
            .target
            .session
            .insert("application_name".into(), format!("{name}_cli"));
        let path = volume.root.join("migration.toml");
        fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
        Self {
            config,
            source,
            target,
            name,
            path,
        }
    }
    fn spawn(&self) -> Process {
        Process(Some(
            Command::new(env!("CARGO_BIN_EXE_my2pg"))
                .arg("run")
                .arg(&self.path)
                .args(["--output", "json", "--progress", "never"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ))
    }
    async fn close(mut self) {
        self.target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA {} CASCADE",
                postgres::quote_ident(&self.name)
            ))
            .await
            .unwrap();
        self.source
            .query_drop(format!(
                "DROP TABLE source.{}",
                mysql::quote_ident(&self.name)
            ))
            .await
            .unwrap();
        self.source.disconnect().await.unwrap();
        self.target.close().await.unwrap();
    }
}
async fn wait(process: &mut Process) -> Output {
    tokio::time::timeout(Duration::from_secs(15), async {
        while process.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("faulted CLI must stop promptly");
    process.0.take().unwrap().wait_with_output().unwrap()
}
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires TLS DB fixture and approved physical ENOSPC image lane"]
async fn actual_cli_physical_enospc_stops_without_replaying_and_preserves_ack_report() {
    let volume = Volume::new();
    let fixture = Fixture::new(&volume).await;
    let locker = target().await;
    locker.client.batch_execute("BEGIN").await.unwrap();
    locker
        .client
        .execute(
            &format!(
                "INSERT INTO {} VALUES(3,'owned locker')",
                postgres::qualified(&fixture.name, &fixture.name)
            ),
            &[],
        )
        .await
        .unwrap();
    let mut process = fixture.spawn();
    let (report_path,prior)=tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let active:bool=fixture.target.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name=$1 AND state='active' AND query LIKE 'COPY %' AND wait_event_type='Lock')",&[&format!("{}_cli",fixture.name)]).await.unwrap().get(0);
            if active {for entry in fs::read_dir(&fixture.config.report.directory).unwrap(){let path=entry.unwrap().path().join("report.json");if let Ok(bytes)=fs::read(&path){let report:RunReport=serde_json::from_slice(&bytes).unwrap();if report.tables[0].committed_rows==2{return (path,bytes);}}}}
            assert!(process.0.as_mut().unwrap().try_wait().unwrap().is_none(),"CLI exited before the blocked second batch");tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("observe real blocked COPY and durable ACK2 before filling image");
    volume.fill();
    locker.client.batch_execute("COMMIT").await.unwrap();
    let output = wait(&mut process).await;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        fs::read(&report_path).unwrap(),
        prior,
        "failed replacement preserves last durable ACK snapshot"
    );
    let retained: RunReport = serde_json::from_slice(&prior).unwrap();
    assert_eq!(retained.tables[0].committed_rows, 2);
    assert_eq!(retained.tables[0].rejected_rows, 0);
    assert!(String::from_utf8_lossy(&output.stderr).contains("artifact"));
    let rows: Vec<(i32, String)> = fixture
        .target
        .client
        .query(
            &format!(
                "SELECT id,value FROM {} ORDER BY id",
                postgres::qualified(&fixture.name, &fixture.name)
            ),
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        rows,
        vec![
            (1, "first".into()),
            (2, "second".into()),
            (3, "owned locker".into())
        ],
        "only ACKed source prefix and independent locker survive; row4 never replays"
    );
    helper("free", Some(&volume.root));
    locker.close().await.unwrap();
    fixture.close().await;
}
