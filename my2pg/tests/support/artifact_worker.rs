//! Default-off test child; descriptor replacement is confined to this process.
use std::{
    fs,
    io::{self, Write},
    os::{
        fd::{AsRawFd, BorrowedFd, OwnedFd},
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::PathBuf,
    time::Duration,
};
unsafe extern "C" {
    fn dup2(old: i32, new: i32) -> i32;
}
fn fault(mode: String, target: PathBuf, arm: PathBuf, ready: PathBuf) -> io::Result<()> {
    while !arm.exists() {
        std::thread::sleep(Duration::from_millis(5));
    }
    let expected = fs::metadata(&target)?;
    for fd in 3..1024 {
        // Only borrow to duplicate a live descriptor; the duplicate is owned and closed.
        let Ok(owned) = (unsafe { BorrowedFd::borrow_raw(fd) }).try_clone_to_owned() else {
            continue;
        };
        let file = fs::File::from(owned);
        let Ok(actual) = file.metadata() else {
            continue;
        };
        if actual.dev() != expected.dev() || actual.ino() != expected.ino() {
            continue;
        }
        let (mut writer, reader) = UnixStream::pair()?;
        if mode == "stall" {
            writer.set_nonblocking(true)?;
            loop {
                match writer.write(&[0; 16384]) {
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }
            writer.set_nonblocking(false)?;
        } else if mode != "sync-failure" {
            return Err(io::Error::other("unknown test fault"));
        }
        // Replace exactly the verified reject metadata descriptor, never stdin/stdout.
        if unsafe { dup2(writer.as_raw_fd(), fd) } != fd {
            return Err(io::Error::last_os_error());
        }
        let probe: OwnedFd = writer.try_clone()?.into();
        let sync_errno = fs::File::from(probe)
            .sync_all()
            .unwrap_err()
            .raw_os_error()
            .unwrap();
        fs::write(
            ready,
            format!(
                "{}:{}:{}:{}",
                std::process::id(),
                expected.dev(),
                expected.ino(),
                sync_errno
            ),
        )?;
        let _owned_peer = reader;
        loop {
            std::thread::park();
        }
    }
    Err(io::Error::other("owned metadata descriptor not found"))
}
struct DelayAck {
    at: usize,
    count: usize,
    arm: PathBuf,
    ready: PathBuf,
}
impl Write for DelayAck {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.count += 1;
        if self.count == self.at {
            fs::write(&self.ready, b"durable response held")?;
            while !self.arm.exists() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        io::stdout().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|a| a == "fault") {
        assert_eq!(args.len(), 5);
        let mode = args[1].clone();
        let target = args[2].clone().into();
        let arm = args[3].clone().into();
        let ready = args[4].clone().into();
        if mode == "ack-delay" || mode == "create-ack-delay" {
            let output = DelayAck {
                at: if mode == "create-ack-delay" { 1 } else { 4 },
                count: 0,
                arm,
                ready,
            };
            if my2pg::report::artifact_io::serve_worker(io::stdin().lock(), output).is_err() {
                std::process::exit(1);
            }
            return;
        }
        std::thread::spawn(move || {
            if fault(mode, target, arm, ready).is_err() {
                std::process::exit(77);
            }
        });
    } else {
        assert_eq!(args.len(), 0);
    }
    if my2pg::report::artifact_io::worker_stdio().is_err() {
        std::process::exit(1);
    }
}
