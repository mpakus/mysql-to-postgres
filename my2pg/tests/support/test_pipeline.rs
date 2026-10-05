use my2pg::{
    config::{MigrationConfig, ResolvedCredentials},
    model::RunReport,
    pipeline::PipelineError,
    report::Console,
};
use std::path::Path;
use tokio::sync::watch;

/// Runs a direct integration-test pipeline call with Cargo's actual `my2pg`
/// binary for its durable artifact worker.
pub async fn run(
    config: &MigrationConfig,
    credentials: &ResolvedCredentials,
    console: &mut Console,
    cancel: watch::Receiver<bool>,
) -> Result<RunReport, PipelineError> {
    my2pg::pipeline::run_with_artifact_executable(
        config,
        credentials,
        console,
        cancel,
        Path::new(env!("CARGO_BIN_EXE_my2pg")),
    )
    .await
}
