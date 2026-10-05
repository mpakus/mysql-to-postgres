use crate::{
    cli::{Args, Color},
    config::{MigrationConfig, OutputFormat, ProgressPolicy},
    model::{EstimateQuality, RunEvent, RunReport, RunStatus},
};
use regex::Regex;
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal, Write},
    sync::OnceLock,
    time::{Duration, Instant},
};

pub struct Console {
    output: OutputFormat,
    progress: bool,
    color: bool,
    quiet: bool,
    verbose: bool,
    width: usize,
    last_draw: Option<Instant>,
    drawn_lines: usize,
    tables: BTreeMap<String, String>,
}

impl Console {
    pub fn new(config: &MigrationConfig, args: &Args) -> Self {
        let terminal =
            io::stderr().is_terminal() && std::env::var("TERM").is_ok_and(|term| term != "dumb");
        let progress = terminal
            && config.report.console == OutputFormat::Text
            && config.report.progress != ProgressPolicy::Never
            && !args.quiet;
        let color = std::env::var_os("NO_COLOR").is_none()
            && match args.color {
                Color::Auto => terminal,
                Color::Always => true,
                Color::Never => false,
            };
        let width = terminal_width();
        Self {
            output: config.report.console,
            progress,
            color,
            quiet: args.quiet,
            verbose: args.verbose,
            width,
            last_draw: None,
            drawn_lines: 0,
            tables: BTreeMap::new(),
        }
    }

    pub fn emit(&mut self, event: &RunEvent) -> io::Result<()> {
        self.emit_to(event, &mut io::stdout().lock(), &mut io::stderr().lock())
    }

    pub fn phase_interval(
        &mut self,
        run_id: &str,
        phase: &str,
        start_elapsed_millis: u64,
        end_elapsed_millis: u64,
        operation_classes: &[&str],
    ) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        self.phase_interval_to(
            run_id,
            phase,
            start_elapsed_millis,
            end_elapsed_millis,
            operation_classes,
            &mut stdout,
        )
    }

    pub fn phase_interval_to(
        &mut self,
        run_id: &str,
        phase: &str,
        start_elapsed_millis: u64,
        end_elapsed_millis: u64,
        operation_classes: &[&str],
        stdout: &mut impl Write,
    ) -> io::Result<()> {
        if self.output != OutputFormat::Json {
            return Ok(());
        }
        #[derive(serde::Serialize)]
        struct Event<'a> {
            version: u32,
            kind: &'static str,
            run_id: &'a str,
            phase: &'a str,
            clock: &'static str,
            scope: &'static str,
            start_elapsed_millis: u64,
            end_elapsed_millis: u64,
            operation_classes: &'a [&'a str],
        }
        serde_json::to_writer(
            &mut *stdout,
            &Event {
                version: 1,
                kind: "phase_interval",
                run_id,
                phase,
                clock: "run_monotonic_millis",
                scope: "run",
                start_elapsed_millis,
                end_elapsed_millis,
                operation_classes,
            },
        )?;
        stdout.write_all(b"\n")?;
        stdout.flush()
    }

    pub fn outcome(&mut self, report: &RunReport) -> io::Result<()> {
        self.outcome_to(report, &mut io::stdout().lock(), &mut io::stderr().lock())
    }

    pub fn outcome_to(
        &mut self,
        report: &RunReport,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> io::Result<()> {
        let mut safe = report.clone();
        for diagnostic in &mut safe.diagnostics {
            diagnostic.message = redact(&diagnostic.message);
            diagnostic.object = diagnostic.object.as_ref().map(|object| visible(object));
        }
        safe.failed_steps = safe.failed_steps.iter().map(|step| redact(step)).collect();
        if self.output == OutputFormat::Json {
            #[derive(serde::Serialize)]
            struct Outcome<'a> {
                version: u32,
                kind: &'static str,
                exit_code: u8,
                report: &'a RunReport,
            }
            serde_json::to_writer(
                &mut *stdout,
                &Outcome {
                    version: 1,
                    kind: "outcome",
                    exit_code: safe.exit_code(),
                    report: &safe,
                },
            )?;
            stdout.write_all(b"\n")?;
            return stdout.flush();
        }
        if self.progress {
            self.clear(stderr)?;
        }
        let committed: u128 = safe
            .tables
            .iter()
            .map(|table| u128::from(table.committed_rows))
            .sum();
        let rejected: u128 = safe
            .tables
            .iter()
            .map(|table| u128::from(table.rejected_rows))
            .sum();
        let committed_bytes: u128 = safe
            .tables
            .iter()
            .map(|table| u128::from(table.committed_bytes))
            .sum();
        let unresolved: u128 = safe
            .tables
            .iter()
            .map(|table| u128::from(table.unresolved_rows))
            .sum();
        let indeterminate: u128 = safe
            .tables
            .iter()
            .map(|table| u128::from(table.indeterminate_rows))
            .sum();
        let failed = safe
            .tables
            .iter()
            .filter(|table| matches!(table.status, RunStatus::Failed | RunStatus::Indeterminate))
            .count();
        let status = match safe.status {
            RunStatus::Running => "running",
            RunStatus::Complete => "complete",
            RunStatus::Partial => "partial",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Indeterminate => "indeterminate",
        };
        let summary = format!(
            "{status}: mode={:?} committed={committed} COPY-text-bytes={committed_bytes} rejected={rejected} unresolved={unresolved} indeterminate={indeterminate} failed_tables={failed} failed_steps={} exit={}",
            safe.mode,
            safe.failed_steps.len(),
            safe.exit_code()
        );
        let verification = format!(
            "verification={:?} status={:?} tables={} elapsed={:.3}s",
            safe.verification.mode,
            safe.verification.status,
            safe.verification.tables_checked,
            safe.elapsed_millis as f64 / 1000.0
        );
        wrapped(stderr, &summary, self.width)?;
        wrapped(stderr, &verification, self.width)?;
        for diagnostic in &safe.diagnostics {
            self.diagnostic(stderr, diagnostic)?;
        }
        if self.verbose {
            wrapped(
                stderr,
                &format!(
                    "run={} consistency={:?}",
                    visible(&safe.run_id),
                    safe.consistency
                ),
                self.width,
            )?;
            for table in &safe.tables {
                let rate = if table.copy_elapsed_millis == 0 {
                    0
                } else {
                    u128::from(table.committed_bytes) * 1000 / u128::from(table.copy_elapsed_millis)
                };
                wrapped(
                    stderr,
                    &format!(
                        "table {} -> {}.{} status={:?} read={} committed={} rejected={} unresolved={} indeterminate={} COPY-text-bytes={} COPY-text-bytes/s={rate}",
                        visible(&table.source_name),
                        visible(&table.target_schema),
                        visible(&table.target_name),
                        table.status,
                        table.rows_read,
                        table.committed_rows,
                        table.rejected_rows,
                        table.unresolved_rows,
                        table.indeterminate_rows,
                        table.committed_bytes
                    ),
                    self.width,
                )?;
            }
            for step in &safe.failed_steps {
                wrapped(stderr, &format!("failed step: {step}"), self.width)?;
            }
        }
        // Artifact paths must stay complete enough to use, even in a narrow terminal.
        writeln!(
            stderr,
            "report: {}/report.json",
            visible(&safe.artifact_dir)
        )?;
        self.drawn_lines = 0;
        stderr.flush()
    }

    /// Writer injection exercises ordinary/closed output devices without a mock pipeline.
    pub fn emit_to(
        &mut self,
        event: &RunEvent,
        stdout: &mut impl Write,
        stderr: &mut impl Write,
    ) -> io::Result<()> {
        // Defense in depth: database error classification must omit raw values and
        // DETAIL. This catches accidental URI/password text at every verbosity.
        let mut safe = event.clone();
        if let Some(diagnostic) = &mut safe.diagnostic {
            diagnostic.message = redact(&diagnostic.message);
            diagnostic.object = diagnostic.object.as_ref().map(|object| visible(object));
            diagnostic.code = visible(&diagnostic.code);
            diagnostic.stage = visible(&diagnostic.stage);
        }
        if self.output == OutputFormat::Json {
            serde_json::to_writer(&mut *stdout, &safe)?;
            stdout.write_all(b"\n")?;
            return stdout.flush();
        }
        if self.quiet && safe.diagnostic.is_none() && safe.kind != "outcome" {
            return Ok(());
        }
        let table = safe
            .table
            .as_deref()
            .map(visible)
            .unwrap_or_else(|| "run".into());
        let estimate = match safe.progress.as_ref().map(|p| p.quality) {
            Some(EstimateQuality::Metadata) => " estimated",
            Some(EstimateQuality::Measured) => " measured",
            _ => "",
        };
        let rate = if safe.elapsed_millis == 0 {
            0
        } else {
            (u128::from(safe.committed_rows) * 1000 / u128::from(safe.elapsed_millis))
                .min(u128::from(u64::MAX)) as u64
        };
        let bytes_rate = if safe.elapsed_millis == 0 {
            0
        } else {
            u128::from(safe.committed_bytes) * 1000 / u128::from(safe.elapsed_millis)
        };
        let line = format!(
            "{table}: {} committed={} rejected={} rows/s={rate} COPY-text-bytes/s={bytes_rate} elapsed={:.1}s{}",
            visible(&safe.phase),
            safe.committed_rows,
            safe.rejected_rows,
            safe.elapsed_millis as f64 / 1000.0,
            estimate
        );
        if self.progress {
            self.tables.insert(table, line);
            let important = safe.kind != "progress" || safe.diagnostic.is_some();
            if !important
                && self
                    .last_draw
                    .is_some_and(|last| last.elapsed() < Duration::from_millis(250))
            {
                return Ok(());
            }
            self.clear(stderr)?;
            // A compact view never grows with a long table list. It does not infer
            // completion from metadata row estimates or COPY acknowledgements.
            for value in self.tables.values().take(8) {
                writeln!(stderr, "{}", truncate(value, self.width))?;
                self.drawn_lines += 1;
            }
            self.last_draw = Some(Instant::now());
            if safe.kind == "table_complete"
                && let Some(table) = &safe.table
            {
                self.tables.remove(&visible(table));
            }
        } else if safe.kind != "progress" {
            writeln!(stderr, "{}", truncate(&line, self.width))?;
        }
        if let Some(diagnostic) = &safe.diagnostic {
            self.diagnostic(stderr, diagnostic)?;
            // Diagnostics are durable lines outside the redraw area.
            self.drawn_lines = 0;
        }
        stderr.flush()
    }

    fn clear(&mut self, stderr: &mut impl Write) -> io::Result<()> {
        for _ in 0..self.drawn_lines {
            stderr.write_all(b"\x1b[1A\r\x1b[2K")?;
        }
        self.drawn_lines = 0;
        Ok(())
    }

    fn diagnostic(
        &self,
        stderr: &mut impl Write,
        diagnostic: &crate::model::Diagnostic,
    ) -> io::Result<()> {
        let object = if self.verbose {
            diagnostic
                .object
                .as_ref()
                .map(|object| format!(" object={}", visible(object)))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let message = format!(
            "{} [{}]{object}: {}",
            visible(&diagnostic.code),
            visible(&diagnostic.stage),
            redact(&diagnostic.message)
        );
        if self.color {
            stderr.write_all(b"\x1b[31m")?;
        }
        wrapped(stderr, &message, self.width)?;
        if self.color {
            stderr.write_all(b"\x1b[0m")?;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> io::Result<()> {
        io::stdout().lock().flush()?;
        io::stderr().lock().flush()
    }
}

/// Render untrusted identifier control bytes visibly, including terminal escapes.
pub fn visible(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if character.is_control() {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}

fn terminal_width() -> usize {
    #[cfg(unix)]
    if let Ok(size) = rustix::termios::tcgetwinsize(io::stderr())
        && size.ws_col > 0
    {
        return usize::from(size.ws_col).min(240);
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(80)
        .min(240)
}

fn truncate(value: &str, width: usize) -> String {
    // Escaping non-ASCII gives deterministic ASCII width without guessing Unicode
    // terminal cell widths. Source identity remains exact in JSON/artifacts.
    let ascii: String = value
        .chars()
        .flat_map(|c| {
            if c.is_ascii() {
                vec![c]
            } else {
                c.escape_unicode().collect()
            }
        })
        .collect();
    if ascii.len() <= width {
        ascii
    } else if width <= 3 {
        ascii[..width].to_owned()
    } else {
        format!("{}...", &ascii[..width.saturating_sub(3)])
    }
}

fn wrapped(output: &mut impl Write, value: &str, width: usize) -> io::Result<()> {
    let ascii = truncate(value, usize::MAX);
    for chunk in ascii.as_bytes().chunks(width.max(1)) {
        output.write_all(chunk)?;
        output.write_all(b"\n")?;
    }
    Ok(())
}

pub fn redact(value: &str) -> String {
    static URI: OnceLock<Regex> = OnceLock::new();
    static SECRET: OnceLock<Regex> = OnceLock::new();
    let uri =
        URI.get_or_init(|| Regex::new(r"(?i)\b[a-z][a-z0-9+.-]*://[^\s]+").expect("URI grammar"));
    let secret = SECRET.get_or_init(|| {
        Regex::new(
            r#"(?i)\b(password|passwd|pwd|token|secret)\s*[:=]\s*("[^"]*"|'[^']*'|[^\s,;]+)"#,
        )
        .expect("secret grammar")
    });
    visible(&secret.replace_all(&uri.replace_all(value, "[redacted-uri]"), "$1=[redacted]"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::Command,
        config,
        model::{Diagnostic, Severity},
    };
    use clap::Parser;
    use std::fs::File;

    fn console(output: &str) -> Console {
        let args = Args::try_parse_from([
            "my2pg", "run", "m.toml", "--output", output, "--color", "never",
        ])
        .unwrap();
        let Command::Run(operation) = &args.command else {
            unreachable!()
        };
        let mut config = config::parse("version=1\n[source]\nurl_env='SRC'\nconsistency='frozen'\n[target]\nurl_env='DST'\nschema='legacy'\n", std::path::Path::new("/tmp")).unwrap();
        args.apply(&mut config, operation).unwrap();
        let mut console = Console::new(&config, &args);
        console.progress = false;
        console
    }
    fn event() -> RunEvent {
        RunEvent {
            version: 1,
            kind: "phase".into(),
            run_id: "run-test".into(),
            table: Some("evil\u{1b}[2J\nname".into()),
            range: None,
            progress: None,
            phase: "copy".into(),
            elapsed_millis: 100,
            committed_rows: 3,
            committed_bytes: 42,
            rejected_rows: 0,
            diagnostic: Some(Diagnostic {
                code: "COPY_FAILED".into(),
                stage: "copy".into(),
                object: None,
                severity: Severity::Error,
                message: "failed mysql://user:secret@host/db password=another_secret".into(),
            }),
        }
    }
    #[test]
    fn json_is_one_event_and_human_stream_is_separate() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        console("json")
            .emit_to(&event(), &mut stdout, &mut stderr)
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["committed_rows"], 3);
        assert!(stderr.is_empty());
        let printed = String::from_utf8(stdout).unwrap();
        assert!(!printed.contains("another_secret") && !printed.contains("user:secret"));
        assert_eq!(printed.lines().count(), 1);
    }
    #[test]
    fn phase_interval_is_a_scoped_json_event_only() {
        let mut json = console("json");
        let mut stdout = Vec::new();
        json.phase_interval_to("run-test", "copy", 100, 250, &["row_copy"], &mut stdout)
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(value["kind"], "phase_interval");
        assert_eq!(value["phase"], "copy");
        assert_eq!(value["run_id"], "run-test");
        assert_eq!(value["clock"], "run_monotonic_millis");
        assert_eq!(value["scope"], "run");
        assert_eq!(value["start_elapsed_millis"], 100);
        assert_eq!(value["end_elapsed_millis"], 250);
        assert_eq!(value["operation_classes"], serde_json::json!(["row_copy"]));

        let mut text = console("text");
        let mut ignored = Vec::new();
        text.phase_interval_to("run-test", "copy", 100, 250, &["row_copy"], &mut ignored)
            .unwrap();
        assert!(ignored.is_empty());
    }
    #[test]
    fn plain_output_escapes_terminal_input_and_is_ascii_narrow() {
        let mut renderer = console("text");
        renderer.width = 30;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        renderer
            .emit_to(&event(), &mut stdout, &mut stderr)
            .unwrap();
        assert!(stdout.is_empty());
        assert!(!stderr.contains(&27));
        let output = String::from_utf8(stderr).unwrap();
        assert!(output.is_ascii());
        assert!(output.lines().all(|line| line.len() <= 30));
        assert!(output.contains("\\u{1b}"));
    }
    #[cfg(unix)]
    #[test]
    fn actual_full_device_errors_propagate() {
        if let Ok(mut device) = File::options().write(true).open("/dev/full") {
            assert!(
                console("json")
                    .emit_to(&event(), &mut device, &mut Vec::new())
                    .is_err()
            );
        } else {
            // macOS lacks /dev/full: a closed real socket still proves writer errors.
            use std::os::unix::net::UnixStream;
            let (mut writer, reader) = UnixStream::pair().unwrap();
            drop(reader);
            assert!(
                console("json")
                    .emit_to(&event(), &mut writer, &mut Vec::new())
                    .is_err()
            );
        }
    }

    #[test]
    fn final_report_is_versioned_and_quiet_still_reports_outcome() {
        let report: RunReport =
            serde_json::from_str(include_str!("../../tests/contracts/report.json")).unwrap();
        let mut renderer = console("json");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        renderer
            .outcome_to(&report, &mut stdout, &mut stderr)
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(value["kind"], "outcome");
        assert_eq!(value["exit_code"], report.exit_code());
        assert_eq!(
            value["report"]["verification"]["tables_checked"],
            report.verification.tables_checked
        );
        assert!(stderr.is_empty());
        let mut renderer = console("text");
        renderer.quiet = true;
        stdout.clear();
        renderer
            .outcome_to(&report, &mut stdout, &mut stderr)
            .unwrap();
        assert!(stdout.is_empty());
        let text = String::from_utf8(stderr).unwrap();
        assert!(text.contains("report.json"));
        assert!(text.contains("verification="));
    }

    #[test]
    fn copy_text_throughput_has_explicit_units_and_zero_time_is_defined() {
        let mut renderer = console("text");
        renderer.width = 240;
        let mut update = event();
        update.table = Some("orders".into());
        update.diagnostic = None;
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        renderer.emit_to(&update, &mut stdout, &mut stderr).unwrap();
        let text = String::from_utf8(stderr).unwrap();
        assert!(text.contains("rows/s=30"));
        assert!(text.contains("COPY-text-bytes/s=420"));
        update.elapsed_millis = 0;
        let mut stderr = Vec::new();
        renderer.emit_to(&update, &mut stdout, &mut stderr).unwrap();
        let text = String::from_utf8(stderr).unwrap();
        assert!(text.contains("rows/s=0"));
        assert!(text.contains("COPY-text-bytes/s=0"));
    }

    #[test]
    fn tty_updates_are_throttled_and_estimates_never_claim_percent_complete() {
        let mut renderer = console("text");
        renderer.progress = true;
        let mut update = event();
        update.diagnostic = None;
        update.kind = "progress".into();
        update.progress = Some(crate::model::ProgressEstimate {
            estimated_rows: Some(3),
            quality: EstimateQuality::Metadata,
        });
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        renderer.emit_to(&update, &mut stdout, &mut stderr).unwrap();
        let drawn = stderr.len();
        update.committed_rows = 4;
        renderer.emit_to(&update, &mut stdout, &mut stderr).unwrap();
        assert_eq!(stderr.len(), drawn);
        update.kind = "table_complete".into();
        renderer.emit_to(&update, &mut stdout, &mut stderr).unwrap();
        assert!(renderer.tables.is_empty());
        assert!(!String::from_utf8(stderr).unwrap().contains('%'));
    }
}
