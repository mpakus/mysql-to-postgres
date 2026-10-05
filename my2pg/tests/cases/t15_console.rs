use clap::Parser;
use my2pg::{
    cli::{Args, Command as CliCommand},
    config::{self, MigrationConfig},
    model::{RunReport, RunStatus},
    report::Console,
};
use std::{env, fs, path::PathBuf, process::Command};

// Actual stderr PTY, with stdout independently captured. The deadline owns and
// reaps the process even when a broken renderer leaves it blocked.
#[cfg(unix)]
const PROCESS: &str = r#"
import errno, fcntl, json, os, pty, select, struct, subprocess, sys, termios, time
mode, width = sys.argv[1], int(sys.argv[2])
master = slave = None
child = None
try:
    if mode == 'pty':
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, width, 0, 0))
    child = subprocess.Popen(sys.argv[3:], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=slave if slave is not None else subprocess.PIPE)
    if slave is not None:
        os.close(slave)
        slave = None
    descriptors = {child.stdout.fileno(): 'stdout',
        master if master is not None else child.stderr.fileno(): 'stderr'}
    buffers = {'stdout': bytearray(), 'stderr': bytearray()}
    deadline = time.monotonic() + 25
    while descriptors:
        if time.monotonic() >= deadline:
            raise TimeoutError('my2pg console process deadline')
        ready, _, _ = select.select(list(descriptors), [], [], .1)
        for fd in ready:
            try:
                data = os.read(fd, 65536)
            except OSError as error:
                if fd == master and error.errno == errno.EIO:
                    data = b''
                else:
                    raise
            if data:
                buffers[descriptors[fd]].extend(data)
                if sum(map(len, buffers.values())) > 4 * 1024 * 1024:
                    raise OverflowError('console output exceeded test bound')
            else:
                del descriptors[fd]
    code = child.wait(timeout=5)
    print(json.dumps({'code': code,
        'stdout': buffers['stdout'].decode('utf8'),
        'stderr': buffers['stderr'].decode('utf8')}))
finally:
    if child is not None and child.poll() is None:
        child.kill()
        child.wait(timeout=5)
    if slave is not None:
        os.close(slave)
    if master is not None:
        os.close(master)
"#;

#[derive(serde::Deserialize)]
struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

fn config(schema: &str, table: &str, directory: &std::path::Path) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":env::var("MY2PG_TLS_CA").expect("owned TLS fixture required")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":format!("{schema}_{}",std::process::id()),"ca_file":env::var("MY2PG_TLS_CA").unwrap()},
        "migration":{"batch_rows":1,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":[table]},
        "report":{"directory":directory.join("runs")}
    })).unwrap()
}

fn workspace(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let directory = PathBuf::from(env::var("MY2PG_ARTIFACT_DIR").unwrap()).join(format!(
        "t15-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    directory
}

#[cfg(unix)]
fn invoke(path: &std::path::Path, tty: bool, width: u16, args: &[&str], no_color: bool) -> Output {
    let mut command = Command::new("python3");
    command
        .args(["-c", PROCESS, if tty { "pty" } else { "pipe" }])
        .arg(width.to_string())
        .arg(env!("CARGO_BIN_EXE_my2pg"))
        .arg("run")
        .arg(path)
        .args(args)
        .env("TERM", "xterm")
        .env_remove("COLUMNS")
        .env_remove("NO_COLOR");
    if no_color {
        command.env("NO_COLOR", "1");
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "PTY helper must complete and reap its child"
    );
    let output: Output = serde_json::from_slice(&result.stdout).unwrap();
    for variable in ["MY2PG_MYSQL_URL", "MY2PG_POSTGRES_URL"] {
        let uri = env::var(variable).unwrap();
        let password = url::Url::parse(&uri)
            .unwrap()
            .password()
            .unwrap()
            .to_owned();
        assert!(!output.stdout.contains(&uri) && !output.stderr.contains(&uri));
        assert!(!output.stdout.contains(&password) && !output.stderr.contains(&password));
    }
    output
}

fn report(config: &MigrationConfig) -> RunReport {
    let directories: Vec<_> = fs::read_dir(&config.report.directory).unwrap().collect();
    assert_eq!(directories.len(), 1, "one exclusive artifact directory");
    serde_json::from_slice(
        &fs::read(directories[0].as_ref().unwrap().path().join("report.json")).unwrap(),
    )
    .unwrap()
}

async fn cleanup(config: &MigrationConfig, directory: PathBuf) {
    let target = my2pg::postgres::connect(&config.target, &env::var("MY2PG_POSTGRES_URL").unwrap())
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            my2pg::postgres::quote_ident(&config.target.schema)
        ))
        .await
        .unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned TLS databases and Python stdlib PTY"]
async fn real_pty_detects_stderr_and_respects_actual_narrow_width() {
    let controls = regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap();
    for width in [12, 1] {
        let directory = workspace(&format!("tty-{width}"));
        let configuration = config(&format!("t15_tty_{width}"), "all_bytes", &directory);
        let path = directory.join("migration.toml");
        fs::write(&path, toml::to_string(&configuration).unwrap()).unwrap();
        let output = invoke(&path, true, width, &["--color", "never"], false);
        assert_eq!(output.code, 0);
        assert!(output.stdout.is_empty());
        assert!(
            output.stderr.contains("\x1b[1A"),
            "actual stderr TTY redraw"
        );
        let text = controls.replace_all(&output.stderr, "");
        let text = text.replace('\r', "");
        // The artifact path is deliberately complete and therefore exempt from
        // truncation. Every other logical line honors even a one-column PTY.
        for line in text.lines().filter(|line| !line.starts_with("report: ")) {
            assert!(line.is_ascii());
            assert!(
                line.len() <= usize::from(width),
                "line exceeds real PTY width"
            );
        }
        assert!(!text.contains("100%"));
        assert_eq!(report(&configuration).tables[0].committed_rows, 1);
        cleanup(&configuration, directory).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned TLS databases and actual binary subprocesses"]
async fn redirected_plain_quiet_verbose_and_json_outputs_match_reports() {
    for (label, args) in [
        ("plain", vec!["--progress", "always", "--color", "auto"]),
        ("quiet", vec!["--quiet"]),
        ("verbose", vec!["--verbose"]),
        ("json", vec!["--output", "json", "--quiet"]),
    ] {
        let directory = workspace(label);
        let configuration = config(&format!("t15_redirect_{label}"), "all_bytes", &directory);
        let path = directory.join("migration.toml");
        fs::write(&path, toml::to_string(&configuration).unwrap()).unwrap();
        let output = invoke(&path, false, 80, &args, false);
        assert_eq!(output.code, 0);
        let persisted = report(&configuration);
        assert_eq!(persisted.tables[0].committed_rows, 1);
        if label == "json" {
            assert!(output.stderr.is_empty());
            let events: Vec<serde_json::Value> = output
                .stdout
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
            let final_event = events.last().unwrap();
            assert_eq!(final_event["version"], 1);
            assert_eq!(final_event["exit_code"], 0);
            assert_eq!(final_event["report"]["run_id"], persisted.run_id);
            assert_eq!(final_event["report"]["tables"][0]["committed_rows"], 1);
        } else {
            let human = output.stderr.replace('\n', "");
            assert!(output.stdout.is_empty());
            assert!(
                !output.stderr.contains('\x1b'),
                "redirects never cursor-control"
            );
            assert!(output.stderr.contains("complete:") && output.stderr.contains("report.json"));
            assert!(human.contains("unresolved=0") && human.contains("indeterminate=0"));
            if label == "quiet" {
                assert!(!output.stderr.contains("copy committed="));
            }
            if label == "verbose" {
                assert!(human.contains("table all_bytes ->"));
                assert!(human.contains("consistency=Frozen"));
            }
        }
        cleanup(&configuration, directory).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned TLS databases and Python stdlib PTY"]
async fn actual_failure_diagnostics_honor_color_no_color_and_verbose() {
    for (index, tty, color, no_color, expect_color) in [
        (0, true, "auto", false, true),
        (1, true, "always", true, false),
        (2, true, "never", false, false),
        (3, false, "auto", false, false),
        (4, false, "always", false, true),
    ] {
        let directory = workspace(&format!("failure-{index}"));
        let configuration = config(
            &format!("t15_failure_{index}"),
            "invalid_values",
            &directory,
        );
        let path = directory.join("migration.toml");
        fs::write(&path, toml::to_string(&configuration).unwrap()).unwrap();
        let output = invoke(
            &path,
            tty,
            160,
            &["--progress", "never", "--color", color, "--verbose"],
            no_color,
        );
        assert_eq!(output.code, 1);
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr.contains("\x1b[31m"), expect_color);
        assert!(
            !output.stderr.contains("\x1b[1A"),
            "progress never suppresses redraw"
        );
        assert!(output.stderr.contains("unresolved=1"));
        assert!(output.stderr.contains("indeterminate=0"));
        assert!(output.stderr.contains("CONVERSION [execution]"));
        assert!(output.stderr.contains("table invalid_values ->"));
        assert!(!output.stderr.contains("before") && !output.stderr.contains("after"));
        let persisted = report(&configuration);
        assert_eq!(persisted.status, RunStatus::Failed);
        assert_eq!(persisted.tables[0].unresolved_rows, 1);
        assert_eq!(persisted.tables[0].committed_rows, 0);
        cleanup(&configuration, directory).await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires owned TLS databases and actual binary subprocesses"]
async fn actual_json_table_metrics_are_scoped_and_acknowledged_copy_text() {
    let directory = workspace("metrics");
    let mut configuration = config("t15_metrics", "all_bytes", &directory);
    configuration.tables.include.push("keyless".into());
    let path = directory.join("migration.toml");
    fs::write(&path, toml::to_string(&configuration).unwrap()).unwrap();
    let output = invoke(&path, false, 80, &["--output", "json"], false);
    assert_eq!(output.code, 0);
    assert!(output.stderr.is_empty());
    let events: Vec<serde_json::Value> = output
        .stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let persisted = report(&configuration);
    // Independent fixture bytes: id1 + TAB + escaped \\x + 512 hex digits +
    // LF = 518 bytes. keyless has 3*("same"+TAB+digit+LF), NULL+TAB+0+LF,
    // and empty+TAB+0+LF = 29 bytes. No production encoder is the oracle.
    for (name, rows, bytes) in [("all_bytes", 1, 518), ("keyless", 5, 29)] {
        let complete = events
            .iter()
            .find(|event| event["kind"] == "table_complete" && event["table"] == name)
            .unwrap();
        assert_eq!(complete["committed_rows"], rows);
        assert_eq!(complete["committed_bytes"], bytes);
        let table = persisted
            .tables
            .iter()
            .find(|table| table.source_name == name)
            .unwrap();
        assert_eq!(table.committed_rows, rows);
        assert_eq!(table.committed_bytes, bytes);
    }
    let outcome = events.last().unwrap();
    assert_eq!(outcome["kind"], "outcome");
    assert_eq!(outcome["exit_code"], 0);
    assert_eq!(
        persisted
            .tables
            .iter()
            .map(|table| table.committed_bytes)
            .sum::<u64>(),
        547
    );
    cleanup(&configuration, directory).await;
}

#[test]
fn indeterminate_final_outcome_preserves_human_and_json_accounting() {
    let mut report: RunReport =
        serde_json::from_str(include_str!("../contracts/report.json")).unwrap();
    report.status = RunStatus::Indeterminate;
    report.tables[0].status = RunStatus::Indeterminate;
    report.tables[0].unresolved_rows = 2;
    report.tables[0].indeterminate_rows = 3;
    for format in ["text", "json"] {
        let args = Args::try_parse_from(["my2pg", "run", "m.toml", "--quiet", "--output", format])
            .unwrap();
        let CliCommand::Run(operation) = &args.command else {
            unreachable!()
        };
        let mut config = config::parse(
            "version=1\n[source]\nurl_env='SRC'\nconsistency='frozen'\n[target]\nurl_env='DST'\nschema='legacy'\n",
            std::path::Path::new("/tmp"),
        ).unwrap();
        args.apply(&mut config, operation).unwrap();
        let mut console = Console::new(&config, &args);
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        console
            .outcome_to(&report, &mut stdout, &mut stderr)
            .unwrap();
        if format == "json" {
            assert!(stderr.is_empty());
            let value: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
            assert_eq!(value["exit_code"], 1);
            assert_eq!(value["report"]["tables"][0]["indeterminate_rows"], 3);
        } else {
            assert!(stdout.is_empty());
            let output = String::from_utf8(stderr).unwrap().replace('\n', "");
            assert!(output.contains("indeterminate:"));
            assert!(output.contains("unresolved=2") && output.contains("indeterminate=3"));
            assert!(output.contains("exit=1"));
        }
    }
}
