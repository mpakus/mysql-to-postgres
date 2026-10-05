use clap::Parser;
use my2pg::{
    cli::{Args, Command, ConfigCommand, Import, ImportConsistency, Operation},
    config::{self, MigrationConfig, OutputFormat},
    model::{MigrationPlan, RunReport, VerificationStatus},
    pipeline,
    report::{self, Console},
};
use serde::Serialize;
use std::{
    ffi::OsStr,
    future::Future,
    io::{self, Read, Write},
    process::ExitCode,
};
use tokio::sync::watch;

type CommandResult<T> = Result<T, (u8, String)>;

fn configuration(args: &Args, operation: &Operation) -> CommandResult<MigrationConfig> {
    let mut config = config::load(&operation.config).map_err(|error| (2, error.to_string()))?;
    args.apply(&mut config, operation)
        .map_err(|error| (2, error.to_string()))?;
    Ok(config)
}

fn redact_document(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = report::redact(text),
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact_document),
        serde_json::Value::Object(values) => values.values_mut().for_each(redact_document),
        _ => (),
    }
}

fn document(value: &impl Serialize, format: OutputFormat) -> CommandResult<()> {
    let mut value = serde_json::to_value(value)
        .map_err(|_| (1, "OUTPUT_SERIALIZE: cannot serialize result".into()))?;
    redact_document(&mut value);
    let mut output = io::stdout().lock();
    let result = match format {
        OutputFormat::Json => serde_json::to_writer(&mut output, &value),
        OutputFormat::Text => serde_json::to_writer_pretty(&mut output, &value),
    };
    result.map_err(|_| (1, "OUTPUT_IO: cannot write result".into()))?;
    writeln!(output)
        .and_then(|()| output.flush())
        .map_err(|_| (1, "OUTPUT_IO: cannot flush result".into()))
}

async fn read_only<T>(
    operation: impl Future<Output = Result<T, pipeline::PipelineError>>,
    mut cancel: watch::Receiver<bool>,
) -> CommandResult<T> {
    if *cancel.borrow() {
        return Err((130, "CANCELLED: read-only operation interrupted".into()));
    }
    tokio::select! {
        result = operation => result.map_err(|error| (error.exit_code(), error.to_string())),
        _ = cancel.changed() => Err((130, "CANCELLED: read-only operation interrupted".into())),
    }
}

fn import_load(args: &Args, import: &Import) -> CommandResult<u8> {
    use config::import::{DefaultPolicy, ImportOptions};
    let file = std::fs::File::open(&import.input)
        .map_err(|_| (2, "IMPORT_INPUT: cannot read load configuration".into()))?;
    let mut text = String::new();
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|_| {
            (
                2,
                "IMPORT_INPUT: load configuration must be readable UTF-8".into(),
            )
        })?;
    let options = ImportOptions {
        target_schema: import.target_schema.clone(),
        consistency: match import.consistency {
            ImportConsistency::Frozen => config::Consistency::Frozen,
            ImportConsistency::SingleSnapshot => config::Consistency::SingleSnapshot,
        },
        source_env: import.source_env.clone(),
        target_env: import.target_env.clone(),
        append_data_only: import.append_data_only,
        default_policy: if import.use_my2pg_defaults {
            DefaultPolicy::My2pgReviewed
        } else {
            DefaultPolicy::RequireExplicitChoice
        },
    };
    let imported =
        config::import::parse(&text, &options).map_err(|error| (2, error.to_string()))?;
    config::import::publish_new(&imported, &import.output)
        .map_err(|error| (1, error.to_string()))?;
    if !args.quiet {
        document(&imported.compatibility, OutputFormat::Json)?;
    }
    Ok(0)
}

fn saved_run(directory: &std::path::Path) -> CommandResult<(MigrationPlan, RunReport)> {
    let read = |name: &str| {
        std::fs::read(directory.join(name))
            .map_err(|_| (2, "VERIFY_ARTIFACT: cannot read saved run evidence".into()))
    };
    let plan: MigrationPlan = serde_json::from_slice(&read("plan.json")?)
        .map_err(|_| (2, "VERIFY_ARTIFACT: saved plan is invalid".into()))?;
    let report: RunReport = serde_json::from_slice(&read("report.json")?)
        .map_err(|_| (2, "VERIFY_ARTIFACT: saved report is invalid".into()))?;
    let requested = std::fs::canonicalize(directory)
        .map_err(|_| (2, "VERIFY_ARTIFACT: run directory is unavailable".into()))?;
    let recorded = std::fs::canonicalize(&report.artifact_dir).map_err(|_| {
        (
            2,
            "VERIFY_ARTIFACT: report run directory is unavailable".into(),
        )
    })?;
    if requested != recorded {
        return Err((
            2,
            "VERIFY_ARTIFACT: plan and report do not belong to the selected run directory".into(),
        ));
    }
    Ok((plan, report))
}

async fn execute(args: &Args, cancel: watch::Receiver<bool>) -> CommandResult<u8> {
    match &args.command {
        Command::Config(config) => match &config.command {
            ConfigCommand::Import(import) => import_load(args, import),
        },
        Command::ImportLoad(import) => import_load(args, import),
        Command::Check(operation) => {
            let config = configuration(args, operation)?;
            config::resolve_credentials(&config).map_err(|e| (2, e.to_string()))?;
            let mut stdout = io::stdout().lock();
            if config.report.console == OutputFormat::Json {
                let document = serde_json::json!({"version": 1, "operation": "check", "status": "valid", "config": config});
                serde_json::to_writer(&mut stdout, &document)
                    .map_err(|_| (1, "OUTPUT_IO: cannot write check result".into()))?;
                writeln!(stdout).map_err(|_| (1, "OUTPUT_IO: cannot write check result".into()))?;
            } else if !args.quiet {
                writeln!(stdout, "Configuration valid: Oracle MySQL -> PostgreSQL, schema {}. No connection opened.", my2pg::report::visible(&config.target.schema))
                    .map_err(|_| (1, "OUTPUT_IO: cannot write check result".into()))?;
            }
            stdout
                .flush()
                .map_err(|_| (1, "OUTPUT_IO: cannot flush check result".into()))?;
            Ok(0)
        }
        Command::Inspect(operation) => {
            let config = configuration(args, operation)?;
            let credentials =
                config::resolve_credentials(&config).map_err(|error| (2, error.to_string()))?;
            let catalogs = read_only(pipeline::inspect(&config, &credentials), cancel).await?;
            document(
                &serde_json::json!({"version":1,"operation":"inspect","config":config,"catalogs":catalogs}),
                config.report.console,
            )?;
            Ok(0)
        }
        Command::Plan(operation) => {
            let config = configuration(args, operation)?;
            let credentials =
                config::resolve_credentials(&config).map_err(|error| (2, error.to_string()))?;
            let plan = read_only(pipeline::plan(&config, &credentials), cancel).await?;
            document(
                &serde_json::json!({"version":1,"operation":"plan","config":config,"plan":plan}),
                config.report.console,
            )?;
            Ok(0)
        }
        Command::Run(operation) => {
            let config = configuration(args, operation)?;
            let credentials =
                config::resolve_credentials(&config).map_err(|error| (2, error.to_string()))?;
            let mut console = Console::new(&config, args);
            let report = pipeline::run(&config, &credentials, &mut console, cancel)
                .await
                .map_err(|error| (error.exit_code(), error.to_string()))?;
            Ok(report.exit_code())
        }
        Command::Verify(verify) => {
            let mut config = configuration(args, &verify.operation)?;
            if let Some(mode) = verify.mode {
                config.verification.mode = mode.into();
            }
            config::validate(&config).map_err(|error| (2, error.to_string()))?;
            if config.verification.mode == config::VerificationMode::None {
                return Err((2, "VERIFY_MODE: verification must be enabled".into()));
            }
            let credentials =
                config::resolve_credentials(&config).map_err(|error| (2, error.to_string()))?;
            let (plan, report) = saved_run(&verify.run_dir)?;
            let result = read_only(
                pipeline::verify_saved_run(&config, &credentials, &plan, &report),
                cancel,
            )
            .await?;
            let code = match result.status {
                VerificationStatus::Complete => 0,
                VerificationStatus::Different | VerificationStatus::Unsupported => 4,
                VerificationStatus::Error => 1,
                VerificationStatus::NotRun => 2,
            };
            document(
                &serde_json::json!({"version":1,"operation":"verify","run_id":report.run_id,"verification":result}),
                config.report.console,
            )?;
            Ok(code)
        }
    }
}

async fn interrupt() -> io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}

async fn command_main() -> ExitCode {
    let args = Args::parse();
    let (sender, receiver) = watch::channel(false);
    let signals = tokio::spawn(async move {
        // Failure to observe signals requests shutdown; never leave a migration
        // running without operator control.
        let _ = interrupt().await;
        let _ = sender.send(true);
    });
    let result = execute(&args, receiver).await;
    signals.abort();
    let _ = signals.await;
    match result {
        Ok(code) => ExitCode::from(code),
        Err((code, message)) => {
            if writeln!(
                io::stderr().lock(),
                "{}",
                report::visible(&report::redact(&message))
            )
            .is_err()
            {
                return ExitCode::from(1);
            }
            ExitCode::from(code)
        }
    }
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    if arguments.next().as_deref() == Some(OsStr::new(my2pg::report::artifact_io::WORKER_ARGUMENT))
    {
        if arguments.next().is_some() {
            let _ = writeln!(
                io::stderr().lock(),
                "ARTIFACT_WORKER: private worker accepts no arguments"
            );
            return ExitCode::from(2);
        }
        return match my2pg::report::artifact_io::worker_stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => {
                let _ = writeln!(
                    io::stderr().lock(),
                    "ARTIFACT_WORKER: protocol or persistence failure"
                );
                ExitCode::from(1)
            }
        };
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = writeln!(io::stderr().lock(), "RUNTIME: cannot initialize runtime");
            return ExitCode::from(1);
        }
    };
    runtime.block_on(command_main())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::SystemTime};

    #[test]
    fn saved_run_requires_plan_and_report_from_the_selected_directory() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "my2pg-verify-artifact-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let canonical = fs::canonicalize(&directory).unwrap();
        fs::write(
            canonical.join("plan.json"),
            include_str!("../tests/contracts/plan.json"),
        )
        .unwrap();
        let mut report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/contracts/report.json")).unwrap();
        report["artifact_dir"] = canonical.to_string_lossy().into_owned().into();
        fs::write(
            canonical.join("report.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        let (plan, report) = saved_run(&canonical).unwrap();
        assert_eq!(plan.version, 1);
        assert_eq!(report.version, 1);

        report_path_mismatch(&canonical);
        fs::remove_dir_all(canonical).unwrap();
    }

    fn report_path_mismatch(directory: &std::path::Path) {
        let mut report: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
        report["artifact_dir"] = directory
            .join("other-run")
            .to_string_lossy()
            .into_owned()
            .into();
        fs::write(
            directory.join("report.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        assert!(saved_run(directory).is_err());
    }
}
