//! Linux-only real FUSE `fsync(EIO)` acceptance; run inside a privileged Linux container.
#![cfg(target_os = "linux")]

use my2pg::{model::RunReport, report::RunArtifacts};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct FuseVolume {
    root: PathBuf,
    mount: PathBuf,
    daemon: Option<Child>,
}

impl FuseVolume {
    fn new() -> Self {
        assert!(
            fs::metadata("/dev/fuse")
                .unwrap()
                .file_type()
                .is_char_device(),
            "missing /dev/fuse"
        );
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "my2pg-t12-fuse-eio-{}-{nonce:x}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let mount = root.join("mount");
        let mut volume = Self {
            root,
            mount,
            daemon: None,
        };
        fs::set_permissions(&volume.root, fs::Permissions::from_mode(0o700)).unwrap();
        let backing = volume.root.join("backing");
        fs::create_dir(&volume.mount).unwrap();
        fs::create_dir(&backing).unwrap();
        let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/t12_fuse_eio.c");
        let binary = volume.root.join("fuse-eio");
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
        let log = File::create(volume.root.join("daemon.log")).unwrap();
        let daemon = Command::new(binary)
            .arg(&volume.mount)
            .arg(&backing)
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        volume.daemon = Some(daemon);
        volume.wait_for_mount();
        volume
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

    fn daemon_pid(&self) -> u32 {
        self.daemon.as_ref().unwrap().id()
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        if !self.root.exists() {
            return Ok(());
        }
        let expected_prefix = format!("my2pg-t12-fuse-eio-{}-", std::process::id());
        if self.root.parent() != Some(std::env::temp_dir().as_path())
            || !self
                .root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&expected_prefix)
            || fs::symlink_metadata(&self.root)?.file_type().is_symlink()
        {
            return Err(std::io::Error::other(
                "refusing cleanup outside owned FUSE root",
            ));
        }
        if self
            .daemon
            .as_ref()
            .is_some_and(|daemon| mount_identity(&self.root, &self.mount, daemon.id()))
        {
            let unmounted = Command::new("fusermount3")
                .args(["-u"])
                .arg(&self.mount)
                .status()?;
            if !unmounted.success() {
                return Err(std::io::Error::other(
                    "could not unmount owned FUSE fixture",
                ));
            }
        }
        if mount_at(&self.mount) {
            return Err(std::io::Error::other(
                "unexpected mount remains at owned mountpoint",
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

impl Drop for FuseVolume {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("owned T12 FUSE cleanup failed: {error}");
        }
    }
}

fn mount_at(mount: &Path) -> bool {
    let Ok(info) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let mount = mount.to_string_lossy();
    info.lines()
        .any(|line| line.split_whitespace().nth(4) == Some(mount.as_ref()))
}

fn mount_identity(root: &Path, mount: &Path, daemon_pid: u32) -> bool {
    let Ok(info) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let mount = mount.to_string_lossy();
    let Some(line) = info
        .lines()
        .find(|line| line.split_whitespace().nth(4) == Some(mount.as_ref()))
    else {
        return false;
    };
    let Some((_, filesystem)) = line.split_once(" - ") else {
        return false;
    };
    let mut filesystem_fields = filesystem.split_whitespace();
    let filesystem_type = filesystem_fields.next().unwrap_or_default();
    let source = filesystem_fields.next().unwrap_or_default();
    let actual_mount = fs::metadata(mount.as_ref()).map(|metadata| metadata.dev());
    let parent_device = fs::metadata(root).map(|metadata| metadata.dev());
    matches!((actual_mount, parent_device), (Ok(actual), Ok(parent)) if actual != parent)
        && filesystem_type.starts_with("fuse")
        && source == format!("my2pg-t12-eio-{daemon_pid}")
}

#[test]
#[ignore = "requires a disposable privileged Linux container with /dev/fuse and libfuse3"]
fn linux_fuse_fsync_eio_preserves_the_last_durable_report() {
    let mut volume = FuseVolume::new();
    assert!(mount_identity(
        &volume.root,
        &volume.mount,
        volume.daemon_pid()
    ));

    let probe = volume.mount.join("probe-eio");
    let mut probe_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .unwrap();
    probe_file.write_all(b"raw fsync probe\n").unwrap();
    assert_eq!(
        probe_file.sync_all().unwrap_err().raw_os_error(),
        Some(5),
        "kernel fsync syscall must return the FUSE callback's EIO"
    );
    drop(probe_file);
    fs::remove_file(probe).unwrap();

    let artifacts = RunArtifacts::create(&volume.mount.join("runs")).unwrap();
    let mut report: RunReport =
        serde_json::from_str(include_str!("../contracts/report.json")).unwrap();
    report.run_id = artifacts.run_id.clone();
    report.artifact_dir = artifacts.directory.to_string_lossy().into_owned();
    artifacts.write_report(&report).unwrap();
    let report_path = artifacts.directory.join("report.json");
    assert_ne!(
        fs::metadata(&report_path).unwrap().dev(),
        fs::metadata(&volume.root).unwrap().dev(),
        "report must be on the FUSE mount"
    );
    let durable_before = fs::read(&report_path).unwrap();

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // SIGUSR1 arms one EIO for the next report.json.tmp fsync callback.
    assert_eq!(unsafe { kill(volume.daemon_pid() as i32, 10) }, 0);
    report.elapsed_millis += 1;
    let failure = artifacts.write_report(&report).unwrap_err();
    assert_eq!(
        failure.raw_os_error(),
        Some(5),
        "RunArtifacts::write_report must expose the physical fsync EIO"
    );
    assert_eq!(fs::read(&report_path).unwrap(), durable_before);
    assert!(!artifacts.directory.join("report.json.tmp").exists());
    volume.cleanup().unwrap();
}
