#![cfg(all(unix, feature = "artifact-worker-tests"))]
use my2pg::{model::*, report::artifact_io::*};
use std::{fs, path::PathBuf, process::Command, sync::Arc, time::Duration};
#[cfg(target_os = "linux")]
use std::{
    fs::File,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::Path,
    process::{Child, Stdio},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(unix)]
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    os::{fd::AsRawFd, unix::process::CommandExt},
};
use tokio::{sync::Semaphore, time::Instant};
static ARTIFACT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(unix)]
struct SpawnGate(Option<UnixStream>);

#[cfg(unix)]
impl SpawnGate {
    fn release(&mut self) {
        if let Some(mut stream) = self.0.take() {
            let _ = stream.write_all(b"x");
        }
    }
}

#[cfg(unix)]
impl Drop for SpawnGate {
    fn drop(&mut self) {
        self.release();
    }
}
struct Fixture {
    root: PathBuf,
    identity: ArtifactIdentity,
    plan: MigrationPlan,
    report: RunReport,
    limits: ArtifactLimits,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "my2pg-artifact-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Self::new_at(root)
    }

    fn new_at(root: PathBuf) -> Self {
        let identity = ArtifactIdentity::new(&root).unwrap();
        let plan: MigrationPlan =
            serde_json::from_str(include_str!("../contracts/plan.json")).unwrap();
        let mut report: RunReport =
            serde_json::from_str(include_str!("../contracts/report.json")).unwrap();
        report.run_id = identity.run_id.clone();
        report.artifact_dir = identity.directory.to_str().unwrap().into();
        let limits = ArtifactLimits::derive(&identity, &plan, &report, 4096).unwrap();
        Self {
            root,
            identity,
            plan,
            report,
            limits,
        }
    }
    async fn start(&self, mode: Option<&str>) -> DurableArtifacts {
        let mut command = Command::new(env!("CARGO_BIN_EXE_my2pg-artifact-test-worker"));
        if let Some(mode) = mode {
            command
                .arg("fault")
                .arg(mode)
                .arg(
                    self.identity
                        .directory
                        .join("t_0000000000000001.reject.jsonl"),
                )
                .arg(self.root.join("arm"))
                .arg(self.root.join("ready"));
        }
        let ids = self
            .plan
            .tables
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match DurableArtifacts::start(
                command_for(&command),
                self.identity.clone(),
                self.limits,
                &ids,
                memory_for(self.limits).await,
            )
            .await
            {
                Ok(writer) => {
                    return writer;
                }
                Err(e) if e.kind == ArtifactFailure::Busy && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(e) => panic!("writer start: {e}"),
            }
        }
    }
}

#[cfg(target_os = "linux")]
struct StalledFuse {
    root: PathBuf,
    mount: PathBuf,
    daemon: Option<Child>,
}

#[cfg(target_os = "linux")]
impl StalledFuse {
    fn new() -> Self {
        assert!(
            fs::metadata("/dev/fuse")
                .unwrap()
                .file_type()
                .is_char_device(),
            "missing /dev/fuse"
        );
        let root = std::env::temp_dir().join(format!(
            "my2pg-t13-fuse-stall-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let mount = root.join("mount");
        let backing = root.join("backing");
        fs::create_dir(&mount).unwrap();
        fs::create_dir(&backing).unwrap();
        let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/t12_fuse_eio.c");
        let binary = root.join("fuse-stall");
        let compiled = Command::new("cc")
            .args(["-D_FILE_OFFSET_BITS=64", "-I/usr/include/fuse3"])
            .arg(&helper)
            .args(["-o"])
            .arg(&binary)
            .args(["-lfuse3", "-lpthread"])
            .output()
            .expect("C compiler and libfuse3 are required");
        assert!(
            compiled.status.success(),
            "FUSE helper compile failed: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let log = File::create(root.join("daemon.log")).unwrap();
        let daemon = Command::new(binary)
            .arg(&mount)
            .arg(&backing)
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let mut volume = Self {
            root,
            mount,
            daemon: Some(daemon),
        };
        volume.wait_for_mount();
        volume
    }

    fn daemon_pid(&self) -> u32 {
        self.daemon.as_ref().unwrap().id()
    }

    fn arm(&self) {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        assert_eq!(unsafe { kill(self.daemon_pid() as i32, 12) }, 0);
    }

    async fn wait_until_stalled(&self) {
        let marker = self.root.join("backing/stalled");
        tokio::time::timeout(Duration::from_secs(3), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("production report fsync never entered the stalled FUSE callback");
    }

    fn release(&self) {
        fs::write(self.root.join("backing/release"), b"release\n").unwrap();
    }

    fn wait_for_mount(&mut self) {
        for _ in 0..100 {
            if mount_identity(&self.root, &self.mount, self.daemon_pid()) {
                return;
            }
            if let Some(status) = self.daemon.as_mut().unwrap().try_wait().unwrap() {
                panic!(
                    "FUSE daemon exited before mounting ({status}): {}",
                    fs::read_to_string(self.root.join("daemon.log")).unwrap_or_default()
                );
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "FUSE mount did not appear: {}",
            fs::read_to_string(self.root.join("daemon.log")).unwrap_or_default()
        );
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        if !self.root.exists() {
            return Ok(());
        }
        self.release();
        if mount_identity(&self.root, &self.mount, self.daemon_pid()) {
            let status = Command::new("fusermount3")
                .args(["-u"])
                .arg(&self.mount)
                .status()?;
            if !status.success() {
                return Err(std::io::Error::other(
                    "could not unmount owned T13 FUSE mount",
                ));
            }
        }
        if mount_at(&self.mount) {
            return Err(std::io::Error::other(
                "unexpected mount remains at owned T13 FUSE mountpoint",
            ));
        }
        if let Some(daemon) = self.daemon.as_mut() {
            if daemon.try_wait()?.is_none() {
                daemon.kill()?;
            }
            let _ = daemon.wait()?;
        }
        fs::remove_dir_all(&self.root)
    }
}

#[cfg(target_os = "linux")]
impl Drop for StalledFuse {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("owned T13 FUSE cleanup failed: {error}");
        }
    }
}

#[cfg(target_os = "linux")]
fn mount_at(mount: &Path) -> bool {
    let Ok(info) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let mount = mount.to_string_lossy();
    info.lines()
        .any(|line| line.split_whitespace().nth(4) == Some(mount.as_ref()))
}

#[cfg(target_os = "linux")]
fn mount_identity(root: &Path, mount: &Path, daemon_pid: u32) -> bool {
    let Ok(info) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let mount_text = mount.to_string_lossy();
    let Some(line) = info
        .lines()
        .find(|line| line.split_whitespace().nth(4) == Some(mount_text.as_ref()))
    else {
        return false;
    };
    let Some((_, filesystem)) = line.split_once(" - ") else {
        return false;
    };
    let mut fields = filesystem.split_whitespace();
    let kind = fields.next().unwrap_or_default();
    let source = fields.next().unwrap_or_default();
    matches!(
        (fs::metadata(mount).map(|metadata| metadata.dev()), fs::metadata(root).map(|metadata| metadata.dev())),
        (Ok(actual), Ok(parent)) if actual != parent
    ) && kind.starts_with("fuse")
        && source == format!("my2pg-t12-eio-{daemon_pid}")
}
fn command_for(command: &Command) -> Command {
    let mut fresh = Command::new(command.get_program());
    fresh.args(command.get_args());
    fresh
}
async fn memory_for(limits: ArtifactLimits) -> tokio::sync::OwnedSemaphorePermit {
    Arc::new(Semaphore::new(limits.reservation_bytes))
        .acquire_many_owned(limits.reservation_bytes as u32)
        .await
        .unwrap()
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("preserved failed artifact fixture: {}", self.root.display());
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
fn reason() -> Diagnostic {
    Diagnostic {
        code: "ROW_CONVERSION".into(),
        stage: "conversion".into(),
        object: None,
        severity: Severity::Warning,
        message: "finite fixture reason".into(),
    }
}
async fn raw(values: Vec<RawValue>) -> RawRetention {
    RawRetention {
        values,
        permit: Arc::new(
            Arc::new(Semaphore::new(4096))
                .acquire_many_owned(4096)
                .await
                .unwrap(),
        ),
    }
}
async fn reaped(writer: &DurableArtifacts) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while writer.confirmed().unreaped_pid.is_some() {
        assert!(Instant::now() < deadline, "owned child did not reap");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn armed(fixture: &Fixture) {
    fs::write(fixture.root.join("arm"), b"arm").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fs::read_to_string(fixture.root.join("ready")).is_ok_and(|text| {
        text.split(':').count() == 4 && text.split(':').all(|field| field.parse::<u64>().is_ok())
    }) {
        assert!(
            Instant::now() < deadline,
            "native descriptor replacement failed"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        fs::read_to_string(fixture.root.join("ready"))
            .unwrap()
            .split(':')
            .count(),
        4
    );
}
#[tokio::test]
async fn durable_writer_prefix_generation_bounds_and_native_descriptor_failures() {
    let _serial = ARTIFACT_TEST_LOCK.lock().await;
    let fixture = Fixture::new();
    let insufficient = Arc::new(Semaphore::new(fixture.limits.reservation_bytes - 1))
        .acquire_many_owned((fixture.limits.reservation_bytes - 1) as u32)
        .await
        .unwrap();
    let ids = fixture
        .plan
        .tables
        .iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>();
    let denied = DurableArtifacts::start(
        Command::new(env!("CARGO_BIN_EXE_my2pg-artifact-test-worker")),
        fixture.identity.clone(),
        fixture.limits,
        &ids,
        insufficient,
    )
    .await;
    assert!(matches!(
        denied,
        Err(ArtifactError {
            kind: ArtifactFailure::Bounds,
            ..
        })
    ));
    assert!(!fixture.identity.directory.exists());
    let writer = fixture.start(None).await;
    let busy = DurableArtifacts::start(
        Command::new(env!("CARGO_BIN_EXE_my2pg-artifact-test-worker")),
        fixture.identity.clone(),
        fixture.limits,
        &ids,
        memory_for(fixture.limits).await,
    )
    .await;
    assert!(matches!(
        busy,
        Err(ArtifactError {
            kind: ArtifactFailure::Busy,
            ..
        })
    ));
    writer
        .lease()
        .await
        .unwrap()
        .write_plan(&fixture.plan)
        .await
        .unwrap();
    writer
        .lease()
        .await
        .unwrap()
        .write_report(1, &fixture.report)
        .await
        .unwrap();
    let budget = OwnedRejectBudget::new(3);
    let locator = RowLocator {
        ordinal: 1,
        key: None,
    };
    let diagnostic = reason();
    writer
        .lease()
        .await
        .unwrap()
        .reject_conversion(
            0,
            &locator,
            raw(vec![
                RawValue::Float(f32::from_bits(0x7fc01234)),
                RawValue::Double(f64::from_bits(0xfff0000000000000)),
                RawValue::Bytes(vec![0, 255]),
            ])
            .await,
            &diagnostic,
            budget.reserve().unwrap(),
        )
        .await
        .unwrap();
    let storage = Arc::new(BatchStorage {
        bytes: bytes::Bytes::from_static(
            b"first
second
",
        ),
        permit: Arc::new(Semaphore::new(64))
            .acquire_many_owned(64)
            .await
            .unwrap(),
    });
    let receipt = writer
        .lease()
        .await
        .unwrap()
        .reject_copy(
            0,
            &locator,
            storage,
            6..13,
            &diagnostic,
            budget.reserve().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.copy_offset, 7);
    assert_eq!(budget.used(), 2);
    let ledger = writer.confirmed();
    assert_eq!(ledger.tables[0].rejected_rows, 2);
    assert_eq!(ledger.tables[0].copy_offset, 7);
    assert!(ledger.pending_sequence.is_none());
    assert_eq!(
        fs::read(
            fixture
                .identity
                .directory
                .join("t_0000000000000001.reject.copy")
        )
        .unwrap(),
        b"second
"
    );
    let lines = fs::read_to_string(
        fixture
            .identity
            .directory
            .join("t_0000000000000001.reject.jsonl"),
    )
    .unwrap();
    let records = lines
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records[0]["values"][0]["bits"].as_u64(), Some(0x7fc01234));
    assert_eq!(
        records[0]["values"][1]["bits"].as_u64(),
        Some(0xfff0000000000000)
    );
    assert!(lines.contains("00ff"));
    assert_eq!(lines.lines().count(), 2);
    let confirmed = fixture
        .identity
        .directory
        .join("report.00000000000000000001.json");
    assert!(confirmed.exists());
    let mut oversized = fixture.report.clone();
    oversized.diagnostics.push(Diagnostic {
        message: "x".repeat(fixture.limits.payload_bytes),
        ..diagnostic.clone()
    });
    let overflow = writer
        .lease()
        .await
        .unwrap()
        .write_report(2, &oversized)
        .await
        .unwrap_err();
    assert_eq!(overflow.kind, ArtifactFailure::Bounds);
    assert_eq!(overflow.confirmed_generation, 1);
    assert_eq!(writer.confirmed().report_generation, 1);
    assert!(confirmed.exists());
    writer
        .lease()
        .await
        .unwrap()
        .write_report(2, &fixture.report)
        .await
        .unwrap();
    assert!(confirmed.exists());
    writer.flush().await.unwrap();
    assert!(!confirmed.exists());
    assert!(
        fixture
            .identity
            .directory
            .join("report.00000000000000000002.json")
            .exists()
    );
    writer
        .shutdown(Instant::now() + Duration::from_secs(1))
        .await;
    reaped(&writer).await;
    drop(writer);
    drop(fixture);
    for mode in ["stall", "sync-failure"] {
        let fixture = Fixture::new();
        let writer = fixture.start(Some(mode)).await;
        writer
            .lease()
            .await
            .unwrap()
            .write_report(1, &fixture.report)
            .await
            .unwrap();
        let budget = OwnedRejectBudget::new(2);
        writer
            .lease()
            .await
            .unwrap()
            .reject_conversion(
                0,
                &locator,
                raw(vec![RawValue::Int(1)]).await,
                &diagnostic,
                budget.reserve().unwrap(),
            )
            .await
            .unwrap();
        armed(&fixture).await;
        let retention = raw(vec![RawValue::Int(2)]).await;
        let weak = Arc::downgrade(&retention.permit);
        let operation = writer.lease().await.unwrap().reject_conversion(
            0,
            &locator,
            retention,
            &diagnostic,
            budget.reserve().unwrap(),
        );
        if mode == "stall" {
            assert!(
                tokio::time::timeout(Duration::from_millis(50), operation)
                    .await
                    .is_err()
            );
            assert!(
                weak.upgrade().is_some(),
                "cancelled waiter released accepted raw permit"
            );
            assert_eq!(budget.used(), 2);
            assert_eq!(writer.confirmed().tables[0].rejected_rows, 1);
            assert!(writer.confirmed().pending_sequence.is_some());
            let started = Instant::now();
            writer
                .shutdown(Instant::now() + Duration::from_millis(100))
                .await;
            assert!(started.elapsed() < Duration::from_millis(500));
        } else {
            let error = tokio::time::timeout(Duration::from_secs(2), operation)
                .await
                .unwrap()
                .unwrap_err();
            assert_eq!(error.kind, ArtifactFailure::Io);
            let ready = fs::read_to_string(fixture.root.join("ready")).unwrap();
            assert_eq!(
                error.errno,
                Some(ready.split(':').nth(3).unwrap().parse().unwrap())
            );
            writer
                .shutdown(Instant::now() + Duration::from_secs(1))
                .await;
        }
        reaped(&writer).await;
        let deadline = Instant::now() + Duration::from_secs(2);
        while weak.upgrade().is_some() || budget.used() != 1 {
            assert!(
                Instant::now() < deadline,
                "reject retention did not release; budget used {}",
                budget.used()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(budget.used(), 1);
        assert_eq!(writer.confirmed().tables[0].rejected_rows, 1);
        assert_eq!(writer.confirmed().report_generation, 1);
        assert!(
            fixture
                .identity
                .directory
                .join("report.00000000000000000001.json")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(
                fixture
                    .identity
                    .directory
                    .join("t_0000000000000001.reject.jsonl")
            )
            .unwrap()
            .lines()
            .count(),
            1
        );
    }
    // Durable ACK remains owned by the actor after the caller drops its receipt.
    let fixture = Fixture::new();
    let writer = fixture.start(Some("ack-delay")).await;
    writer
        .lease()
        .await
        .unwrap()
        .write_report(1, &fixture.report)
        .await
        .unwrap();
    let budget = OwnedRejectBudget::new(2);
    writer
        .lease()
        .await
        .unwrap()
        .reject_conversion(
            0,
            &locator,
            raw(vec![RawValue::Int(1)]).await,
            &diagnostic,
            budget.reserve().unwrap(),
        )
        .await
        .unwrap();
    let retention = raw(vec![RawValue::Int(2)]).await;
    let weak = Arc::downgrade(&retention.permit);
    let operation = writer.lease().await.unwrap().reject_conversion(
        0,
        &locator,
        retention,
        &diagnostic,
        budget.reserve().unwrap(),
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), operation)
            .await
            .is_err()
    );
    assert!(fixture.root.join("ready").exists());
    assert_eq!(writer.confirmed().tables[0].rejected_rows, 1);
    assert!(weak.upgrade().is_some());
    assert!(budget.reserve().is_none());
    fs::write(fixture.root.join("arm"), b"release ACK").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while writer.confirmed().tables[0].rejected_rows != 2 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(budget.used(), 2);
    assert!(writer.confirmed().pending_sequence.is_none());
    assert!(weak.upgrade().is_none());
    writer
        .shutdown(Instant::now() + Duration::from_secs(1))
        .await;
    reaped(&writer).await;
    drop(writer);
    drop(fixture);
    // Runtime teardown cannot discard global child ownership or accepted row bytes.
    let (fixture, writer, budget, weak) = std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let fixture = Fixture::new();
            let writer = fixture.start(Some("stall")).await;
            writer
                .lease()
                .await
                .unwrap()
                .write_report(1, &fixture.report)
                .await
                .unwrap();
            let budget = OwnedRejectBudget::new(2);
            let locator = RowLocator {
                ordinal: 1,
                key: None,
            };
            let diagnostic = reason();
            writer
                .lease()
                .await
                .unwrap()
                .reject_conversion(
                    0,
                    &locator,
                    raw(vec![RawValue::Int(1)]).await,
                    &diagnostic,
                    budget.reserve().unwrap(),
                )
                .await
                .unwrap();
            armed(&fixture).await;
            let retention = raw(vec![RawValue::Int(2)]).await;
            let weak = Arc::downgrade(&retention.permit);
            let operation = writer.lease().await.unwrap().reject_conversion(
                0,
                &locator,
                retention,
                &diagnostic,
                budget.reserve().unwrap(),
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), operation)
                    .await
                    .is_err()
            );
            assert!(weak.upgrade().is_some());
            (fixture, writer, budget, weak)
        })
    })
    .join()
    .unwrap();
    reaped(&writer).await;
    let deadline = Instant::now() + Duration::from_secs(2);
    while weak.upgrade().is_some() || budget.used() != 1 {
        assert!(
            Instant::now() < deadline,
            "runtime-teardown reject retention did not release; budget used {}",
            budget.used()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(budget.used(), 1);
    assert_eq!(writer.confirmed().tables[0].rejected_rows, 1);
    drop(writer);
    drop(fixture);
    let fixture = Fixture::new();
    {
        let mut command = Command::new(env!("CARGO_BIN_EXE_my2pg-artifact-test-worker"));
        command
            .arg("fault")
            .arg("create-ack-delay")
            .arg(fixture.identity.directory.join("unused"))
            .arg(fixture.root.join("arm"))
            .arg(fixture.root.join("ready"));
        let ids = fixture
            .plan
            .tables
            .iter()
            .map(|table| table.id.clone())
            .collect::<Vec<_>>();
        let start = DurableArtifacts::start(
            command,
            fixture.identity.clone(),
            fixture.limits,
            &ids,
            memory_for(fixture.limits).await,
        );
        tokio::pin!(start);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            tokio::select! {result=&mut start=>panic!("initial ACK unexpectedly returned {}",result.is_ok()),_=tokio::time::sleep(Duration::from_millis(5))=>{assert!(Instant::now()<deadline);if fixture.root.join("ready").exists(){break;}}}
        }
        let state = live_worker().unwrap().unwrap();
        assert!(state.unreaped_pid.is_some());
        assert_eq!(state.report_generation, 0);
        assert_eq!(state.pending_sequence, Some(1));
        // Drop the entire start future; its internally owned facade kills the child.
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while live_worker().unwrap().is_some() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(fixture.identity.directory.exists());
    assert!(!fixture.identity.directory.join("report.json").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_blocking_spawn_keeps_slot_owned_until_exact_child_is_reaped() {
    let _serial = ARTIFACT_TEST_LOCK.lock().await;
    unsafe extern "C" {
        fn read(fd: i32, buffer: *mut std::ffi::c_void, count: usize) -> isize;
        fn write(fd: i32, buffer: *const std::ffi::c_void, count: usize) -> isize;
    }

    let fixture = Fixture::new();
    let (notify_child, mut notify_parent) = UnixStream::pair().unwrap();
    let (gate_parent, gate_child) = UnixStream::pair().unwrap();
    notify_parent.set_nonblocking(true).unwrap();
    let notify_fd = notify_child.as_raw_fd();
    let gate_fd = gate_child.as_raw_fd();
    let mut command = Command::new(env!("CARGO_BIN_EXE_my2pg-artifact-test-worker"));
    unsafe {
        command.pre_exec(move || {
            let byte = b"x";
            let mut received = 0_u8;
            let _ = write(notify_fd, byte.as_ptr().cast(), 1);
            let _ = read(gate_fd, (&mut received as *mut u8).cast(), 1);
            Ok(())
        });
    }
    let mut gate = SpawnGate(Some(gate_parent));
    let ids = fixture
        .plan
        .tables
        .iter()
        .map(|table| table.id.clone())
        .collect::<Vec<_>>();
    let deadline = Instant::now() + Duration::from_secs(3);
    {
        let start = DurableArtifacts::start(
            command,
            fixture.identity.clone(),
            fixture.limits,
            &ids,
            memory_for(fixture.limits).await,
        );
        tokio::pin!(start);
        loop {
            let mut byte = [0_u8; 1];
            match notify_parent.read(&mut byte) {
                Ok(1) => break,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("spawn handshake notification: {error}"),
            }
            assert!(Instant::now() < deadline, "spawn hook did not start");
            tokio::select! {
                result = &mut start => panic!("blocked startup unexpectedly returned: {}", result.is_ok()),
                _ = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
    }
    drop(notify_child);
    drop(gate_child);
    let state = live_worker().unwrap().expect("starting slot missing");
    assert!(state.starting);
    assert_eq!(state.unreaped_pid, None);
    let busy = DurableArtifacts::start(
        Command::new(env!("CARGO_BIN_EXE_my2pg-artifact-test-worker")),
        fixture.identity.clone(),
        fixture.limits,
        &ids,
        memory_for(fixture.limits).await,
    )
    .await;
    assert!(matches!(
        busy,
        Err(ArtifactError {
            kind: ArtifactFailure::Busy,
            ..
        })
    ));

    // The blocked native spawn must not block this executor or the lifecycle
    // observer. Cancellation records intent while the reaper retains ownership.
    tokio::time::timeout(Duration::from_millis(250), async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(live_worker().unwrap().is_some());
    })
    .await
    .expect("executor stalled during native process spawn");
    let still_owned = live_worker()
        .unwrap()
        .expect("cancelled spawn slot disappeared");
    assert!(still_owned.starting);

    gate.release();
    let deadline = Instant::now() + Duration::from_secs(3);
    while live_worker().unwrap().is_some() {
        assert!(Instant::now() < deadline, "cancelled worker was not reaped");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[test]
fn worker_rejects_malformed_and_short_frames_without_ack() {
    for bytes in [vec![0; 80], b"M2PGART1".to_vec()] {
        let mut output = Vec::new();
        assert!(serve_worker(&bytes[..], &mut output).is_err());
        assert!(output.is_empty());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires a disposable privileged Linux container with /dev/fuse and libfuse3"]
async fn production_worker_stalled_fsync_returns_only_the_confirmed_report() {
    let _serial = ARTIFACT_TEST_LOCK.lock().await;
    let mut volume = StalledFuse::new();
    let fixture = Fixture::new_at(volume.mount.join("artifacts"));
    let writer = fixture.start(None).await;
    let report = fixture.report.clone();
    let first = writer
        .lease()
        .await
        .unwrap()
        .write_report(1, &report)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "initial production report write failed: {error}; FUSE log: {}",
                fs::read_to_string(volume.root.join("daemon.log")).unwrap_or_default()
            )
        });
    let report_path = fixture.identity.directory.join("report.json");
    let immutable_path = fixture
        .identity
        .directory
        .join("report.00000000000000000001.json");
    let durable_before = fs::read(&report_path).unwrap();
    assert_eq!(fs::read(&immutable_path).unwrap(), durable_before);

    volume.arm();
    {
        let mut next_report = report;
        next_report.elapsed_millis += 1;
        let operation = writer.lease().await.unwrap().write_report(2, &next_report);
        tokio::pin!(operation);
        tokio::select! {
            result = &mut operation => panic!("stalled report unexpectedly completed: {}", result.is_ok()),
            _ = volume.wait_until_stalled() => {}
        }

        let started = Instant::now();
        let confirmed = writer
            .shutdown(Instant::now() + Duration::from_millis(150))
            .await;
        assert!(
            started.elapsed() < Duration::from_millis(750),
            "shutdown waited on a stalled production filesystem syscall"
        );
        assert_eq!(confirmed.report_generation, 1);
        assert_eq!(confirmed.sequence, first.sequence);
        assert_eq!(confirmed.pending_sequence, Some(first.sequence + 1));
        assert_eq!(fs::read(&report_path).unwrap(), durable_before);
        assert_eq!(fs::read(&immutable_path).unwrap(), durable_before);

        volume.release();
        let result = tokio::time::timeout(Duration::from_secs(3), &mut operation)
            .await
            .expect("accepted report operation remained stuck after FUSE release");
        assert!(
            result.is_err(),
            "a post-timeout operation was unexpectedly ACKed"
        );
    }
    reaped(&writer).await;
    assert_eq!(fs::read(&report_path).unwrap(), durable_before);
    assert_eq!(fs::read(&immutable_path).unwrap(), durable_before);
    drop(writer);
    drop(fixture);
    volume.cleanup().unwrap();
}
