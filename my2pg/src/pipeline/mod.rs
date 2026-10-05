//! Bounded concrete table pipelines. No source rows replay after uncertainty.
pub mod hooks;
pub mod ranges;
pub mod recovery;
pub mod scheduler;

use crate::{
    config::{
        self, Consistency, ExistingPolicy, MigrationConfig, MigrationMode, ResolvedCredentials,
        RowErrorPolicy, VerificationMode,
    },
    convert,
    model::*,
    mysql, plan as planner, postgres,
    report::{
        Console,
        artifact_io::{
            ArtifactIdentity, ArtifactLimits, DurableArtifacts, RawRetention, WORKER_ARGUMENT,
        },
    },
    verify,
};
use bytes::Bytes;
use futures_util::{FutureExt, StreamExt, TryStreamExt, stream::FuturesUnordered};
use mysql_async::prelude::Queryable;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, mpsc, watch};

const CATALOG_OPERATION_CLASSES: &[&str] = &[
    "source_mysql_catalog",
    "target_postgresql_schema_table_prepare",
];
const COPY_OPERATION_CLASSES: &[&str] = &["row_copy"];
const INDEX_CONSTRAINT_OPERATION_CLASSES: &[&str] = &[
    "primary_keys",
    "indexes",
    "foreign_keys",
    "check_constraints",
];

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub source: SourceCatalog,
    pub target: TargetCatalog,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PipelineError {
    message: String,
    exit: u8,
}
impl PipelineError {
    pub fn exit_code(&self) -> u8 {
        self.exit
    }
    fn preflight(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit: 2,
        }
    }
    fn io() -> Self {
        Self {
            message: "migration artifact or console I/O failed".into(),
            exit: 1,
        }
    }
    fn cancelled() -> Self {
        Self {
            message: "migration cancelled before execution".into(),
            exit: 130,
        }
    }
    fn source_shutdown_unconfirmed(mut self) -> Self {
        self.message
            .push_str("; MySQL server-side termination could not be confirmed");
        self
    }
}

#[derive(Default)]
struct SnapshotCancellation {
    source_id: AtomicU64,
    source_connecting: AtomicBool,
    control: tokio::sync::Mutex<Option<mysql::SourceConnection>>,
}
impl SnapshotCancellation {
    fn begin_source_connect(&self) {
        self.source_connecting.store(true, Ordering::Release);
    }
    fn register_source(&self, source_id: u64) {
        self.source_id.store(source_id, Ordering::Release);
        self.source_connecting.store(false, Ordering::Release);
    }
    async fn install_control(&self, control: mysql::SourceConnection) {
        *self.control.lock().await = Some(control);
    }
    async fn cancel_preflight(&self, deadline: tokio::time::Instant) -> Result<(), ()> {
        if self.source_connecting.load(Ordering::Acquire) {
            return Err(());
        }
        let id = self.source_id.load(Ordering::Acquire);
        if id == 0 {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, async {
            let mut control = self.control.lock().await;
            let control = control.as_mut().ok_or(())?;
            cancel_mysql_connection(control, id, deadline).await
        })
        .await
        .map_err(|_| ())?
    }
    async fn close(&self) {
        self.source_id.store(0, Ordering::Release);
        self.control.lock().await.take();
    }
}

async fn cancel_mysql_connection(
    control: &mut mysql::SourceConnection,
    id: u64,
    deadline: tokio::time::Instant,
) -> Result<(), ()> {
    tokio::time::timeout_at(deadline, async {
        match control.query_drop(format!("KILL CONNECTION {id}")).await {
            Ok(()) => {}
            Err(mysql_async::Error::Server(error)) if error.code == 1094 => {}
            Err(_) => return Err(()),
        }
        loop {
            let active: u64 = control
                .exec_first(
                    "SELECT count(*) FROM information_schema.processlist WHERE ID=?",
                    (id,),
                )
                .await
                .map_err(|_| ())?
                .unwrap_or(0);
            if active == 0 {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| ())?
}

async fn preflight_shutdown_error(
    error: PipelineError,
    cancellation: &SnapshotCancellation,
) -> PipelineError {
    if cancellation
        .cancel_preflight(tokio::time::Instant::now() + Duration::from_secs(10))
        .await
        .is_err()
    {
        error.source_shutdown_unconfirmed()
    } else {
        error
    }
}

async fn connect(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
) -> Result<(mysql::SourceConnection, postgres::TargetConnection), PipelineError> {
    config::validate(config)
        .map_err(|_| PipelineError::preflight("invalid migration configuration"))?;
    let source = mysql::connect(
        &config.source,
        creds.source.expose(),
        config.migration.max_row_bytes,
    )
    .await
    .map_err(|_| PipelineError::preflight("MySQL connection or session validation failed"))?;
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .map_err(|_| {
            PipelineError::preflight("PostgreSQL connection or session validation failed")
        })?;
    Ok((source, target))
}
async fn catalogs(
    config: &MigrationConfig,
    source: &mut mysql::SourceConnection,
    target: &mut postgres::TargetConnection,
) -> Result<Inspection, PipelineError> {
    let source = mysql::inspect(source)
        .await
        .map_err(|_| PipelineError::preflight("MySQL catalog inspection failed"))?;
    let schemas: Vec<_> = config
        .tables
        .rename
        .iter()
        .filter_map(|rename| rename.schema.clone())
        .collect();
    let target = postgres::inspect_schemas(&mut target.client, &config.target.schema, &schemas)
        .await
        .map_err(|_| PipelineError::preflight("PostgreSQL catalog inspection failed"))?;
    Ok(Inspection { source, target })
}
fn build(
    config: &MigrationConfig,
    inspection: &Inspection,
) -> Result<MigrationPlan, PipelineError> {
    planner::build(config, &inspection.source, &inspection.target).map_err(|error| match error {
        planner::PlanError::Config(_) => {
            PipelineError::preflight("migration planning configuration rejected")
        }
        planner::PlanError::Blocked(diagnostics) => PipelineError::preflight(format!(
            "migration planning rejected: {}",
            diagnostics
                .iter()
                .map(|d| d.code.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    })
}
pub async fn inspect(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
) -> Result<Inspection, PipelineError> {
    let (mut source, mut target) = connect(config, creds).await?;
    catalogs(config, &mut source, &mut target).await
}
pub async fn plan(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
) -> Result<MigrationPlan, PipelineError> {
    build(config, &inspect(config, creds).await?)
}

/// Recheck a completed run using its saved plan and accounting. This is a new
/// read-only scan; the operator must keep the source dataset unchanged.
pub async fn verify_saved_run(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    plan: &MigrationPlan,
    report: &RunReport,
) -> Result<VerificationReport, PipelineError> {
    config::validate(config)
        .map_err(|_| PipelineError::preflight("invalid verification configuration"))?;
    let plan_sha256 = plan_sha256(plan)?;
    let source_endpoint_sha256 = endpoint_sha256(creds.source.expose())?;
    let target_endpoint_sha256 = endpoint_sha256(creds.target.expose())?;
    if plan.version != 1
        || report.version != 1
        || report.plan_sha256 != plan_sha256
        || report.source_endpoint_sha256 != source_endpoint_sha256
        || report.target_endpoint_sha256 != target_endpoint_sha256
        || plan.consistency != config.source.consistency
        || plan.target_schema != config.target.schema
        || report.status != RunStatus::Complete
        || report.consistency != plan.consistency
        || report.mode != plan.mode
        || report.exit_code() != 0
        || report.tables.len() != plan.tables.len()
        || plan.tables.iter().any(|table| {
            let mut matches = report.tables.iter().filter(|row| row.id == table.id);
            let Some(row) = matches.next() else {
                return true;
            };
            matches.next().is_some()
                || row.source_name != table.source_name
                || row.target_schema != table.target_schema
                || row.target_name != table.target_name
                || row.status != RunStatus::Complete
                || row.accounted_rows() != Some(row.rows_read)
        })
    {
        return Err(PipelineError::preflight(
            "saved run plan and report are incomplete or do not match this configuration",
        ));
    }
    let mut effective_plan = plan.clone();
    effective_plan.verification = config.verification.mode;
    if effective_plan.verification == VerificationMode::None {
        return Err(PipelineError::preflight(
            "verification must be enabled for this command",
        ));
    }
    let (mut source, mut target) = connect(config, creds).await?;
    let source_catalog = mysql::inspect(&mut source)
        .await
        .map_err(|_| PipelineError::preflight("MySQL verification source inspection failed"))?;
    if source_catalog.database != plan.source_database
        || source_catalog.server_version != plan.source_version
    {
        return Err(PipelineError::preflight(
            "connected MySQL source identity differs from the saved plan",
        ));
    }
    let target_version: String = target
        .client
        .query_one("SELECT current_setting('server_version')", &[])
        .await
        .map_err(|_| PipelineError::preflight("PostgreSQL verification target inspection failed"))?
        .try_get(0)
        .map_err(|_| {
            PipelineError::preflight("PostgreSQL verification target metadata is invalid")
        })?;
    if target_version != plan.target_version {
        return Err(PipelineError::preflight(
            "connected PostgreSQL server version differs from the saved plan",
        ));
    }

    match effective_plan.verification {
        VerificationMode::CountsAndSchema => {
            mysql::start_snapshot(&mut source)
                .await
                .map_err(|_| PipelineError::preflight("MySQL verification snapshot failed"))?;
            verify::counts_and_schema(
                &mut source,
                &target,
                &effective_plan,
                &report.tables,
                &BTreeMap::new(),
            )
            .await
            .map_err(|_| PipelineError::preflight("database verification query failed"))
        }
        VerificationMode::Content => Ok(verify::content::verify_content(
            &mut source,
            &mut target,
            &effective_plan,
            &report.tables,
            verify::content::ContentVerificationOptions {
                memory_bytes: config.migration.memory_bytes,
                max_row_bytes: config.migration.max_row_bytes,
                source_snapshot_already_pinned: false,
                append_baseline: None,
            },
        )
        .await),
        VerificationMode::None => unreachable!("checked above"),
    }
}

fn plan_sha256(plan: &MigrationPlan) -> Result<String, PipelineError> {
    let bytes = serde_json::to_vec(plan).map_err(|_| PipelineError::io())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn endpoint_sha256(endpoint: &str) -> Result<String, PipelineError> {
    let mut endpoint = url::Url::parse(endpoint)
        .map_err(|_| PipelineError::preflight("database endpoint identity is invalid"))?;
    endpoint
        .set_password(None)
        .map_err(|_| PipelineError::preflight("database endpoint identity is invalid"))?;
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    Ok(format!(
        "{:x}",
        Sha256::digest(endpoint.as_str().as_bytes())
    ))
}

#[cfg(test)]
mod saved_run_identity_tests {
    use super::*;

    #[test]
    fn endpoint_identity_omits_password_and_tracks_host_and_database() {
        let first = endpoint_sha256("mysql://operator:first@db.example:3306/source").unwrap();
        let rotated = endpoint_sha256("mysql://operator:second@db.example:3306/source").unwrap();
        let other_host =
            endpoint_sha256("mysql://operator:second@other.example:3306/source").unwrap();
        let other_database =
            endpoint_sha256("mysql://operator:second@db.example:3306/other").unwrap();
        assert_eq!(first, rotated);
        assert_ne!(first, other_host);
        assert_ne!(first, other_database);
    }

    #[test]
    fn plan_identity_changes_when_a_verification_contract_changes() {
        let mut plan: MigrationPlan =
            serde_json::from_str(include_str!("../../tests/contracts/plan.json")).unwrap();
        let original = plan_sha256(&plan).unwrap();
        plan.target_schema.push_str("_changed");
        assert_ne!(original, plan_sha256(&plan).unwrap());
    }
}

fn execution_options(config: &MigrationConfig) -> Result<(usize, usize), PipelineError> {
    let options = &config.migration;
    if config.verification.mode == VerificationMode::Content {
        if options.mode == MigrationMode::SchemaOnly {
            return Err(PipelineError::preflight(
                "content verification requires a data migration",
            ));
        }
        if options.memory_bytes < 512 * 1024 {
            return Err(PipelineError::preflight(
                "content verification requires at least 512 KiB of memory",
            ));
        }
    }
    let layout = scheduler::MemoryLayout::compute(options).map_err(|_| {
        PipelineError::preflight(
            "memory_bytes resource arithmetic overflow or reservation exceeds u32",
        )
    })?;
    if (options.mode != MigrationMode::SchemaOnly
        && options.memory_bytes < layout.pipeline_reservation)
        || options.memory_bytes > Semaphore::MAX_PERMITS
    {
        return Err(PipelineError::preflight(
            "memory_bytes must cover queued, active and filling batches plus bounded row workspaces, and fit semaphore capacity",
        ));
    }
    Ok((layout.batch_reservation, layout.workspace_reservation))
}

#[cfg(all(test, target_pointer_width = "64"))]
mod resource_validation_tests {
    use super::*;

    #[tokio::test]
    async fn preflight_surfaces_unconfirmed_source_shutdown_without_changing_exit_code() {
        let cancellation = SnapshotCancellation::default();
        cancellation.begin_source_connect();
        let error = preflight_shutdown_error(PipelineError::cancelled(), &cancellation).await;
        assert_eq!(error.exit_code(), 130);
        assert!(
            error
                .to_string()
                .contains("MySQL server-side termination could not be confirmed")
        );
    }

    #[tokio::test]
    async fn preflight_surfaces_missing_control_for_registered_source() {
        let cancellation = SnapshotCancellation::default();
        cancellation.register_source(42);
        let error =
            preflight_shutdown_error(PipelineError::preflight("catalog failed"), &cancellation)
                .await;
        assert_eq!(error.exit_code(), 2);
        assert!(
            error
                .to_string()
                .contains("MySQL server-side termination could not be confirmed")
        );
    }

    #[test]
    fn runtime_preflight_uses_the_offline_queue_minimum_before_connecting() {
        let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
            "version":1,
            "source":{"url_env":"UNUSED_SOURCE","consistency":"frozen"},
            "target":{"url_env":"UNUSED_TARGET","schema":"selected"},
            "migration":{"batch_rows":2,"batch_bytes":1024,"max_row_bytes":64,"memory_bytes":70471}
        }))
        .unwrap();
        assert_eq!(execution_options(&config).unwrap_err().exit_code(), 2);
        assert!(config::validate(&config).is_err());
        config.migration.memory_bytes = 70472;
        assert_eq!(execution_options(&config).unwrap(), (1120, 65992));
        assert!(config::validate(&config).is_ok());
        config.migration.queue_batches = 3;
        assert!(execution_options(&config).is_err());
        assert!(config::validate(&config).is_err());
        config.migration.mode = MigrationMode::SchemaOnly;
        config.migration.memory_bytes = 1;
        assert!(execution_options(&config).is_ok());
        assert!(config::validate(&config).is_ok());
    }
}

fn fingerprint(inspection: &Inspection, plan: &MigrationPlan) -> Result<Vec<u8>, PipelineError> {
    let mut source = inspection.source.clone();
    source.tables.retain(|table| {
        plan.tables
            .iter()
            .any(|planned| planned.source_name == table.name)
    });
    for table in &mut source.tables {
        table.estimated_rows = None;
        table.next_auto_increment = None;
    }
    let mut target = inspection.target.clone();
    for table in &mut target.tables {
        table.row_count = None;
    }
    serde_json::to_vec(&(source, target))
        .map(|bytes| Sha256::digest(bytes).to_vec())
        .map_err(|_| PipelineError::preflight("catalog fingerprint encoding failed"))
}
fn advisory_key(schema: &str) -> i64 {
    let digest = Sha256::digest(format!("my2pg:v1:{schema}").as_bytes());
    i64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("SHA-256 contains eight bytes"),
    )
}
async fn acquire_schema_locks(
    client: &tokio_postgres::Client,
    schemas: &BTreeSet<String>,
) -> Result<(), PipelineError> {
    let mut acquired = Vec::with_capacity(schemas.len());
    for schema in schemas {
        let key = advisory_key(schema);
        let locked = match client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&key])
            .await
        {
            Ok(row) => row.get::<_, bool>(0),
            Err(_) => {
                release_schema_locks(client, &acquired).await;
                return Err(PipelineError::preflight("migration advisory lock failed"));
            }
        };
        if !locked {
            release_schema_locks(client, &acquired).await;
            return Err(PipelineError::preflight(
                "another migration holds a target schema advisory lock",
            ));
        }
        acquired.push(key);
    }
    Ok(())
}

async fn release_schema_locks(client: &tokio_postgres::Client, acquired: &[i64]) {
    for key in acquired.iter().rev() {
        let _ = client
            .query_one("SELECT pg_advisory_unlock($1)", &[key])
            .await;
    }
}
async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

pub async fn run(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    console: &mut Console,
    cancel: watch::Receiver<bool>,
) -> Result<RunReport, PipelineError> {
    let executable = std::env::current_exe().map_err(|_| PipelineError::io())?;
    run_with_artifact_executable(config, creds, console, cancel, &executable).await
}

/// Trusted embedding seam: executes the supplied compatible my2pg binary's
/// private artifact protocol. The normal CLI always uses its own executable.
#[doc(hidden)]
pub async fn run_with_artifact_executable(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    console: &mut Console,
    mut cancel: watch::Receiver<bool>,
    executable: &std::path::Path,
) -> Result<RunReport, PipelineError> {
    let phase_clock = Instant::now();
    execution_options(config)?;
    let snapshot_cancellation = Arc::new(SnapshotCancellation::default());
    let preflight_cancellation = snapshot_cancellation.clone();
    let preflight = async {
        config::validate(config)
            .map_err(|_| PipelineError::preflight("invalid migration configuration"))?;
        let single_snapshot = config.source.consistency == Consistency::SingleSnapshot;
        if single_snapshot {
            preflight_cancellation.begin_source_connect();
        }
        let source_result = mysql::connect(
            &config.source,
            creds.source.expose(),
            config.migration.max_row_bytes,
        )
        .await;
        let mut source = match source_result {
            Ok(source) => source,
            Err(_) => {
                return Err(PipelineError::preflight(
                    "MySQL connection or session validation failed",
                ));
            }
        };
        if single_snapshot {
            let source_id = u64::from(source.id());
            preflight_cancellation.register_source(source_id);
            let control = mysql::connect(
                &config.source,
                creds.source.expose(),
                config.migration.max_row_bytes,
            )
            .await
            .map_err(|_| {
                PipelineError::preflight("MySQL cancellation control connection failed")
            })?;
            preflight_cancellation.install_control(control).await;
        }
        let mut target = postgres::connect(&config.target, creds.target.expose())
            .await
            .map_err(|_| {
                PipelineError::preflight("PostgreSQL connection or session validation failed")
            })?;
        let catalog_started_elapsed_millis = phase_clock.elapsed().as_millis() as u64;
        let initial = catalogs(config, &mut source, &mut target).await?;
        let initial_plan = build(config, &initial)?;
        let mut lock_schemas = BTreeSet::from([config.target.schema.clone()]);
        lock_schemas.extend(
            initial_plan
                .tables
                .iter()
                .map(|table| table.target_schema.clone()),
        );
        acquire_schema_locks(&target.client, &lock_schemas).await?;
        if config.source.consistency == Consistency::SingleSnapshot {
            mysql::start_snapshot(&mut source)
                .await
                .map_err(|_| PipelineError::preflight("MySQL read-only snapshot failed"))?;
        }
        let current = catalogs(config, &mut source, &mut target).await?;
        if fingerprint(&initial, &initial_plan)? != fingerprint(&current, &initial_plan)? {
            return Err(PipelineError::preflight(
                "database structures changed during preflight; replan the migration",
            ));
        }
        let plan = build(config, &current)?;
        if plan.verification == VerificationMode::Content && plan.tables.is_empty() {
            return Err(PipelineError::preflight(
                "content verification requires at least one selected table",
            ));
        }
        if config.migration.on_row_error == RowErrorPolicy::Reject
            && plan.mode != MigrationMode::SchemaOnly
            && config.target.on_existing != ExistingPolicy::Recreate
        {
            for table in &plan.tables {
                if current.target.tables.iter().any(|existing| {
                    existing.schema == table.target_schema && existing.name == table.target_name
                }) {
                    recovery::check_recovery_safety(&target, table)
                        .await
                        .map_err(|_| {
                            PipelineError::preflight("target behavior is unsafe for row recovery")
                        })?;
                }
            }
        }
        let mut baseline = BTreeMap::new();
        if config.target.on_existing == ExistingPolicy::Append {
            for table in &plan.tables {
                let sql = format!(
                    "SELECT count(*)::bigint FROM {}",
                    postgres::qualified(&table.target_schema, &table.target_name)
                );
                let count: i64 = target
                    .client
                    .query_one(&sql, &[])
                    .await
                    .map_err(|_| PipelineError::preflight("append baseline count failed"))?
                    .get(0);
                baseline.insert(
                    table.id.clone(),
                    u64::try_from(count).map_err(|_| {
                        PipelineError::preflight("append baseline count is invalid")
                    })?,
                );
            }
        }
        let content_baseline = if plan.verification == VerificationMode::Content
            && plan.on_existing == ExistingPolicy::Append
        {
            Some(
                verify::content::capture_append_baseline(
                    &mut target,
                    &plan,
                    config.migration.memory_bytes,
                    config.migration.max_row_bytes,
                )
                .await
                .map_err(|_| PipelineError::preflight("append content baseline capture failed"))?,
            )
        } else {
            None
        };
        Ok::<_, PipelineError>((
            source,
            target,
            plan,
            baseline,
            content_baseline,
            catalog_started_elapsed_millis,
        ))
    };
    let preflight = tokio::select! {
        biased;
        _ = cancelled(&mut cancel) => {
            return Err(preflight_shutdown_error(
                PipelineError::cancelled(),
                &snapshot_cancellation,
            ).await);
        },
        result = preflight => result,
    };
    let (mut source, mut target, plan, baseline, content_baseline, catalog_started_elapsed_millis) =
        match preflight {
            Ok(result) => result,
            Err(error) => {
                return Err(preflight_shutdown_error(error, &snapshot_cancellation).await);
            }
        };
    let snapshot_data_run = plan.consistency == Consistency::SingleSnapshot
        && plan.mode != MigrationMode::SchemaOnly
        && !plan.tables.is_empty();
    if !snapshot_data_run {
        snapshot_cancellation.close().await;
    }
    let identity = match ArtifactIdentity::new(&config.report.directory) {
        Ok(identity) => identity,
        Err(_) => {
            return Err(
                preflight_shutdown_error(PipelineError::io(), &snapshot_cancellation).await,
            );
        }
    };
    let started = Instant::now();
    let report_plan_sha256 = plan_sha256(&plan)?;
    let source_endpoint_sha256 = endpoint_sha256(creds.source.expose())?;
    let target_endpoint_sha256 = endpoint_sha256(creds.target.expose())?;
    let mut report = RunReport {
        version: 1,
        run_id: identity.run_id.clone(),
        plan_sha256: report_plan_sha256,
        source_endpoint_sha256,
        target_endpoint_sha256,
        status: RunStatus::Running,
        consistency: plan.consistency,
        mode: plan.mode,
        tables: plan
            .tables
            .iter()
            .map(|table| TableReport {
                id: table.id.clone(),
                source_name: table.source_name.clone(),
                target_schema: table.target_schema.clone(),
                target_name: table.target_name.clone(),
                status: RunStatus::Running,
                rows_read: 0,
                committed_rows: 0,
                committed_bytes: 0,
                copy_elapsed_millis: 0,
                rejected_rows: 0,
                unresolved_rows: 0,
                indeterminate_rows: 0,
                transformations: BTreeMap::new(),
            })
            .collect(),
        diagnostics: plan.diagnostics.clone(),
        exclusions: plan.exclusions.clone(),
        failed_steps: vec![],
        verification: VerificationReport {
            mode: plan.verification,
            status: VerificationStatus::NotRun,
            tables_checked: 0,
            differences: vec![],
        },
        elapsed_millis: 0,
        artifact_dir: identity.directory.to_string_lossy().into_owned(),
    };
    let index_jobs = finalization_groups(&plan).len();
    // Admission precedes artifact process creation and every target mutation.
    report.diagnostics.push(Diagnostic {
        code: "RESOURCE_ADMISSION".into(),
        stage: "scheduling".into(),
        object: None,
        severity: Severity::Warning,
        message: resource_message(
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            usize::MAX,
        ),
    });
    let limits =
        match ArtifactLimits::derive(&identity, &plan, &report, config.migration.max_row_bytes) {
            Ok(limits) => limits,
            Err(_) => {
                return Err(preflight_shutdown_error(
                    PipelineError::preflight("artifact reservation exceeds supported bounds"),
                    &snapshot_cancellation,
                )
                .await);
            }
        };
    let admission = match scheduler::Admission::new_with_artifact_reservation(
        &config.migration,
        config.source.consistency,
        plan.tables.len(),
        index_jobs,
        limits.reservation_bytes,
    ) {
        Ok(admission) => admission,
        Err(error) => {
            return Err(preflight_shutdown_error(
                PipelineError::preflight(error.to_string()),
                &snapshot_cancellation,
            )
            .await);
        }
    };
    let resources = scheduler::Resources::new(admission);
    report
        .diagnostics
        .last_mut()
        .expect("resource diagnostic")
        .message = resource_message(
        admission.table_workers,
        config.migration.table_workers,
        admission.index_workers,
        config.migration.index_workers,
        admission.queue_batches,
        admission.memory_bytes,
        admission.artifact_reservation_bytes,
        admission.connections.normal_source,
        admission.connections.normal_target,
        admission.connections.emergency_target,
        admission.connections.peak_target,
        config.migration.readers_per_table,
    );
    let mut command = std::process::Command::new(executable);
    command.arg(WORKER_ARGUMENT);
    let ids = plan
        .tables
        .iter()
        .map(|table| table.id.clone())
        .collect::<Vec<_>>();
    let artifact_reservation = match resources.take_artifact_reservation() {
        Ok(reservation) => reservation,
        Err(_) => {
            return Err(
                preflight_shutdown_error(PipelineError::io(), &snapshot_cancellation).await,
            );
        }
    };
    let artifacts = tokio::select! {
        biased;
        _ = cancelled(&mut cancel) => {
            return Err(preflight_shutdown_error(
                PipelineError::cancelled(),
                &snapshot_cancellation,
            ).await);
        },
        result = DurableArtifacts::start(command, identity, limits, &ids,
            artifact_reservation) => result,
    };
    let artifacts = match artifacts {
        Ok(artifacts) => artifacts,
        Err(_) => {
            return Err(
                preflight_shutdown_error(PipelineError::io(), &snapshot_cancellation).await,
            );
        }
    };
    let generation = AtomicU64::new(1);
    let initialize = async {
        artifacts.lease().await?.write_plan(&plan).await?;
        artifacts.lease().await?.write_report(1, &report).await
    };
    let initialized = tokio::select! {
        biased;
        _ = cancelled(&mut cancel) => None,
        result = initialize => Some(result),
    };
    if !matches!(initialized, Some(Ok(_))) {
        artifacts
            .shutdown(tokio::time::Instant::now() + Duration::from_secs(3))
            .await;
        let error = if initialized.is_none() {
            PipelineError::cancelled()
        } else {
            PipelineError::io()
        };
        return Err(preflight_shutdown_error(error, &snapshot_cancellation).await);
    }
    let registry = ExecutionRegistry::new(
        plan.tables.len(),
        config.migration.readers_per_table.saturating_sub(1),
        if snapshot_data_run {
            u64::from(source.id())
        } else {
            0
        },
    );
    registry.control.register(&target);
    let (stop, stopped) = watch::channel(false);
    let fault = {
        let execution = execute(
            config,
            creds,
            &mut source,
            &mut target,
            &plan,
            &baseline,
            content_baseline,
            &mut report,
            console,
            &artifacts,
            &generation,
            &resources,
            &registry,
            &stop,
            stopped,
            started,
            phase_clock,
            catalog_started_elapsed_millis,
        );
        tokio::pin!(execution);
        tokio::select! {
            biased;
            result = &mut execution => result.err(),
            _ = cancelled(&mut cancel) => {
                let _ = stop.send(true);
                resources.close();
                let cleanup = async {
                    // Mark cancellation on every registered transport before allowing
                    // a stopped worker to poll COPY again; requests proceed together.
                    let (_, outcome) = tokio::join!(registry.cancel_all(), &mut execution);
                    outcome
                };
                let result = tokio::time::timeout_at(registry.shutdown_deadline(), cleanup).await;
                match result {
                    Ok(Err(error)) if error.status != RunStatus::Cancelled => Some(error),
                    Err(_) => { let mut fault = Fault::cancelled(); fault.shutdown_deadline = true; Some(fault) },
                    _ => Some(Fault::cancelled()),
                }
            }
        }
    };
    let settled;
    if let Some(mut fault) = fault {
        fault = registry.retain_fault(fault);
        registry
            .cancel_single_snapshot(&snapshot_cancellation)
            .await;
        if registry.source_cancel_failed.load(Ordering::Acquire) != 0 {
            report.diagnostics.push(Diagnostic { code: "SOURCE_CANCEL".into(),stage: "shutdown".into(),object:None,severity:Severity::Warning,
                message:"owned source termination could not be acknowledged; client sockets close, but blocked server queries may remain until their locks are released".into() });
        }
        if fault.shutdown_deadline {
            report.diagnostics.push(Diagnostic { code: "SHUTDOWN_DEADLINE".into(), stage: "shutdown".into(), object: None, severity: Severity::Error,
                message: "worker teardown exceeded the shared 10-second deadline; retained ACKs and original failure remain authoritative".into() });
        }
        for index in 0..registry.tables.len() {
            if let Some((_, table_started)) = registry.table_copy_started(index) {
                report.tables[index].copy_elapsed_millis = elapsed(table_started);
            }
        }
        drop(source);
        let target_cancellation =
            tokio::time::timeout_at(registry.shutdown_deadline(), registry.cancel_all()).await;
        if !matches!(target_cancellation, Ok(false))
            || registry.target_cancel_failed.load(Ordering::Acquire) != 0
        {
            report.diagnostics.push(Diagnostic {
                code: "TARGET_CANCEL".into(),
                stage: "shutdown".into(),
                object: None,
                severity: Severity::Warning,
                message:
                    "target cancellation request failed; the target protocol connection was aborted"
                        .into(),
            });
        }
        if registry.any_indeterminate() {
            fault.status = RunStatus::Indeterminate;
            fault.code = "COMMIT_INDETERMINATE";
            fault.message = "COMMIT was attempted without acknowledgement; do not replay the batch automatically".into();
        }
        // Drop source before any filesystem or console operation: its guard
        // closes the socket without draining a pending result.
        drop(target);
        settled = settle_artifacts(&artifacts, &mut report).await;
        report.status = fault.status;
        let mut interrupted_step = None;
        if fault.status == RunStatus::Cancelled {
            for state in std::iter::once(&registry.control).chain(&registry.indexes) {
                if state.phase.load(Ordering::Acquire) == 1
                    && let Some(object) = state.step.lock().expect("DDL step registry").as_ref()
                    && let Some(index) = report.failed_steps.iter().rposition(|step| step == object)
                {
                    interrupted_step = Some(report.failed_steps.remove(index));
                }
            }
        }
        for (index, table) in report.tables.iter_mut().enumerate() {
            if table.status == RunStatus::Running {
                let state = &registry.tables[index];
                let current_copy = registry.table_indeterminate(index);
                table.status = if fault.status == RunStatus::Indeterminate
                    && !current_copy
                    && !registry.indexes[index].indeterminate()
                    && !registry.control.indeterminate()
                {
                    RunStatus::Cancelled
                } else {
                    fault.status
                };
                let remaining = table
                    .rows_read
                    .saturating_sub(table.committed_rows)
                    .saturating_sub(table.rejected_rows);
                if fault.status == RunStatus::Indeterminate && current_copy {
                    let active = std::iter::once(state)
                        .chain(&registry.range_lanes[index])
                        .filter(|reader| reader.indeterminate())
                        .map(|reader| reader.inflight.load(Ordering::Acquire))
                        .fold(0u64, u64::saturating_add);
                    table.indeterminate_rows = remaining.min(active);
                    table.unresolved_rows = remaining - table.indeterminate_rows;
                } else {
                    table.unresolved_rows = remaining;
                }
            }
        }
        report.diagnostics.push(Diagnostic {
            code: fault.code.into(),
            stage: "execution".into(),
            object: interrupted_step,
            severity: if fault.status == RunStatus::Cancelled {
                Severity::Warning
            } else {
                Severity::Error
            },
            message: fault.message,
        });
    } else {
        report.status = if report.tables.iter().any(|table| table.rejected_rows > 0) {
            RunStatus::Partial
        } else {
            RunStatus::Complete
        };
        drop(source);
        drop(target);
        settled = settle_artifacts(&artifacts, &mut report).await;
    }
    report.elapsed_millis = elapsed(started);
    if !settled {
        if report.status != RunStatus::Indeterminate {
            report.status = RunStatus::Failed;
        }
        report
            .diagnostics
            .push(artifact_diagnostic(artifacts.confirmed().report_generation));
    }
    let publication = persist_final(&artifacts, &generation, &report).await;
    if publication.is_err() {
        if report.status != RunStatus::Indeterminate {
            report.status = RunStatus::Failed;
        }
        report
            .diagnostics
            .push(artifact_diagnostic(artifacts.confirmed().report_generation));
    }
    // The successful report.json interface is published only after its durable
    // receipt. A failed newer generation leaves its confirmed predecessor intact.
    let rendered = console.outcome(&report).and_then(|()| console.finish());
    if rendered.is_err() {
        if report.status != RunStatus::Indeterminate {
            report.status = RunStatus::Failed;
        }
        report.diagnostics.push(Diagnostic {
            code: "CONSOLE_IO".into(),
            stage: "console".into(),
            object: None,
            severity: Severity::Error,
            message: "console output failed".into(),
        });
        let _ = persist_final(&artifacts, &generation, &report).await;
    }
    let confirmed = artifacts
        .shutdown(tokio::time::Instant::now() + Duration::from_secs(3))
        .await;
    if publication.is_err() || rendered.is_err() {
        return Err(PipelineError {
            message: format!(
                "migration artifact or console I/O failed; last confirmed report generation {}",
                confirmed.report_generation
            ),
            exit: 1,
        });
    }

    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn resource_message(
    t: usize,
    rt: usize,
    i: usize,
    ri: usize,
    q: usize,
    m: usize,
    a: usize,
    s: usize,
    n: usize,
    e: usize,
    p: usize,
    r: usize,
) -> String {
    format!(
        "effective table_workers={t}/{rt} index_workers={i}/{ri} queue_batches={q} application_bytes={m} artifact_bytes={a} normal_source_sockets={s} normal_target_sockets={n} emergency_target_sockets={e} peak_target_sockets={p} readers_per_table={r}; driver/TLS/allocator/catalog overhead is additional"
    )
}
fn artifact_diagnostic(generation: u64) -> Diagnostic {
    Diagnostic {
        code: "ARTIFACT_SHUTDOWN_UNKNOWN".into(),
        stage: "artifacts".into(),
        object: None,
        severity: Severity::Error,
        message: format!(
            "artifact persistence was not acknowledged; last confirmed report generation {generation}; retain its immutable report and inspect the unacknowledged suffix"
        ),
    }
}
async fn settle_artifacts(artifacts: &DurableArtifacts, report: &mut RunReport) -> bool {
    let settled = matches!(
        tokio::time::timeout(Duration::from_secs(3), artifacts.flush()).await,
        Ok(Ok(_))
    );
    if !settled {
        artifacts
            .shutdown(tokio::time::Instant::now() + Duration::from_secs(3))
            .await;
    }
    for (table, ack) in report.tables.iter_mut().zip(artifacts.confirmed().tables) {
        table.rejected_rows = ack.rejected_rows;
    }
    settled
}
async fn persist_final(
    artifacts: &DurableArtifacts,
    generation: &AtomicU64,
    report: &RunReport,
) -> Result<(), Fault> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let lease = artifacts.lease().await.map_err(|_| Fault::io())?;
        let next = generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| Fault::io())?
            + 1;
        lease
            .write_report(next, report)
            .await
            .map_err(|_| Fault::io())?;
        Ok(())
    })
    .await
    .map_err(|_| Fault::io())?
}

#[derive(Clone)]
struct Fault {
    status: RunStatus,
    code: &'static str,
    message: String,
    shutdown_deadline: bool,
}
#[derive(Default)]
struct ExecutionState {
    phase: AtomicU8, // 1 = DDL, 2 = DDL COMMIT, 3 = COPY.
    inflight: AtomicU64,
    copy_started: std::sync::Mutex<Option<(usize, Instant)>>,
    shutdown: std::sync::Mutex<Option<postgres::ShutdownHandle>>,
    step: std::sync::Mutex<Option<String>>,
    unknown: AtomicU8,
    source_id: AtomicU64,
    source_cancelled: tokio::sync::Notify,
}
struct RegisteredSource<'a>(&'a AtomicU64);
impl Drop for RegisteredSource<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}
impl ExecutionState {
    fn register(&self, target: &postgres::TargetConnection) {
        *self.shutdown.lock().expect("shutdown registry") = Some(target.shutdown_handle());
    }
    fn indeterminate(&self) -> bool {
        self.unknown.load(Ordering::Acquire) != 0
            || self.phase.load(Ordering::Acquire) == 2
            || (self.phase.load(Ordering::Acquire) == 3
                && self
                    .shutdown
                    .lock()
                    .expect("shutdown registry")
                    .as_ref()
                    .is_some_and(|handle| handle.stage.get() == postgres::CopyStage::CommitAttempt))
    }
}
struct ExecutionRegistry {
    control: ExecutionState,
    tables: Vec<ExecutionState>,
    range_lanes: Vec<Vec<ExecutionState>>,
    indexes: Vec<ExecutionState>,
    single_snapshot_source_id: u64,
    deadline: std::sync::Mutex<Option<tokio::time::Instant>>,
    cancellation_started: AtomicU8,
    source_cancel_failed: AtomicU8,
    target_cancel_failed: AtomicU8,
    first_fault: std::sync::Mutex<Option<Fault>>,
}
impl ExecutionRegistry {
    fn new(tables: usize, range_lanes_per_table: usize, single_snapshot_source_id: u64) -> Self {
        Self {
            control: ExecutionState::default(),
            tables: (0..tables).map(|_| ExecutionState::default()).collect(),
            range_lanes: (0..tables)
                .map(|_| {
                    (0..range_lanes_per_table)
                        .map(|_| ExecutionState::default())
                        .collect()
                })
                .collect(),
            indexes: (0..tables).map(|_| ExecutionState::default()).collect(),
            single_snapshot_source_id,
            deadline: std::sync::Mutex::new(None),
            cancellation_started: AtomicU8::new(0),
            source_cancel_failed: AtomicU8::new(0),
            target_cancel_failed: AtomicU8::new(0),
            first_fault: std::sync::Mutex::new(None),
        }
    }
    fn any_indeterminate(&self) -> bool {
        std::iter::once(&self.control)
            .chain(&self.tables)
            .chain(self.range_lanes.iter().flatten())
            .chain(&self.indexes)
            .any(ExecutionState::indeterminate)
    }
    fn table_indeterminate(&self, index: usize) -> bool {
        self.tables[index].indeterminate()
            || self.range_lanes[index]
                .iter()
                .any(ExecutionState::indeterminate)
    }
    fn table_copy_started(&self, index: usize) -> Option<(usize, Instant)> {
        self.range_lanes[index]
            .iter()
            .filter_map(|state| *state.copy_started.lock().expect("copy clock"))
            .next()
            .or_else(|| *self.tables[index].copy_started.lock().expect("copy clock"))
    }
    async fn cancel_all(&self) -> bool {
        self.shutdown_deadline();
        if self.cancellation_started.swap(1, Ordering::AcqRel) != 0 {
            return false;
        }
        let handles: Vec<_> = std::iter::once(&self.control)
            .chain(&self.tables)
            .chain(self.range_lanes.iter().flatten())
            .chain(&self.indexes)
            .filter_map(|state| state.shutdown.lock().expect("shutdown registry").clone())
            .collect();
        // Atomic stop barrier on every target precedes polling any worker again.
        // Abort owning protocol tasks now; CancelToken requests remain independent.
        for handle in &handles {
            handle.abort();
        }
        let mut requests: FuturesUnordered<_> =
            handles.iter().map(|handle| handle.cancel()).collect();
        let mut failed = false;
        while let Some(result) = requests.next().await {
            if result.is_err() {
                failed = true;
                // Retain an observed failure even if another request outlives
                // the outer deadline and this scoped future is dropped.
                self.target_cancel_failed.store(1, Ordering::Release);
            }
        }
        failed
    }
    fn publish_fault(&self, fault: &Fault) {
        let mut first = self
            .first_fault
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *first = Some(first.take().map_or_else(
            || fault.clone(),
            |previous| previous.retain_stronger(fault.clone()),
        ));
    }
    fn retain_fault(&self, fault: Fault) -> Fault {
        let first = self
            .first_fault
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match first {
            Some(first) => first.retain_stronger(fault),
            None => fault,
        }
    }
    fn shutdown_deadline(&self) -> tokio::time::Instant {
        *self
            .deadline
            .lock()
            .expect("shutdown deadline")
            .get_or_insert_with(|| tokio::time::Instant::now() + Duration::from_secs(10))
    }
    async fn cancel_sources(&self, control: &mut mysql::SourceConnection) {
        // Scoped workers are not polled while this method awaits. Their guards
        // still own these IDs; normal teardown clears registration before close.
        let readers: Vec<_> = self
            .tables
            .iter()
            .chain(self.range_lanes.iter().flatten())
            .filter_map(|state| {
                let id = state.source_id.load(Ordering::Acquire);
                (id != 0).then_some((state, id))
            })
            .collect();
        for (state, id) in readers {
            match tokio::time::timeout_at(
                self.shutdown_deadline(),
                control.query_drop(format!("KILL CONNECTION {id}")),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(mysql_async::Error::Server(error))) if error.code == 1094 => {} // Already gone.
                _ => {
                    self.source_cancel_failed.store(1, Ordering::Release);
                }
            }
            state.source_id.store(0, Ordering::Release);
            state.source_cancelled.notify_one();
        }
    }
    async fn cancel_single_snapshot(&self, control: &SnapshotCancellation) {
        let id = self.single_snapshot_source_id;
        if id == 0 {
            return;
        }
        let result = tokio::time::timeout_at(self.shutdown_deadline(), async {
            let mut connection = control.control.lock().await;
            let connection = connection.as_mut().ok_or(())?;
            cancel_mysql_connection(connection, id, self.shutdown_deadline()).await
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            self.source_cancel_failed.store(1, Ordering::Release);
        }
    }
}
impl Fault {
    fn failed(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: RunStatus::Failed,
            code,
            message: message.into(),
            shutdown_deadline: false,
        }
    }
    fn io() -> Self {
        Self::failed(
            "ARTIFACT_OR_CONSOLE_IO",
            "artifact or console output failed",
        )
    }
    fn cancelled() -> Self {
        Self {
            status: RunStatus::Cancelled,
            code: "CANCELLED",
            message: "migration interrupted".into(),
            shutdown_deadline: false,
        }
    }
    fn retain_stronger(self, other: Self) -> Self {
        fn priority(status: RunStatus) -> u8 {
            match status {
                RunStatus::Indeterminate => 3,
                RunStatus::Failed => 2,
                _ => 1,
            }
        }
        let deadline = self.shutdown_deadline || other.shutdown_deadline;
        let mut retained = if priority(other.status) > priority(self.status) {
            other
        } else {
            self
        };
        retained.shutdown_deadline = deadline;
        retained
    }
}
fn elapsed(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}
async fn drain_workers<F>(mut fault: Fault, workers: &mut FuturesUnordered<F>) -> Fault
where
    F: std::future::Future<Output = std::thread::Result<Result<(), Fault>>>,
{
    while let Some(result) = workers.next().await {
        let other = result.unwrap_or_else(|_| {
            Err(Fault::failed(
                "WORKER_PANIC",
                "worker panicked during teardown",
            ))
        });
        if let Err(other) = other {
            fault = fault.retain_stronger(other);
        }
    }
    fault
}

fn event(
    console: &mut Console,
    report: &RunReport,
    kind: &str,
    phase: &str,
    table: Option<&TablePlan>,
    range: Option<String>,
    started: Instant,
) -> Result<(), Fault> {
    let scope = table.map(|table| {
        report
            .tables
            .iter()
            .find(|reported| reported.id == table.id)
            .expect("planned table report")
    });
    let counters = scope
        .into_iter()
        .chain(report.tables.iter().filter(|_| table.is_none()))
        .try_fold((0u64, 0u64, 0u64), |(rows, bytes, rejects), table| {
            Some((
                rows.checked_add(table.committed_rows)?,
                bytes.checked_add(table.committed_bytes)?,
                rejects.checked_add(table.rejected_rows)?,
            ))
        })
        .ok_or_else(|| Fault::failed("ROW_OVERFLOW", "event counter overflow"))?;
    console
        .emit(&RunEvent {
            version: 1,
            kind: kind.into(),
            run_id: report.run_id.clone(),
            table: table.map(|t| t.source_name.clone()),
            range,
            progress: table.map(|t| ProgressEstimate {
                estimated_rows: t.estimated_rows,
                quality: if t.estimated_rows.is_some() {
                    EstimateQuality::Metadata
                } else {
                    EstimateQuality::Unknown
                },
            }),
            phase: phase.into(),
            elapsed_millis: scope
                .map_or_else(|| elapsed(started), |table| table.copy_elapsed_millis),
            committed_rows: counters.0,
            committed_bytes: counters.1,
            rejected_rows: counters.2,
            diagnostic: None,
        })
        .map_err(|_| Fault::io())
}

#[cfg(test)]
tokio::task_local! {
    static SCOPED_WORKER_FAULT: Box<dyn Fn(&str, Option<&TablePlan>, &RunReport) + Send + Sync>;
}
struct ReportOwner<'a> {
    report: std::sync::Mutex<&'a mut RunReport>,
    console: std::sync::Mutex<&'a mut Console>,
    artifacts: &'a DurableArtifacts,
    generation: &'a AtomicU64,
    started: Instant,
    phase_clock: Instant,
    copy_window: CopyPhaseWindow,
}
#[derive(Default)]
struct CopyPhaseWindow(std::sync::Mutex<Option<(u64, u64)>>);
impl CopyPhaseWindow {
    fn started(&self, elapsed_millis: u64) {
        let mut window = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &mut *window {
            Some((start, _)) => *start = (*start).min(elapsed_millis),
            slot @ None => *slot = Some((elapsed_millis, elapsed_millis)),
        }
    }
    fn finished(&self, elapsed_millis: u64) {
        if let Some((_, end)) = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            *end = (*end).max(elapsed_millis);
        }
    }
    fn interval(&self) -> Option<(u64, u64)> {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
#[cfg(test)]
mod phase_window_tests {
    use super::CopyPhaseWindow;

    #[test]
    fn concurrent_copy_workers_form_one_earliest_to_latest_wall_interval() {
        let window = CopyPhaseWindow::default();
        window.started(10);
        window.started(20);
        window.finished(50);
        window.started(30);
        window.finished(80);
        assert_eq!(window.interval(), Some((10, 80)));
    }
}
impl ReportOwner<'_> {
    fn update<T>(
        &self,
        change: impl FnOnce(&mut RunReport) -> Result<T, Fault>,
        persist: bool,
    ) -> Result<T, Fault> {
        let result = change(
            &mut self
                .report
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )?;
        debug_assert!(!persist, "publication must be awaited separately");
        Ok(result)
    }
    async fn persist(&self) -> Result<(), Fault> {
        // The actor lease serializes snapshot capture and generation allocation;
        // no older concurrent snapshot can supersede a newer publication.
        let lease = self.artifacts.lease().await.map_err(|_| Fault::io())?;
        let snapshot = (**self
            .report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
        .clone();
        let generation = self
            .generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| Fault::io())?
            + 1;
        lease
            .write_report(generation, &snapshot)
            .await
            .map_err(|_| Fault::io())?;
        Ok(())
    }
    fn event(&self, kind: &str, phase: &str, table: Option<&TablePlan>) -> Result<(), Fault> {
        self.event_range(kind, phase, table, None)
    }
    fn phase_elapsed_millis(&self) -> u64 {
        self.phase_clock.elapsed().as_millis() as u64
    }
    fn phase_interval(
        &self,
        phase: &str,
        start_elapsed_millis: u64,
        end_elapsed_millis: u64,
        operation_classes: &[&str],
    ) -> Result<(), Fault> {
        let report = self
            .report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.console
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .phase_interval(
                &report.run_id,
                phase,
                start_elapsed_millis,
                end_elapsed_millis,
                operation_classes,
            )
            .map_err(|_| Fault::io())
    }
    fn copy_started(&self) {
        self.copy_window.started(self.phase_elapsed_millis());
    }
    fn copy_finished(&self) {
        self.copy_window.finished(self.phase_elapsed_millis());
    }
    fn copy_interval(&self) -> Option<(u64, u64)> {
        self.copy_window.interval()
    }
    fn event_range(
        &self,
        kind: &str,
        phase: &str,
        table: Option<&TablePlan>,
        range: Option<String>,
    ) -> Result<(), Fault> {
        let snapshot = (**self
            .report
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
        .clone();
        #[cfg(test)]
        let _ = SCOPED_WORKER_FAULT.try_with(|fault| fault(kind, table, &snapshot));
        event(
            &mut self
                .console
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            &snapshot,
            kind,
            phase,
            table,
            range,
            self.started,
        )
    }
}
fn resource_fault(error: scheduler::ResourceError) -> Fault {
    if error == scheduler::ResourceError::Cancelled {
        Fault::cancelled()
    } else {
        Fault::failed("RESOURCE", error.to_string())
    }
}
fn copy_fault(error: recovery::RecoveryError, state: &ExecutionState) -> Fault {
    let unknown = error
        .target
        .as_ref()
        .is_some_and(|target| target.kind == postgres::FailureKind::Indeterminate);
    if unknown {
        state.unknown.store(1, Ordering::Release);
    }
    let mut message = error.to_string();
    let mut target = error.target.as_ref();
    while let Some(failure) = target {
        message.push_str(&format!("; {failure}"));
        target = failure.cause.as_deref();
    }
    Fault {
        status: if unknown {
            RunStatus::Indeterminate
        } else {
            RunStatus::Failed
        },
        code: match error.kind {
            recovery::RecoveryFailure::RejectLimit => "REJECT_LIMIT",
            recovery::RecoveryFailure::RejectIo => "REJECT_ARTIFACT_IO",
            recovery::RecoveryFailure::ObserverIo => "ARTIFACT_OR_CONSOLE_IO",
            recovery::RecoveryFailure::RetryExhausted => "COPY_RETRY_EXHAUSTED",
            _ => "COPY",
        },
        message,
        shutdown_deadline: false,
    }
}
fn foreign_key_step(plan: &MigrationPlan, step: &DdlStep) -> bool {
    plan.tables
        .iter()
        .filter(|table| Some(&table.id) == step.table_id.as_ref())
        .any(|table| {
            table.structure.foreign_keys.iter().any(|key| {
                step.sql.starts_with(&format!(
                    "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY (",
                    postgres::qualified(&table.target_schema, &table.target_name),
                    postgres::quote_ident(&key.name)
                ))
            })
        })
}
fn finalization_groups(plan: &MigrationPlan) -> BTreeMap<usize, Vec<&DdlStep>> {
    let mut groups: BTreeMap<usize, Vec<&DdlStep>> = BTreeMap::new();
    for step in plan
        .ddl
        .iter()
        .filter(|step| step.phase == DdlPhase::Finalize && !foreign_key_step(plan, step))
    {
        if let Some(index) = plan
            .tables
            .iter()
            .position(|table| Some(&table.id) == step.table_id.as_ref())
        {
            groups.entry(index).or_default().push(step);
        }
    }
    groups
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    source: &mut mysql::SourceConnection,
    target: &mut postgres::TargetConnection,
    plan: &MigrationPlan,
    baseline: &BTreeMap<String, u64>,
    content_baseline: Option<verify::content::AppendContentBaseline>,
    report: &mut RunReport,
    console: &mut Console,
    artifacts: &DurableArtifacts,
    generation: &AtomicU64,
    resources: &scheduler::Resources,
    registry: &ExecutionRegistry,
    stop: &watch::Sender<bool>,
    mut stopped: watch::Receiver<bool>,
    started: Instant,
    phase_clock: Instant,
    catalog_started_elapsed_millis: u64,
) -> Result<(), Fault> {
    let owner = ReportOwner {
        report: std::sync::Mutex::new(report),
        console: std::sync::Mutex::new(console),
        artifacts,
        generation,
        started,
        phase_clock,
        copy_window: CopyPhaseWindow::default(),
    };
    let rejects = recovery::RejectBudget::new(config.migration.max_rejected_rows);
    owner.event("start", "prepare", None)?;
    execute_hooks(target, plan, true).await?;
    stopped_ddl(
        target,
        &plan
            .ddl
            .iter()
            .filter(|step| step.phase == DdlPhase::Prepare)
            .collect::<Vec<_>>(),
        &owner,
        &registry.control,
        &mut stopped,
    )
    .await?;
    owner.phase_interval(
        "catalog",
        catalog_started_elapsed_millis,
        owner.phase_elapsed_millis(),
        CATALOG_OPERATION_CLASSES,
    )?;
    if plan.mode != MigrationMode::SchemaOnly {
        if plan.consistency == Consistency::SingleSnapshot {
            for (index, table) in plan.tables.iter().enumerate() {
                let _permit = resources
                    .table(&mut stopped)
                    .await
                    .map_err(resource_fault)?;
                *registry.control.shutdown.lock().expect("shutdown registry") = None;
                registry.tables[index].register(target);
                table_pipeline(
                    config,
                    source,
                    target,
                    plan,
                    table,
                    index,
                    &owner,
                    resources,
                    &registry.tables[index],
                    &rejects,
                    _permit.workspace(),
                    stopped.clone(),
                    None,
                )
                .await?;
                *registry.tables[index]
                    .shutdown
                    .lock()
                    .expect("shutdown registry") = None;
                registry.control.register(target);
            }
        } else {
            let mut remaining = plan.tables.iter().enumerate();
            let mut workers = FuturesUnordered::new();
            for (index, table) in remaining.by_ref().take(resources.admission().table_workers) {
                workers.push(
                    AssertUnwindSafe(frozen_table(
                        config,
                        creds,
                        plan,
                        table,
                        index,
                        &owner,
                        resources,
                        &registry.tables[index],
                        registry,
                        &rejects,
                        stop,
                        stopped.clone(),
                    ))
                    .catch_unwind(),
                );
            }
            loop {
                let next = tokio::select! {
                    biased;
                    _ = cancelled(&mut stopped) => Some(Ok(Err(Fault::cancelled()))),
                    next = workers.next() => next,
                };
                let Some(result) = next else {
                    break;
                };
                let result = result.unwrap_or_else(|_| {
                    Err(Fault::failed(
                        "WORKER_PANIC",
                        "table worker panicked; all sibling pipelines are stopped",
                    ))
                });
                if let Err(first) = result {
                    registry.publish_fault(&first);
                    let first = registry.retain_fault(first);
                    let original_code = first.code;
                    let original_message = first.message.clone();
                    let _ = stop.send(true);
                    resources.close();
                    registry.cancel_sources(source).await;
                    let drain = drain_workers(first, &mut workers);
                    return match tokio::time::timeout_at(registry.shutdown_deadline(), async {
                        let (_, fault) = tokio::join!(registry.cancel_all(), drain);
                        fault
                    })
                    .await
                    {
                        Ok(fault) => Err(fault),
                        Err(_) => {
                            let mut fault = Fault::failed(original_code, original_message);
                            fault.shutdown_deadline = true;
                            Err(fault)
                        }
                    };
                }
                if !*stopped.borrow()
                    && let Some((index, table)) = remaining.next()
                {
                    workers.push(
                        AssertUnwindSafe(frozen_table(
                            config,
                            creds,
                            plan,
                            table,
                            index,
                            &owner,
                            resources,
                            &registry.tables[index],
                            registry,
                            &rejects,
                            stop,
                            stopped.clone(),
                        ))
                        .catch_unwind(),
                    );
                }
            }
            if *stopped.borrow() {
                return Err(Fault::cancelled());
            }
        }
    }
    if let Some((copy_start, copy_end)) = owner.copy_interval() {
        owner.phase_interval("copy", copy_start, copy_end, COPY_OPERATION_CLASSES)?;
    }
    owner.event("phase", "finalize", None)?;
    let index_constraints_started = owner.phase_elapsed_millis();
    let groups = finalization_groups(plan);
    let mut remaining = groups.iter();
    let mut workers = FuturesUnordered::new();
    for (&index, steps) in remaining.by_ref().take(resources.admission().index_workers) {
        workers.push(
            AssertUnwindSafe(index_group(
                config,
                creds,
                steps,
                &owner,
                resources,
                &registry.indexes[index],
                registry,
                stop,
                stopped.clone(),
            ))
            .catch_unwind(),
        );
    }
    while let Some(result) = workers.next().await {
        let result =
            result.unwrap_or_else(|_| Err(Fault::failed("WORKER_PANIC", "index worker panicked")));
        if let Err(first) = result {
            registry.publish_fault(&first);
            let first = registry.retain_fault(first);
            let original_code = first.code;
            let original_message = first.message.clone();
            let _ = stop.send(true);
            resources.close();
            let drain = drain_workers(first, &mut workers);
            return match tokio::time::timeout_at(registry.shutdown_deadline(), async {
                let (_, fault) = tokio::join!(registry.cancel_all(), drain);
                fault
            })
            .await
            {
                Ok(fault) => Err(fault),
                Err(_) => {
                    let mut fault = Fault::failed(original_code, original_message);
                    fault.shutdown_deadline = true;
                    Err(fault)
                }
            };
        }
        if !*stopped.borrow()
            && let Some((&index, steps)) = remaining.next()
        {
            workers.push(
                AssertUnwindSafe(index_group(
                    config,
                    creds,
                    steps,
                    &owner,
                    resources,
                    &registry.indexes[index],
                    registry,
                    stop,
                    stopped.clone(),
                ))
                .catch_unwind(),
            );
        }
    }
    if *stopped.borrow() {
        return Err(Fault::cancelled());
    }
    let remaining: Vec<_> = plan
        .ddl
        .iter()
        .filter(|step| {
            step.phase == DdlPhase::Finalize
                && (foreign_key_step(plan, step) || step.table_id.is_none())
        })
        .collect();
    stopped_ddl(target, &remaining, &owner, &registry.control, &mut stopped).await?;
    owner.phase_interval(
        "index_constraints",
        index_constraints_started,
        owner.phase_elapsed_millis(),
        INDEX_CONSTRAINT_OPERATION_CLASSES,
    )?;
    stopped_ddl(
        target,
        &plan
            .ddl
            .iter()
            .filter(|step| step.phase == DdlPhase::Sequence)
            .collect::<Vec<_>>(),
        &owner,
        &registry.control,
        &mut stopped,
    )
    .await?;
    owner.update(
        |report| {
            for table in &mut report.tables {
                table.status = if table.rejected_rows > 0 {
                    RunStatus::Partial
                } else {
                    RunStatus::Complete
                };
            }
            Ok(())
        },
        false,
    )?;
    let rejected = owner
        .report
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .tables
        .iter()
        .any(|table| table.rejected_rows > 0);
    if rejected {
        if plan.hooks.iter().any(|hook| !hook.before) {
            owner.update(
                |report| {
                    report.diagnostics.push(Diagnostic {
                        code: "AFTER_HOOK_SUPPRESSED".into(),
                        stage: "hooks".into(),
                        object: None,
                        severity: Severity::Warning,
                        message: "after hooks were skipped because the load rejected rows".into(),
                    });
                    Ok(())
                },
                false,
            )?;
        }
    } else {
        execute_hooks(target, plan, false).await?;
    }
    owner.event("phase", "verify", None)?;
    let tables = owner
        .report
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .tables
        .clone();
    let verified = tokio::select! {
        biased;
        _ = cancelled(&mut stopped) => return Err(Fault::cancelled()),
        result = async {
            if plan.verification == VerificationMode::Content {
                let mut schema_plan = plan.clone();
                schema_plan.verification = VerificationMode::CountsAndSchema;
                let mut combined = verify::counts_and_schema(
                    source,
                    target,
                    &schema_plan,
                    &tables,
                    baseline,
                ).await?;
                let content = verify::content::verify_content(
                    source,
                    target,
                    plan,
                    &tables,
                    verify::content::ContentVerificationOptions {
                        memory_bytes: config.migration.memory_bytes,
                        max_row_bytes: config.migration.max_row_bytes,
                        source_snapshot_already_pinned: plan.consistency
                            == Consistency::SingleSnapshot,
                        append_baseline: content_baseline,
                    },
                ).await;
                combined.tables_checked = combined.tables_checked.min(content.tables_checked);
                combined.differences.extend(content.differences);
                combined.status = if combined.status == VerificationStatus::Different
                    || content.status == VerificationStatus::Different
                {
                    VerificationStatus::Different
                } else if combined.status == VerificationStatus::Error
                    || content.status == VerificationStatus::Error
                {
                    VerificationStatus::Error
                } else if combined.status != VerificationStatus::Complete
                    || content.status != VerificationStatus::Complete
                {
                    VerificationStatus::Unsupported
                } else {
                    VerificationStatus::Complete
                };
                combined.mode = plan.verification;
                Ok::<_, verify::VerifyError>(combined)
            } else {
                verify::counts_and_schema(source, target, plan, &tables, baseline).await
            }
        } => result,
    };
    let verification = match verified {
        Ok(verification) => verification,
        Err(_) => {
            owner.update(
                |report| {
                    report.verification.status = VerificationStatus::Error;
                    Ok(())
                },
                false,
            )?;
            return Err(Fault::failed(
                "VERIFICATION_EXECUTION",
                "requested verification could not complete",
            ));
        }
    };
    owner.update(
        |report| {
            report.verification = verification;
            Ok(())
        },
        false,
    )?;
    for table in &plan.tables {
        owner.event("table_complete", "complete", Some(table))?;
    }
    Ok(())
}

async fn execute_hooks(
    target: &postgres::TargetConnection,
    plan: &MigrationPlan,
    before: bool,
) -> Result<(), Fault> {
    for hook in plan.hooks.iter().filter(|hook| hook.before == before) {
        let path = std::path::Path::new(&hook.path);
        let contents = hooks::read_sql(path)
            .map_err(|_| Fault::failed("HOOK_UNREADABLE", "planned SQL hook could not be read"))?;
        let digest = hooks::digest(&contents);
        if digest != hook.sha256 {
            return Err(Fault::failed(
                "HOOK_CHANGED",
                "SQL hook changed after planning; replan before running",
            ));
        }
        let sql = std::str::from_utf8(&contents)
            .map_err(|_| Fault::failed("HOOK_UNREADABLE", "planned SQL hook is not UTF-8"))?;
        target
            .client
            .batch_execute(sql)
            .await
            .map_err(|_| Fault::failed("HOOK_EXECUTION", "PostgreSQL SQL hook failed"))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn frozen_table(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    plan: &MigrationPlan,
    table: &TablePlan,
    index: usize,
    owner: &ReportOwner<'_>,
    resources: &scheduler::Resources,
    state: &ExecutionState,
    registry: &ExecutionRegistry,
    rejects: &recovery::RejectBudget,
    stop: &watch::Sender<bool>,
    mut stopped: watch::Receiver<bool>,
) -> Result<(), Fault> {
    let _permit = resources
        .table(&mut stopped)
        .await
        .map_err(resource_fault)?;
    let connection = async {
        let source = mysql::connect(
            &config.source,
            creds.source.expose(),
            config.migration.max_row_bytes,
        )
        .await
        .map_err(|_| Fault::failed("SOURCE_CONNECT", "table MySQL connection/session failed"))?;
        let target = postgres::connect(&config.target, creds.target.expose())
            .await
            .map_err(|_| {
                Fault::failed(
                    "TARGET_CONNECT",
                    "table PostgreSQL connection/session failed",
                )
            })?;
        Ok::<_, Fault>((source, target))
    };
    let (mut source, mut target) = tokio::select! {
        biased;
        _ = cancelled(&mut stopped) => return Err(Fault::cancelled()),
        result = connection => result?,
    };
    state.register(&target);
    state
        .source_id
        .store(u64::from(source.id()), Ordering::Release);
    let registered_source = RegisteredSource(&state.source_id);
    // Catch the pipeline while its transports and exact source registration
    // remain owned here; unwinding the outer worker would clear the kill ID.
    let result = AssertUnwindSafe(async {
        if config.migration.readers_per_table > 1 {
            let (key, unsigned) = match integer_primary_key(table) {
                Ok(key) => key,
                Err(reason) => {
                    range_fallback(owner, table, reason)?;
                    return table_pipeline(
                        config,
                        &mut source,
                        &mut target,
                        plan,
                        table,
                        index,
                        owner,
                        resources,
                        state,
                        rejects,
                        _permit.workspace(),
                        stopped.clone(),
                        None,
                    )
                    .await;
                }
            };
            let endpoints = mysql::integer_key_bounds(
                &mut source,
                &plan.source_database,
                &table.source_name,
                key,
                unsigned,
            )
            .await
            .map_err(|_| Fault::failed("RANGE_BOUNDS", "integer key bounds query failed"))?;
            if let Some((minimum, maximum)) = endpoints {
                let span = config.migration.max_key_span.expect("validated range span");
                let mut ranges = ranges::IntegerRanges::new(Some(minimum), Some(maximum), span)
                    .map_err(|_| {
                        Fault::failed("RANGE_BOUNDS", "integer key bounds were inconsistent")
                    })?;
                let first = ranges.next();
                let second = ranges.next();
                if let (Some(first), Some(second)) = (first, second) {
                    let mut seeded = VecDeque::from([first, second]);
                    while seeded.len() < config.migration.readers_per_table {
                        let Some(next) = ranges.next() else { break };
                        seeded.push_back(next);
                    }
                    let active_readers = seeded.len();
                    let pending = Arc::new(std::sync::Mutex::new((seeded, ranges)));
                    run_range_lanes(
                        config,
                        creds,
                        plan,
                        table,
                        index,
                        key,
                        owner,
                        resources,
                        registry,
                        rejects,
                        state,
                        &mut source,
                        &mut target,
                        _permit.workspace(),
                        pending,
                        active_readers,
                        stop,
                        stopped.clone(),
                    )
                    .await
                } else {
                    range_fallback(owner, table, "key domain fits within one configured span")?;
                    table_pipeline(
                        config,
                        &mut source,
                        &mut target,
                        plan,
                        table,
                        index,
                        owner,
                        resources,
                        state,
                        rejects,
                        _permit.workspace(),
                        stopped.clone(),
                        None,
                    )
                    .await
                }
            } else {
                range_fallback(owner, table, "empty table has no key bounds")?;
                table_pipeline(
                    config,
                    &mut source,
                    &mut target,
                    plan,
                    table,
                    index,
                    owner,
                    resources,
                    state,
                    rejects,
                    _permit.workspace(),
                    stopped.clone(),
                    None,
                )
                .await
            }
        } else {
            table_pipeline(
                config,
                &mut source,
                &mut target,
                plan,
                table,
                index,
                owner,
                resources,
                state,
                rejects,
                _permit.workspace(),
                stopped.clone(),
                None,
            )
            .await
        }
    })
    .catch_unwind()
    .await
    .unwrap_or_else(|_| {
        Err(Fault::failed(
            "WORKER_PANIC",
            "table worker panicked; all sibling pipelines are stopped",
        ))
    });
    if let Err(fault) = &result {
        registry.publish_fault(fault);
        let _ = stop.send(true);
        // A scheduling yield can be repolled by FuturesUnordered before its
        // coordinator sees stop. Hold the exact guards/ID until the coordinator
        // completes the termination request; no stale-ID lookup or reuse.
        let cancelled = state.source_cancelled.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        if state.source_id.load(Ordering::Acquire) != 0 {
            cancelled.await;
        }
    }
    drop(registered_source);
    let source_closed = source.disconnect().await;
    let closed = target.close().await;
    if result.is_ok() {
        source_closed
            .map_err(|_| Fault::failed("SOURCE_CLOSE", "table source socket close failed"))?;
        closed.map_err(|_| {
            if *stopped.borrow() {
                Fault::cancelled()
            } else {
                Fault::failed("TARGET_CLOSE", "table target teardown failed")
            }
        })?;
        *state.shutdown.lock().expect("shutdown registry") = None;
    }
    result
}

fn integer_primary_key(table: &TablePlan) -> Result<(&str, bool), &'static str> {
    if table.is_view {
        return Err("views do not have a rangeable table primary key");
    }
    if table.primary_key.len() != 1 {
        return Err("table does not have exactly one primary-key column");
    }
    let name = &table.primary_key[0];
    let Some(column) = table
        .columns
        .iter()
        .find(|column| column.source_name.eq_ignore_ascii_case(name))
    else {
        return Err("primary-key column metadata is missing");
    };
    if column.nullable || !column.copy {
        return Err("primary-key column is nullable or not copied");
    }
    let source_type = column.source_type.to_ascii_lowercase();
    let mut words = source_type.split_whitespace();
    let base = words
        .next()
        .unwrap_or_default()
        .split('(')
        .next()
        .unwrap_or_default();
    if !matches!(
        base,
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint"
    ) {
        return Err("primary-key column is not a supported integer type");
    }
    let unsigned = column
        .source_type
        .to_ascii_lowercase()
        .split_whitespace()
        .any(|word| word == "unsigned");
    Ok((name, unsigned))
}

fn range_fallback(owner: &ReportOwner<'_>, table: &TablePlan, reason: &str) -> Result<(), Fault> {
    owner.update(
        |report| {
            report.diagnostics.push(Diagnostic {
                code: "RANGE_FALLBACK".into(),
                stage: "copy".into(),
                object: Some(table.id.clone()),
                severity: Severity::Warning,
                message: format!(
                    "integer range readers were not used: {reason}; using the full-table stream"
                ),
            });
            Ok(())
        },
        false,
    )
}

type PendingRanges = Arc<std::sync::Mutex<(VecDeque<ranges::IntegerRange>, ranges::IntegerRanges)>>;

fn next_range(pending: &PendingRanges) -> Option<ranges::IntegerRange> {
    let mut pending = pending.lock().expect("range iterator lock");
    pending.0.pop_front().or_else(|| pending.1.next())
}

#[allow(clippy::too_many_arguments)]
async fn run_range_lanes(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    plan: &MigrationPlan,
    table: &TablePlan,
    index: usize,
    key: &str,
    owner: &ReportOwner<'_>,
    resources: &scheduler::Resources,
    registry: &ExecutionRegistry,
    rejects: &recovery::RejectBudget,
    first_state: &ExecutionState,
    first_source: &mut mysql::SourceConnection,
    first_target: &mut postgres::TargetConnection,
    first_workspace: Arc<tokio::sync::OwnedSemaphorePermit>,
    pending: PendingRanges,
    active_readers: usize,
    stop: &watch::Sender<bool>,
    stopped: watch::Receiver<bool>,
) -> Result<(), Fault> {
    let first = async {
        let result = range_lane_loop(
            config,
            plan,
            table,
            index,
            key,
            owner,
            resources,
            first_state,
            rejects,
            first_source,
            first_target,
            first_workspace,
            pending.clone(),
            stopped.clone(),
        )
        .await;
        if let Err(fault) = &result {
            registry.publish_fault(fault);
            let _ = stop.send(true);
            let cancelled = first_state.source_cancelled.notified();
            tokio::pin!(cancelled);
            cancelled.as_mut().enable();
            if first_state.source_id.load(Ordering::Acquire) != 0 {
                cancelled.await;
            }
        }
        result
    };
    let mut lanes = FuturesUnordered::new();
    for state in registry.range_lanes[index].iter().take(active_readers - 1) {
        let lane = extra_range_lane(
            config,
            creds,
            plan,
            table,
            index,
            key,
            owner,
            resources,
            registry,
            rejects,
            state,
            pending.clone(),
            stop,
            stopped.clone(),
        );
        let lane = async move {
            let result = lane.await;
            if let Err(fault) = &result {
                registry.publish_fault(fault);
                let _ = stop.send(true);
                let cancelled = state.source_cancelled.notified();
                tokio::pin!(cancelled);
                cancelled.as_mut().enable();
                if state.source_id.load(Ordering::Acquire) != 0 {
                    cancelled.await;
                }
            }
            result
        };
        lanes.push(AssertUnwindSafe(lane).catch_unwind());
    }
    let drain = async {
        let mut fault = None;
        while let Some(result) = lanes.next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    fault =
                        Some(fault.map_or(error.clone(), |old: Fault| old.retain_stronger(error)))
                }
                Err(_) => {
                    fault = Some(fault.map_or_else(
                        || Fault::failed("WORKER_PANIC", "range reader panicked"),
                        |old: Fault| {
                            old.retain_stronger(Fault::failed(
                                "WORKER_PANIC",
                                "range reader panicked",
                            ))
                        },
                    ))
                }
            }
        }
        fault
    };
    let (first, other) = tokio::join!(first, drain);
    match (first, other) {
        (Ok(()), None) => Ok(()),
        (Ok(()), Some(fault)) | (Err(fault), None) => Err(fault),
        (Err(first), Some(other)) => Err(first.retain_stronger(other)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn range_lane_loop(
    config: &MigrationConfig,
    plan: &MigrationPlan,
    table: &TablePlan,
    index: usize,
    key: &str,
    owner: &ReportOwner<'_>,
    resources: &scheduler::Resources,
    state: &ExecutionState,
    rejects: &recovery::RejectBudget,
    source: &mut mysql::SourceConnection,
    target: &mut postgres::TargetConnection,
    workspace: Arc<tokio::sync::OwnedSemaphorePermit>,
    pending: PendingRanges,
    stopped: watch::Receiver<bool>,
) -> Result<(), Fault> {
    while let Some(range) = next_range(&pending) {
        table_pipeline(
            config,
            source,
            target,
            plan,
            table,
            index,
            owner,
            resources,
            state,
            rejects,
            workspace.clone(),
            stopped.clone(),
            Some((key, range)),
        )
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn extra_range_lane(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    plan: &MigrationPlan,
    table: &TablePlan,
    index: usize,
    key: &str,
    owner: &ReportOwner<'_>,
    resources: &scheduler::Resources,
    registry: &ExecutionRegistry,
    rejects: &recovery::RejectBudget,
    state: &ExecutionState,
    pending: PendingRanges,
    stop: &watch::Sender<bool>,
    mut stopped: watch::Receiver<bool>,
) -> Result<(), Fault> {
    let permit = resources
        .reader(&mut stopped)
        .await
        .map_err(resource_fault)?;
    let (mut source, mut target) = tokio::select! {
        biased;
        _ = cancelled(&mut stopped) => return Err(Fault::cancelled()),
        result = async {
            let source = mysql::connect(&config.source, creds.source.expose(), config.migration.max_row_bytes)
                .await.map_err(|_| Fault::failed("SOURCE_CONNECT", "range MySQL connection failed"))?;
            let target = postgres::connect(&config.target, creds.target.expose())
                .await.map_err(|_| Fault::failed("TARGET_CONNECT", "range PostgreSQL connection failed"))?;
            Ok::<_, Fault>((source, target))
        } => result?,
    };
    state.register(&target);
    state
        .source_id
        .store(u64::from(source.id()), Ordering::Release);
    let registered = RegisteredSource(&state.source_id);
    let result = range_lane_loop(
        config,
        plan,
        table,
        index,
        key,
        owner,
        resources,
        state,
        rejects,
        &mut source,
        &mut target,
        permit.workspace(),
        pending,
        stopped.clone(),
    )
    .await;
    if let Err(fault) = &result {
        registry.publish_fault(fault);
        let _ = stop.send(true);
        let cancelled = state.source_cancelled.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        if state.source_id.load(Ordering::Acquire) != 0 {
            cancelled.await;
        }
    }
    drop(registered);
    let source_closed = source.disconnect().await;
    let target_closed = target.close().await;
    if result.is_ok() {
        source_closed.map_err(|_| Fault::failed("SOURCE_CLOSE", "range source close failed"))?;
        target_closed.map_err(|_| Fault::failed("TARGET_CLOSE", "range target close failed"))?;
        *state.shutdown.lock().expect("shutdown registry") = None;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn table_pipeline(
    config: &MigrationConfig,
    source: &mut mysql::SourceConnection,
    target: &mut postgres::TargetConnection,
    plan: &MigrationPlan,
    table: &TablePlan,
    index: usize,
    owner: &ReportOwner<'_>,
    resources: &scheduler::Resources,
    state: &ExecutionState,
    rejects: &recovery::RejectBudget,
    workspace: Arc<tokio::sync::OwnedSemaphorePermit>,
    mut stopped: watch::Receiver<bool>,
    range: Option<(&str, ranges::IntegerRange)>,
) -> Result<(), Fault> {
    let table_started = Instant::now();
    owner.copy_started();
    *state.copy_started.lock().expect("copy clock") = Some((index, table_started));
    let range_label =
        range.map(|(_, bounds)| format!("{}..={}", key_text(bounds.lower), key_text(bounds.upper)));
    owner.event_range("phase", "copy", Some(table), range_label.clone())?;
    let (send, receive) = mpsc::channel(config.migration.queue_batches);
    let produce = produce(
        config,
        source,
        &plan.source_database,
        table,
        index,
        owner,
        resources,
        send,
        rejects,
        workspace,
        stopped.clone(),
        range,
    );
    let consume = consume(
        config,
        target,
        table,
        index,
        owner,
        receive,
        rejects,
        state,
        table_started,
    );
    tokio::select! {
        biased;
        _ = cancelled(&mut stopped) => return Err(Fault::cancelled()),
        result = async { tokio::try_join!(produce, consume).map(|_| ()) } => result?,
    }
    owner.update(
        |report| {
            report.tables[index].copy_elapsed_millis = elapsed(table_started);
            Ok(())
        },
        false,
    )?;
    *state.copy_started.lock().expect("copy clock") = None;
    owner.copy_finished();
    owner.event_range("progress", "copy", Some(table), range_label)?;
    Ok(())
}

fn key_text(key: ranges::IntegerKey) -> String {
    match key {
        ranges::IntegerKey::Signed(value) => value.to_string(),
        ranges::IntegerKey::Unsigned(value) => value.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn produce(
    config: &MigrationConfig,
    source: &mut mysql::SourceConnection,
    database: &str,
    table: &TablePlan,
    index: usize,
    owner: &ReportOwner<'_>,
    resources: &scheduler::Resources,
    send: mpsc::Sender<EncodedBatch>,
    rejects: &recovery::RejectBudget,
    workspace: Arc<tokio::sync::OwnedSemaphorePermit>,
    mut stopped: watch::Receiver<bool>,
    range: Option<(&str, ranges::IntegerRange)>,
) -> Result<(), Fault> {
    let mut stream = match range {
        Some((key, bounds)) => {
            mysql::table_range_stream(source, database, table, key, bounds).await
        }
        None => mysql::table_stream(source, database, table).await,
    }
    .map_err(|_| Fault::failed("SOURCE_READ", "MySQL table query failed"))?;
    let mut permit = Some(
        resources
            .batch(&mut stopped)
            .await
            .map_err(resource_fault)?,
    );
    let mut buffer = Vec::with_capacity(config.migration.batch_bytes);
    let mut positions = Vec::with_capacity(config.migration.batch_rows);
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|_| Fault::failed("SOURCE_READ", "MySQL row stream failed"))?
    {
        let ordinal = owner.update(
            |report| {
                let table = &mut report.tables[index];
                table.rows_read = table
                    .rows_read
                    .checked_add(1)
                    .ok_or_else(|| Fault::failed("ROW_OVERFLOW", "source row count overflow"))?;
                Ok(table.rows_read)
            },
            false,
        )?;
        let raw = mysql::raw_values(row, config.migration.max_row_bytes)
            .map_err(|_| Fault::failed("ROW_SIZE", "source row exceeds max_row_bytes"))?;
        let encoded = match convert::encode_row_limited(table, &raw, config.migration.max_row_bytes)
        {
            Ok(encoded) => encoded,
            Err(error) if config.migration.on_row_error == RowErrorPolicy::Reject => {
                let ticket = rejects
                    .reserve()
                    .ok_or_else(|| Fault::failed("REJECT_LIMIT", "global reject limit exceeded"))?;
                owner
                    .artifacts
                    .lease()
                    .await
                    .map_err(|_| Fault::io())?
                    .reject_conversion(
                        index,
                        &RowLocator { ordinal, key: None },
                        RawRetention {
                            values: raw,
                            permit: workspace.clone(),
                        },
                        &Diagnostic {
                            code: "CONVERSION_ROW_REJECTED".into(),
                            stage: "conversion".into(),
                            object: Some(table.id.clone()),
                            severity: Severity::Warning,
                            message: error.to_string(),
                        },
                        ticket.0,
                    )
                    .await
                    .map_err(|_| Fault::io())?;
                owner.update(
                    |report| {
                        report.tables[index].rejected_rows = report.tables[index]
                            .rejected_rows
                            .checked_add(1)
                            .ok_or_else(|| {
                                Fault::failed("ROW_OVERFLOW", "rejected row count overflow")
                            })?;
                        Ok(())
                    },
                    false,
                )?;
                owner.persist().await?;
                continue;
            }
            Err(error) => return Err(Fault::failed("CONVERSION", error.to_string())),
        };
        owner.update(
            |report| {
                for (column, value) in table.columns.iter().filter(|column| column.copy).zip(&raw) {
                    if convert::transformation_applied(column, value)
                        .map_err(|error| Fault::failed("TRANSFORMATION", error.to_string()))?
                    {
                        let count = report.tables[index]
                            .transformations
                            .entry(column.source_name.clone())
                            .or_default();
                        *count = count.checked_add(1).ok_or_else(|| {
                            Fault::failed("ROW_OVERFLOW", "transformation count overflow")
                        })?;
                    }
                }
                Ok(())
            },
            false,
        )?;
        if !positions.is_empty() && buffer.len() + encoded.len() > config.migration.batch_bytes {
            send_batch(
                &send,
                &mut buffer,
                &mut positions,
                permit.take().expect("filling batch permit"),
                &mut stopped,
            )
            .await?;
            permit = Some(
                resources
                    .batch(&mut stopped)
                    .await
                    .map_err(resource_fault)?,
            );
            buffer = Vec::with_capacity(config.migration.batch_bytes);
            positions = Vec::with_capacity(config.migration.batch_rows);
        }
        let start = buffer.len();
        buffer.extend_from_slice(&encoded);
        positions.push(RowPosition {
            start,
            end: buffer.len(),
            locator: RowLocator { ordinal, key: None },
        });
        if positions.len() == config.migration.batch_rows
            || buffer.len() == config.migration.batch_bytes
        {
            send_batch(
                &send,
                &mut buffer,
                &mut positions,
                permit.take().expect("filling batch permit"),
                &mut stopped,
            )
            .await?;
            permit = Some(
                resources
                    .batch(&mut stopped)
                    .await
                    .map_err(resource_fault)?,
            );
            buffer = Vec::with_capacity(config.migration.batch_bytes);
            positions = Vec::with_capacity(config.migration.batch_rows);
        }
    }
    if !positions.is_empty() {
        send_batch(
            &send,
            &mut buffer,
            &mut positions,
            permit.take().expect("filling batch permit"),
            &mut stopped,
        )
        .await?;
    }
    Ok(())
}

async fn send_batch(
    send: &mpsc::Sender<EncodedBatch>,
    buffer: &mut Vec<u8>,
    positions: &mut Vec<RowPosition>,
    permit: tokio::sync::OwnedSemaphorePermit,
    stopped: &mut watch::Receiver<bool>,
) -> Result<(), Fault> {
    let batch = EncodedBatch {
        storage: Arc::new(BatchStorage {
            bytes: Bytes::from(std::mem::take(buffer)),
            permit,
        }),
        rows: std::mem::take(positions),
    };
    tokio::select! {
        biased;
        _ = cancelled(stopped) => Err(Fault::cancelled()),
        result = send.send(batch) => result.map_err(|_| Fault::failed("QUEUE_CLOSED", "table writer stopped before producer finished")),
    }
}

#[allow(clippy::too_many_arguments)]
async fn consume(
    config: &MigrationConfig,
    target: &mut postgres::TargetConnection,
    table: &TablePlan,
    index: usize,
    owner: &ReportOwner<'_>,
    mut receive: mpsc::Receiver<EncodedBatch>,
    rejects: &recovery::RejectBudget,
    state: &ExecutionState,
    table_started: Instant,
) -> Result<(), Fault> {
    while let Some(batch) = receive.recv().await {
        commit_batch(
            target,
            table,
            &batch,
            owner,
            index,
            state,
            config.migration.on_row_error,
            rejects,
            table_started,
        )
        .await?;
        owner.event("progress", "copy", Some(table))?;
        owner.persist().await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn commit_batch(
    target: &mut postgres::TargetConnection,
    table: &TablePlan,
    batch: &EncodedBatch,
    owner: &ReportOwner<'_>,
    index: usize,
    state: &ExecutionState,
    policy: RowErrorPolicy,
    rejects: &recovery::RejectBudget,
    table_started: Instant,
) -> Result<(), Fault> {
    let mut previous = recovery::RecoveryProgress::default();
    let mut observer = |progress: recovery::RecoveryProgress| {
        // Merge deltas, including concurrent producer conversion rejects. Never
        // overwrite a newer durable producer rejection with a stale batch baseline.
        owner
            .update(
                |report| {
                    let reported = &mut report.tables[index];
                    let committed = progress.committed_rows - previous.committed_rows;
                    reported.committed_rows = reported
                        .committed_rows
                        .checked_add(committed)
                        .ok_or_else(|| Fault::failed("ROW_OVERFLOW", "ACK count overflow"))?;
                    reported.rejected_rows = reported
                        .rejected_rows
                        .checked_add(progress.rejected_rows - previous.rejected_rows)
                        .ok_or_else(|| Fault::failed("ROW_OVERFLOW", "reject count overflow"))?;
                    if committed > 0 {
                        let first = (previous.committed_rows + previous.rejected_rows) as usize;
                        let end = first + committed as usize;
                        let bytes = batch.rows[end - 1].end - batch.rows[first].start;
                        reported.committed_bytes = reported
                            .committed_bytes
                            .checked_add(bytes as u64)
                            .ok_or_else(|| {
                                Fault::failed("ROW_OVERFLOW", "ACK byte count overflow")
                            })?;
                    }
                    reported.copy_elapsed_millis = elapsed(table_started);
                    Ok(())
                },
                false,
            )
            .map_err(|_| std::io::Error::other("report accounting failed"))?;
        state
            .inflight
            .store(progress.active_rows, Ordering::Release);
        state.phase.store(
            if progress.active_rows > 0 { 3 } else { 0 },
            Ordering::Release,
        );
        previous = progress;
        Ok(())
    };
    recovery::copy_with_durable_recovery(
        target,
        table,
        batch,
        &mut recovery::DurableRecoveryContext {
            policy,
            owner,
            table_index: index,
            reject_budget: rejects,
            observer: &mut observer,
        },
    )
    .await
    .map_err(|error| copy_fault(error, state))?;
    state.phase.store(0, Ordering::Release);
    state.inflight.store(0, Ordering::Release);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn index_group(
    config: &MigrationConfig,
    creds: &ResolvedCredentials,
    steps: &[&DdlStep],
    owner: &ReportOwner<'_>,
    resources: &scheduler::Resources,
    state: &ExecutionState,
    registry: &ExecutionRegistry,
    stop: &watch::Sender<bool>,
    mut stopped: watch::Receiver<bool>,
) -> Result<(), Fault> {
    let _permit = resources
        .index(&mut stopped)
        .await
        .map_err(resource_fault)?;
    let target = tokio::select! {
        biased;
        _ = cancelled(&mut stopped) => return Err(Fault::cancelled()),
        target = postgres::connect(&config.target, creds.target.expose()) => target.map_err(|_| Fault::failed("INDEX_CONNECT", "index target connection/session failed"))?,
    };
    state.register(&target);
    let result = tokio::select! {
        biased;
        _ = cancelled(&mut stopped) => Err(Fault::cancelled()),
        result = ddl(&target, steps, owner, state) => result,
    };
    if let Err(fault) = &result {
        registry.publish_fault(fault);
        let _ = stop.send(true);
    }
    let closed = target.close().await;
    if result.is_ok() {
        closed.map_err(|_| {
            if *stopped.borrow() {
                Fault::cancelled()
            } else {
                Fault::failed("INDEX_CLOSE", "index target teardown failed")
            }
        })?;
        *state.shutdown.lock().expect("shutdown registry") = None;
    }
    result
}

async fn stopped_ddl(
    target: &postgres::TargetConnection,
    steps: &[&DdlStep],
    owner: &ReportOwner<'_>,
    state: &ExecutionState,
    stopped: &mut watch::Receiver<bool>,
) -> Result<(), Fault> {
    tokio::select! {
        biased;
        _ = cancelled(stopped) => Err(Fault::cancelled()),
        result = ddl(target, steps, owner, state) => result,
    }
}
async fn ddl(
    target: &postgres::TargetConnection,
    steps: &[&DdlStep],
    owner: &ReportOwner<'_>,
    state: &ExecutionState,
) -> Result<(), Fault> {
    for step in steps {
        owner.update(
            |report| {
                report.failed_steps.push(step.object.clone());
                Ok(())
            },
            false,
        )?;
        *state.step.lock().expect("DDL step registry") = Some(step.object.clone());
        state.phase.store(1, Ordering::Release);
        let operation = async {
            target.client.batch_execute("BEGIN").await?;
            target.client.batch_execute(&step.sql).await?;
            state.phase.store(2, Ordering::Release);
            target.client.batch_execute("COMMIT").await
        };
        match tokio::time::timeout(Duration::from_secs(60), operation).await {
            Ok(Ok(())) => {
                owner.update(
                    |report| {
                        if let Some(index) = report
                            .failed_steps
                            .iter()
                            .rposition(|object| object == &step.object)
                        {
                            report.failed_steps.remove(index);
                        }
                        Ok(())
                    },
                    false,
                )?;
                *state.step.lock().expect("DDL step registry") = None;
                state.phase.store(0, Ordering::Release);
            }
            Ok(Err(error)) => {
                let indeterminate =
                    state.phase.load(Ordering::Acquire) == 2 && error.code().is_none();
                if !indeterminate {
                    // A server error acknowledges failure even if subsequent
                    // cleanup loses the connection. Do not turn it into an
                    // unknown COMMIT merely because ROLLBACK also fails.
                    state.phase.store(1, Ordering::Release);
                    match tokio::time::timeout(
                        Duration::from_secs(3),
                        target.client.batch_execute("ROLLBACK"),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        _ => {
                            return Err(Fault::failed(
                                "DDL_ROLLBACK",
                                format!(
                                    "planned DDL failed (SQLSTATE {}); rollback could not be acknowledged; target connection will be discarded",
                                    error.code().map_or("unavailable", |code| code.code())
                                ),
                            ));
                        }
                    }
                }
                return Err(Fault {
                    status: if indeterminate {
                        RunStatus::Indeterminate
                    } else {
                        RunStatus::Failed
                    },
                    code: "DDL",
                    message: format!(
                        "planned DDL failed (SQLSTATE {})",
                        error
                            .code()
                            .map(|code| code.code())
                            .unwrap_or("unavailable")
                    ),
                    shutdown_deadline: false,
                });
            }
            Err(_) => {
                return Err(Fault::failed(
                    "DDL_DEADLINE",
                    "planned DDL exceeded the 60-second transaction deadline",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod worker_fault_tests {
    use super::*;
    use futures_util::future::BoxFuture;

    #[tokio::test]
    async fn cancelled_sibling_finishes_before_delayed_fatal_teardown() {
        let mut workers: FuturesUnordered<BoxFuture<'_, std::thread::Result<Result<(), Fault>>>> =
            FuturesUnordered::new();
        let permits = Arc::new(Semaphore::new(2));
        let fatal_permit = permits.clone().acquire_owned().await.unwrap();
        workers.push(
            async move {
                let _owned_until_teardown = fatal_permit;
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(Err(Fault::failed(
                    "REJECT_LIMIT",
                    "already observed durable cap failure",
                )))
            }
            .boxed(),
        );
        workers.push(async { Ok(Err(Fault::cancelled())) }.boxed());
        let first = workers.next().await.unwrap().unwrap().unwrap_err();
        assert_eq!(first.status, RunStatus::Cancelled);
        let fault = drain_workers(first, &mut workers).await;
        assert_eq!(
            (fault.status, fault.code),
            (RunStatus::Failed, "REJECT_LIMIT")
        );
        assert_eq!(permits.available_permits(), 2);
    }
    #[tokio::test]
    async fn scoped_panic_releases_owned_permit_and_unknown_commit_still_wins() {
        let permits = Arc::new(Semaphore::new(1));
        let permit = permits.clone().acquire_owned().await.unwrap();
        let mut workers: FuturesUnordered<BoxFuture<'_, std::thread::Result<Result<(), Fault>>>> =
            FuturesUnordered::new();
        workers.push(
            AssertUnwindSafe(async move {
                let _owned_until_unwind = permit;
                panic!("owned worker fault");
                #[allow(unreachable_code)]
                Ok(())
            })
            .catch_unwind()
            .boxed(),
        );
        let mut unknown = Fault::failed("COMMIT_INDETERMINATE", "actual stage evidence retained");
        unknown.status = RunStatus::Indeterminate;
        let fault = drain_workers(unknown, &mut workers).await;
        assert_eq!(fault.status, RunStatus::Indeterminate);
        assert_eq!(fault.code, "COMMIT_INDETERMINATE");
        assert_eq!(permits.available_permits(), 1);
    }
    #[test]
    fn published_failure_survives_synthetic_stop_and_expired_teardown() {
        let registry = ExecutionRegistry::new(2, 1, 0);
        registry.publish_fault(&Fault::failed("REJECT_LIMIT", "first observed fatal cause"));
        registry.publish_fault(&Fault::failed("SOURCE_CLOSE", "later teardown failure"));
        let mut stopped = Fault::cancelled();
        stopped.shutdown_deadline = true;
        let retained = registry.retain_fault(stopped);
        assert_eq!(
            (retained.status, retained.code),
            (RunStatus::Failed, "REJECT_LIMIT")
        );
        assert_eq!(retained.message, "first observed fatal cause");
        assert!(retained.shutdown_deadline);
        let mut unknown = Fault::failed("COMMIT_INDETERMINATE", "unacknowledged COMMIT");
        unknown.status = RunStatus::Indeterminate;
        registry.publish_fault(&unknown);
        assert_eq!(
            registry.retain_fault(retained).status,
            RunStatus::Indeterminate
        );
    }

    #[tokio::test]
    #[ignore = "requires owned native MySQL84/PostgreSQL16 TLS fixture; private scoped runner panic"]
    async fn native_runner_panic_after_ack_retains_prefix_and_closes_sibling() {
        native_runner_panic(false).await;
    }

    #[tokio::test]
    #[ignore = "requires owned native MySQL84/PostgreSQL16 TLS fixture; private scoped unread-source panic"]
    async fn native_runner_panic_preserves_registered_unread_source_backend() {
        native_runner_panic(true).await;
    }

    async fn native_runner_panic(unread_source: bool) {
        use clap::Parser;
        use std::{env, path::PathBuf, sync::atomic::AtomicBool};
        let required = |name: &str| {
            env::var(name).unwrap_or_else(|_| panic!("required native fixture {name}"))
        };
        // The libtest executable is not the production artifact worker entry point.
        let artifact_executable = PathBuf::from(required("MY2PG_TEST_ARTIFACT_EXE"));
        assert!(artifact_executable.is_file());
        let schema = format!(
            "t13panic_{}_{}",
            if unread_source { "unread" } else { "small" },
            std::process::id()
        );
        let names = [format!("{schema}_a"), format!("{schema}_b")];
        let app = format!("{schema}_run");
        let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,
            "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},
            "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":required("MY2PG_TLS_CA"),"session":{"application_name":app}},
            "migration":{"mode":"schema_only","table_workers":2,"index_workers":2,"queue_batches":1,"batch_rows":1,"batch_bytes":1024,"max_row_bytes":1024,"memory_bytes":1048576,"reset_sequences":false},
            "tables":{"include":names},"report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&schema),"progress":"never"}})).unwrap();
        let credentials = config::resolve_credentials(&config).unwrap();
        let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1048576)
            .await
            .unwrap();
        for name in &names {
            source.query_drop(format!("CREATE TABLE {}(id INT NOT NULL PRIMARY KEY,value TEXT NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",mysql::quote_ident(name))).await.unwrap();
            source
                .query_drop(format!(
                    "INSERT INTO {} VALUES(1,'ack'),(2,'tail')",
                    mysql::quote_ident(name)
                ))
                .await
                .unwrap();
        }
        if unread_source {
            config.migration.max_row_bytes = 4096;
            config.migration.batch_bytes = 4096;
            source.query_drop(format!("DELETE FROM {}; INSERT INTO {} WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM seq WHERE n<500) SELECT (a.n-1)*100+b.n,IF(a.n=1 AND b.n=1,'ack',REPEAT('x',2048)) FROM seq a JOIN seq b ON b.n<=100", mysql::quote_ident(&names[0]), mysql::quote_ident(&names[0]))).await.unwrap();
            let (rows, bytes): (u64, u64) = source
                .query_first(format!(
                    "SELECT COUNT(*),SUM(OCTET_LENGTH(value)) FROM {}",
                    mysql::quote_ident(&names[0])
                ))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(rows, 50_000);
            assert!(bytes > 100_000_000);
            eprintln!("unread native source: rows={rows} payload_bytes={bytes}");
        }
        let args =
            crate::cli::Args::try_parse_from(["my2pg", "run", "native.toml", "--quiet"]).unwrap();
        let (_, receiver) = watch::channel(false);
        let mut console = Console::new(&config, &args);
        let schema_report = run_with_artifact_executable(
            &config,
            &credentials,
            &mut console,
            receiver,
            &artifact_executable,
        )
        .await
        .unwrap();
        assert_eq!(schema_report.exit_code(), 0, "{schema_report:?}");
        config.migration.mode = MigrationMode::DataOnly;
        config.target.on_existing = ExistingPolicy::Append;
        let pipelines = 2 * scheduler::MemoryLayout::compute(&config.migration)
            .unwrap()
            .pipeline_reservation;
        let copy_plan = plan(&config, &credentials).await.unwrap();
        let identity = ArtifactIdentity::new(&config.report.directory).unwrap();
        let mut initial_report = schema_report.clone();
        initial_report.run_id = identity.run_id.clone();
        initial_report.artifact_dir = identity.directory.to_string_lossy().into_owned();
        initial_report.mode = copy_plan.mode;
        initial_report.diagnostics = copy_plan.diagnostics.clone();
        initial_report.exclusions = copy_plan.exclusions.clone();
        initial_report.diagnostics.push(Diagnostic {
            code: "RESOURCE_ADMISSION".into(),
            stage: "scheduling".into(),
            object: None,
            severity: Severity::Warning,
            message: resource_message(
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
                usize::MAX,
            ),
        });
        let artifact_bytes = ArtifactLimits::derive(
            &identity,
            &copy_plan,
            &initial_report,
            config.migration.max_row_bytes,
        )
        .unwrap()
        .reservation_bytes;
        config.migration.memory_bytes = pipelines.checked_add(artifact_bytes).unwrap();
        let admission = scheduler::Admission::new_with_artifact_reservation(
            &config.migration,
            config.source.consistency,
            copy_plan.tables.len(),
            finalization_groups(&copy_plan).len(),
            artifact_bytes,
        )
        .unwrap();
        assert_eq!(config.migration.memory_bytes - artifact_bytes, pipelines);
        assert_eq!(admission.table_workers, 2);
        let mut admin = config.target.clone();
        admin
            .session
            .insert("application_name".into(), format!("{schema}_admin"));
        let target = postgres::connect(&admin, credentials.target.expose())
            .await
            .unwrap();
        let gate0 = postgres::connect(&admin, credentials.target.expose())
            .await
            .unwrap();
        gate0
            .client
            .batch_execute(&format!(
                "BEGIN; INSERT INTO {} VALUES(1,'locker')",
                postgres::qualified(&schema, &names[0])
            ))
            .await
            .unwrap();
        target
            .client
            .batch_execute(&format!(
                "BEGIN; INSERT INTO {} VALUES(1,'locker')",
                postgres::qualified(&schema, &names[1])
            ))
            .await
            .unwrap();
        let user = mysql_async::Opts::from_url(credentials.source.expose())
            .unwrap()
            .user()
            .unwrap()
            .to_owned();
        let kills_before: (String, String) = source
            .query_first("SHOW GLOBAL STATUS LIKE 'Com_kill'")
            .await
            .unwrap()
            .unwrap();
        let unread_id = Arc::new(AtomicU64::new(0));
        let panic_source_id = unread_id.clone();
        let triggered = Arc::new(AtomicBool::new(false));
        let observed = triggered.clone();
        let selected = names[0].clone();
        let callback = move |kind: &str, table: Option<&TablePlan>, snapshot: &RunReport| {
            if kind == "progress"
                && table.is_some_and(|table| table.source_name == selected)
                && snapshot
                    .tables
                    .iter()
                    .any(|table| table.source_name == selected && table.committed_rows == 1)
                && !observed.swap(true, Ordering::AcqRel)
            {
                if unread_source {
                    assert_ne!(
                        panic_source_id.load(Ordering::Acquire),
                        0,
                        "native source backend was observed unread before releasing ACK"
                    );
                }
                let disk: RunReport = serde_json::from_slice(
                    &std::fs::read(PathBuf::from(&snapshot.artifact_dir).join("report.json"))
                        .unwrap(),
                )
                .unwrap();
                let acknowledged = disk
                    .tables
                    .iter()
                    .find(|table| table.source_name == selected)
                    .unwrap();
                assert_eq!(
                    (acknowledged.committed_rows, acknowledged.committed_bytes),
                    (1, 6)
                );
                panic!("scoped native post-ACK worker panic");
            }
        };
        let observe = async {
            tokio::time::timeout(Duration::from_secs(15),async {
                loop {
                    target.client.batch_execute("SELECT pg_stat_clear_snapshot()").await.unwrap();
                    let count:i64=target.client.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND backend_type='client backend' AND query LIKE 'COPY %' AND wait_event_type='Lock'",&[&app]).await.unwrap().get(0);
                    if count==2 {break;}tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.expect("both actual COPY workers must block before releasing only the post-ACK panic worker");
            if unread_source {
                let pattern = format!("%`{}`%", names[0]);
                tokio::time::timeout(Duration::from_secs(15),async {
                    let mut previous=0;
                    let mut stable=0;
                    loop {
                        let id:Option<u64>=source.exec_first("SELECT ID FROM information_schema.processlist WHERE USER=? AND DB=DATABASE() AND INFO LIKE ? AND COMMAND='Execute' AND STATE='Sending to client'",(&user,&pattern)).await.unwrap();
                        if let Some(id)=id {
                            if id==previous {stable+=1;} else {previous=id;stable=1;}
                            if stable>=5 {
                                assert_eq!(source.exec_first::<u64,_,_>("SELECT COUNT(*) FROM information_schema.processlist WHERE USER=? AND DB=DATABASE()",(&user,)).await.unwrap().unwrap(),3,"one Frozen control plus two admitted readers");
                                unread_id.store(id,Ordering::Release);
                                eprintln!("unread native source backend={id} Sending to client samples={stable}");
                                break;
                            }
                        } else {stable=0;previous=0;}
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }).await.expect("exact panic worker must retain a real unread server query beyond bounded queue/socket buffering");
            }
            gate0.client.batch_execute("ROLLBACK").await.unwrap();
        };
        let (_, receiver) = watch::channel(false);
        let mut console = Console::new(&config, &args);
        let (report, ()) = tokio::join!(
            SCOPED_WORKER_FAULT.scope(
                Box::new(callback),
                run_with_artifact_executable(
                    &config,
                    &credentials,
                    &mut console,
                    receiver,
                    &artifact_executable
                )
            ),
            observe
        );
        let report = report.unwrap();
        assert!(triggered.load(Ordering::Acquire));
        assert_eq!(report.status, RunStatus::Failed, "{report:?}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "WORKER_PANIC")
        );
        assert_eq!(
            (
                report.tables[0].committed_rows,
                report.tables[0].committed_bytes
            ),
            (1, 6)
        );
        assert_eq!(report.tables[1].committed_rows, 0);
        assert!(
            report
                .tables
                .iter()
                .all(|table| table.indeterminate_rows == 0
                    && table.accounted_rows() == Some(table.rows_read))
        );
        let disk: RunReport = serde_json::from_slice(
            &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(disk.tables[0].committed_rows, 1);
        assert_eq!(disk.status, RunStatus::Failed);
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {target.client.batch_execute("SELECT pg_stat_clear_snapshot()").await.unwrap();let count:i64=target.client.query_one("SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND backend_type='client backend'",&[&app]).await.unwrap().get(0);
                if count==0 {break;}tokio::time::sleep(Duration::from_millis(10)).await;}
        }).await.expect("all actual worker/control target sockets must close after scoped panic");
        if unread_source {
            let kills_after: (String, String) = source
                .query_first("SHOW GLOBAL STATUS LIKE 'Com_kill'")
                .await
                .unwrap()
                .unwrap();
            let kill_delta =
                kills_after.1.parse::<u64>().unwrap() - kills_before.1.parse::<u64>().unwrap();
            assert_eq!(
                kill_delta, 2,
                "coordinator must still own and target the panicking reader ID plus its sibling"
            );
            eprintln!("native registered-reader termination requests={kill_delta}");
            let id = unread_id.load(Ordering::Acquire);
            assert_ne!(id, 0);
            tokio::time::timeout(Duration::from_secs(5), async {
                while source
                    .exec_first::<u64, _, _>(
                        "SELECT COUNT(*) FROM information_schema.processlist WHERE ID=?",
                        (id,),
                    )
                    .await
                    .unwrap()
                    .unwrap()
                    != 0
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect(
                "exact unread panic backend must disappear while sibling COPY gate remains held",
            );
        }
        tokio::time::timeout(Duration::from_secs(5),async {
            while source.exec_first::<u64,_,_>("SELECT count(*) FROM information_schema.processlist WHERE USER=? AND DB=DATABASE()",(&user,)).await.unwrap().unwrap()!=0 {tokio::time::sleep(Duration::from_millis(10)).await;}
        }).await.expect("all source worker/control sockets close after scoped panic");
        target.client.batch_execute("ROLLBACK").await.unwrap();
        for (index, expected) in [1i64, 0].into_iter().enumerate() {
            let count: i64 = target
                .client
                .query_one(
                    &format!(
                        "SELECT count(*) FROM {}",
                        postgres::qualified(&schema, &names[index])
                    ),
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(count, expected, "no panic-induced replay");
        }
        target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA {} CASCADE",
                postgres::quote_ident(&schema)
            ))
            .await
            .unwrap();
        for name in &names {
            source
                .query_drop(format!("DROP TABLE {}", mysql::quote_ident(name)))
                .await
                .unwrap();
        }
        source.disconnect().await.unwrap();
        gate0.close().await.unwrap();
        target.close().await.unwrap();
    }
}
