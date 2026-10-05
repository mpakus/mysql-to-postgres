//! Command parsing only; dispatch remains in main.
use crate::config::{
    self, ConfigError, MigrationConfig, OutputFormat, ProgressPolicy, VerificationMode,
};
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use std::{num::NonZeroUsize, path::PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "my2pg",
    version,
    about = "Migrate Oracle MySQL structures and data to PostgreSQL",
    arg_required_else_help = true
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
    #[arg(long, global = true, value_enum, default_value = "auto")]
    pub color: Color,
    #[arg(long, global = true, value_enum)]
    pub progress: Option<Progress>,
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
    #[arg(long, short, global = true, conflicts_with = "quiet")]
    pub verbose: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Validate local configuration and credential references without connecting.
    Check(Operation),
    /// Inspect both database catalogs without writing.
    Inspect(Operation),
    /// Inspect the resolved migration plan without writing.
    Plan(Operation),
    /// Preflight and execute a fresh migration.
    Run(Operation),
    /// Recheck a completed run against an unchanged source and target.
    Verify(Verify),
    /// Translate a supported pgloader MySQL configuration.
    Config(Config),
    /// Alias for config import.
    ImportLoad(Import),
}

#[derive(Debug, ClapArgs)]
pub struct Operation {
    pub config: PathBuf,
    #[arg(long, value_enum)]
    pub output: Option<Output>,
    #[arg(long)]
    pub table_workers: Option<NonZeroUsize>,
    #[arg(long)]
    pub readers_per_table: Option<NonZeroUsize>,
    #[arg(long)]
    pub index_workers: Option<NonZeroUsize>,
}

#[derive(Debug, ClapArgs)]
pub struct Verify {
    #[command(flatten)]
    pub operation: Operation,
    /// Directory containing plan.json and report.json from the completed run.
    #[arg(long, required = true)]
    pub run_dir: PathBuf,
    #[arg(long, value_enum)]
    pub mode: Option<Verification>,
}

#[derive(Debug, ClapArgs)]
pub struct Config {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Import(Import),
}

#[derive(Debug, ClapArgs)]
pub struct Import {
    pub input: PathBuf,
    #[arg(long, id = "import_output")]
    pub output: PathBuf,
    #[arg(long)]
    pub target_schema: Option<String>,
    #[arg(long, value_enum, default_value = "frozen")]
    pub consistency: ImportConsistency,
    #[arg(long, default_value = "MY2PG_SOURCE_URL")]
    pub source_env: String,
    #[arg(long, default_value = "MY2PG_TARGET_URL")]
    pub target_env: String,
    #[arg(long)]
    pub append_data_only: bool,
    /// Adopt the documented my2pg defaults after reviewing legacy differences.
    #[arg(long)]
    pub use_my2pg_defaults: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ImportConsistency {
    Frozen,
    #[value(name = "single_snapshot", alias = "single-snapshot")]
    SingleSnapshot,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Output {
    Text,
    Json,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Color {
    Auto,
    Always,
    Never,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Progress {
    Auto,
    Always,
    Never,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Verification {
    #[value(name = "counts_and_schema", alias = "counts-and-schema")]
    CountsAndSchema,
    Content,
    None,
}

impl From<Output> for OutputFormat {
    fn from(value: Output) -> Self {
        match value {
            Output::Text => Self::Text,
            Output::Json => Self::Json,
        }
    }
}
impl From<Progress> for ProgressPolicy {
    fn from(value: Progress) -> Self {
        match value {
            Progress::Auto => Self::Auto,
            Progress::Always => Self::Always,
            Progress::Never => Self::Never,
        }
    }
}
impl From<Verification> for VerificationMode {
    fn from(value: Verification) -> Self {
        match value {
            Verification::CountsAndSchema => Self::CountsAndSchema,
            Verification::Content => Self::Content,
            Verification::None => Self::None,
        }
    }
}

impl Args {
    /// Apply only operational overrides, then validate their combined policy.
    pub fn apply(
        &self,
        config: &mut MigrationConfig,
        operation: &Operation,
    ) -> Result<(), ConfigError> {
        if let Some(output) = operation.output {
            config.report.console = output.into();
        }
        if let Some(progress) = self.progress {
            config.report.progress = progress.into();
        }
        if let Some(workers) = operation.table_workers {
            config.migration.table_workers = workers.get();
        }
        if let Some(readers) = operation.readers_per_table {
            config.migration.readers_per_table = readers.get();
        }
        if let Some(workers) = operation.index_workers {
            config.migration.index_workers = workers.get();
        }
        config::validate(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_commands_and_operational_controls_parse() {
        for command in ["check", "inspect", "plan", "run"] {
            let args = Args::try_parse_from([
                "my2pg",
                command,
                "migration.toml",
                "--output",
                "json",
                "--progress",
                "never",
            ])
            .unwrap();
            let operation = match args.command {
                Command::Check(operation)
                | Command::Inspect(operation)
                | Command::Plan(operation)
                | Command::Run(operation) => operation,
                _ => unreachable!(),
            };
            assert!(matches!(operation.output, Some(Output::Json)));
        }
        let verify = Args::try_parse_from([
            "my2pg",
            "verify",
            "migration.toml",
            "--run-dir",
            "runs/run-1",
            "--mode",
            "content",
            "--output",
            "json",
        ])
        .unwrap();
        let Command::Verify(verify) = verify.command else {
            unreachable!();
        };
        assert_eq!(verify.run_dir, PathBuf::from("runs/run-1"));
        assert!(matches!(verify.operation.output, Some(Output::Json)));
        assert!(matches!(verify.mode, Some(Verification::Content)));
        assert!(Args::try_parse_from(["my2pg", "verify", "migration.toml"]).is_err());
        let args = Args::try_parse_from([
            "my2pg", "config", "import", "old.load", "--output", "new.toml",
        ])
        .unwrap();
        assert!(matches!(
            args.command,
            Command::Config(Config {
                command: ConfigCommand::Import(_)
            })
        ));
        assert!(Args::try_parse_from(["my2pg", "run", "m.toml", "--table-workers", "0"]).is_err());
        assert!(Args::try_parse_from(["my2pg", "run", "m.toml", "--quiet", "--verbose"]).is_err());
        assert!(Args::try_parse_from(["my2pg", "sqlite", "m.toml"]).is_err());
    }
}
