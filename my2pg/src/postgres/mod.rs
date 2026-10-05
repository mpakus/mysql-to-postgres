//! PostgreSQL target transport. Acknowledged COMMIT is the accounting boundary.
pub mod catalog_graph;
pub mod observed;
pub mod sequences;
use crate::{
    config::{TargetConfig, TlsMode},
    model::{
        EncodedBatch, TablePlan, TargetCatalog, TargetColumn, TargetDeparseObserved,
        TargetDependency, TargetNamespaceObserved, TargetTable,
    },
};
use bytes::Bytes;
use futures_util::SinkExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_postgres::{Client, Config, GenericClient, IsolationLevel, NoTls, config::SslMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CopyStage {
    Idle,
    Begin,
    CopyInit,
    CopyData,
    CopyFinish,
    CommitAttempt,
    Rollback,
    Complete,
}
#[derive(Clone, Debug)]
pub struct StageHandle(Arc<AtomicU8>);
impl StageHandle {
    pub fn get(&self) -> CopyStage {
        match self.0.load(Ordering::Acquire) & 0x7f {
            1 => CopyStage::Begin,
            2 => CopyStage::CopyInit,
            3 => CopyStage::CopyData,
            4 => CopyStage::CopyFinish,
            5 => CopyStage::CommitAttempt,
            6 => CopyStage::Rollback,
            7 => CopyStage::Complete,
            _ => CopyStage::Idle,
        }
    }
    fn set(&self, stage: CopyStage) {
        let _ = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                Some((state & 0x80) | stage as u8)
            });
    }
    fn request_cancel(&self) {
        self.0.fetch_or(0x80, Ordering::AcqRel);
    }
    fn canceled(&self) -> bool {
        self.0.load(Ordering::Acquire) & 0x80 != 0
    }
    fn enter_commit(&self) -> bool {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state & 0x80 == 0).then_some(CopyStage::CommitAttempt as u8)
            })
            .is_ok()
    }
}
#[derive(Clone)]
pub struct ShutdownHandle {
    task: AbortHandle,
    pub stage: StageHandle,
    token: tokio_postgres::CancelToken,
    tls: CancelTls,
}
#[derive(Clone)]
enum CancelTls {
    Plain,
    Verified(postgres_native_tls::MakeTlsConnector),
}
impl std::fmt::Debug for ShutdownHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownHandle")
            .field("stage", &self.stage)
            .finish_non_exhaustive()
    }
}
impl ShutdownHandle {
    pub fn abort(&self) -> CopyStage {
        self.stage.request_cancel();
        self.task.abort();
        self.stage.get()
    }
    /// Cancellation has no server acknowledgement. The stage governs recovery;
    /// COMMIT already attempted remains indeterminate regardless of this result.
    pub async fn cancel(&self) -> Result<CopyStage, TargetError> {
        self.stage.request_cancel();
        let request = async {
            match &self.tls {
                CancelTls::Plain => self.token.cancel_query(NoTls).await,
                CancelTls::Verified(tls) => self.token.cancel_query(tls.clone()).await,
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(3), request).await;
        self.task.abort();
        let stage = self.stage.get();
        match result {
            Ok(Ok(())) => Ok(stage),
            Ok(Err(error)) => Err(TargetError::database(error, stage)),
            Err(_) => Err(TargetError {
                stage,
                kind: if stage == CopyStage::CommitAttempt {
                    FailureKind::Indeterminate
                } else {
                    FailureKind::Operational
                },
                sqlstate: None,
                message: "target cancellation deadline expired",
                cause: None,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Configuration,
    Operational,
    RowData,
    TransactionRetry,
    Indeterminate,
    Rollback,
}
#[derive(Debug, thiserror::Error)]
#[error("PostgreSQL {stage:?} failed ({kind:?}, SQLSTATE {sqlstate:?}): {message}")]
pub struct TargetError {
    pub stage: CopyStage,
    pub kind: FailureKind,
    pub sqlstate: Option<String>,
    pub message: &'static str,
    /// Original sanitized operation failure when cleanup failed too.
    pub cause: Option<Box<TargetError>>,
}
impl TargetError {
    fn configuration(message: &'static str) -> Self {
        Self {
            stage: CopyStage::Idle,
            kind: FailureKind::Configuration,
            sqlstate: None,
            message,
            cause: None,
        }
    }
    fn database(error: tokio_postgres::Error, stage: CopyStage) -> Self {
        let sqlstate = error.code().map(|code| code.code().to_owned());
        let confirmed_commit_abort = sqlstate.as_deref().is_some_and(|state| {
            state != "40003"
                && (state.starts_with("22") || state.starts_with("23") || state.starts_with("40"))
        });
        let kind = if stage == CopyStage::CommitAttempt && !confirmed_commit_abort {
            FailureKind::Indeterminate
        } else if stage == CopyStage::Rollback {
            FailureKind::Rollback
        } else if matches!(sqlstate.as_deref(), Some("40001" | "40P01")) {
            FailureKind::TransactionRetry
        } else if matches!(stage, CopyStage::CopyData | CopyStage::CopyFinish)
            && matches!(
                sqlstate.as_deref(),
                Some(
                    "22001"
                        | "22003"
                        | "22007"
                        | "22008"
                        | "22021"
                        | "22P02"
                        | "23502"
                        | "23505"
                        | "23514"
                )
            )
        {
            FailureKind::RowData
        } else {
            FailureKind::Operational
        };
        Self {
            stage,
            kind,
            sqlstate,
            message: "database operation did not complete",
            cause: None,
        }
    }
}

pub struct TargetConnection {
    pub client: Client,
    task: Option<JoinHandle<Result<(), tokio_postgres::Error>>>,
    stage: StageHandle,
    cancel_tls: CancelTls,
}
impl std::fmt::Debug for TargetConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TargetConnection")
            .field("stage", &self.stage)
            .finish_non_exhaustive()
    }
}
impl Drop for TargetConnection {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
impl TargetConnection {
    pub fn stage_handle(&self) -> StageHandle {
        self.stage.clone()
    }
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            task: self.task.as_ref().expect("live target").abort_handle(),
            stage: self.stage.clone(),
            token: self.client.cancel_token(),
            tls: self.cancel_tls.clone(),
        }
    }
    pub async fn close(mut self) -> Result<(), TargetError> {
        // Close client before waiting: a live client keeps the protocol task alive.
        let task = self.task.take().expect("live target");
        drop(self);
        let mut task = task;
        match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(error))) => Err(TargetError::database(error, CopyStage::Idle)),
            Ok(Err(_)) => Err(TargetError::configuration("connection task failed")),
            Err(_) => {
                task.abort();
                let _ = task.await;
                Err(TargetError::configuration(
                    "connection close deadline expired",
                ))
            }
        }
    }
}

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
pub fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_ident(schema), quote_ident(name))
}

fn private_file(path: &Path) -> Result<String, TargetError> {
    use std::io::Read;
    let metadata = fs::metadata(path)
        .map_err(|_| TargetError::configuration("cannot read configured password file"))?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(TargetError::configuration(
            "password file must be a regular file under 1 MiB",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(TargetError::configuration(
                "password file must be private (chmod 600)",
            ));
        }
    }
    // Validate and read the same opened file, and cap reads even if it grows.
    let file = fs::File::open(path)
        .map_err(|_| TargetError::configuration("cannot read configured password file"))?;
    let metadata = file
        .metadata()
        .map_err(|_| TargetError::configuration("cannot inspect configured password file"))?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(TargetError::configuration(
            "password file must be a regular file under 1 MiB",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(TargetError::configuration(
                "password file must be private (chmod 600)",
            ));
        }
    }
    let mut contents = String::new();
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut contents)
        .map_err(|_| TargetError::configuration("password file must be readable UTF-8"))?;
    if contents.len() > 1024 * 1024 {
        return Err(TargetError::configuration(
            "password file must be a regular file under 1 MiB",
        ));
    }
    Ok(contents)
}
fn passfields(line: &str) -> Option<Vec<String>> {
    let mut fields = vec![String::new()];
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            fields.last_mut()?.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            fields.push(String::new());
        } else {
            fields.last_mut()?.push(ch);
        }
    }
    (!escaped && fields.len() == 5).then_some(fields)
}
fn passfile(path: &Path, config: &Config, host: &str) -> Result<String, TargetError> {
    let port = config
        .get_ports()
        .first()
        .copied()
        .unwrap_or(5432)
        .to_string();
    let user = config
        .get_user()
        .ok_or_else(|| TargetError::configuration("target username required"))?;
    let database = config.get_dbname().unwrap_or(user);
    for line in private_file(path)?
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields = passfields(line)
            .ok_or_else(|| TargetError::configuration("malformed password-file entry"))?;
        if fields[..4]
            .iter()
            .zip([host, port.as_str(), database, user])
            .all(|(entry, value)| entry == "*" || entry == value)
        {
            return Ok(fields[4].clone());
        }
    }
    Err(TargetError::configuration(
        "password file has no matching target entry",
    ))
}

pub async fn connect(
    config: &TargetConfig,
    resolved_url: &str,
) -> Result<TargetConnection, TargetError> {
    tokio::time::timeout(
        Duration::from_secs(10),
        connect_initialized(config, resolved_url),
    )
    .await
    .map_err(|_| TargetError {
        stage: CopyStage::Idle,
        kind: FailureKind::Operational,
        sqlstate: None,
        message: "connection/session initialization exceeded the 10-second deadline",
        cause: None,
    })?
}

async fn connect_initialized(
    config: &TargetConfig,
    resolved_url: &str,
) -> Result<TargetConnection, TargetError> {
    let uri = url::Url::parse(resolved_url)
        .map_err(|_| TargetError::configuration("invalid target URL"))?;
    if !matches!(uri.scheme(), "postgres" | "postgresql")
        || uri.host_str().is_none()
        || uri.username().is_empty()
        || uri.path().trim_matches('/').is_empty()
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err(TargetError::configuration(
            "target requires a network PostgreSQL URL with explicit identity and no URL options",
        ));
    }
    let mut pg: Config = resolved_url
        .parse()
        .map_err(|_| TargetError::configuration("invalid target URL"))?;
    let host = match pg.get_hosts() {
        [tokio_postgres::config::Host::Tcp(host)] if !host.is_empty() => host.clone(),
        _ => {
            return Err(TargetError::configuration(
                "target requires exactly one TCP host; Unix sockets and multi-host failover are unsupported",
            ));
        }
    };
    pg.connect_timeout(Duration::from_secs(10));
    if let Some(path) = &config.passfile {
        let password = passfile(path, &pg, &host)?;
        if pg.get_password().is_none() {
            pg.password(password);
        }
    }
    let mut session = config.session.clone();
    for (name, value) in &session {
        if !matches!(
            name.as_str(),
            "application_name"
                | "timezone"
                | "statement_timeout"
                | "lock_timeout"
                | "idle_in_transaction_session_timeout"
                | "work_mem"
                | "maintenance_work_mem"
                | "client_min_messages"
                | "standard_conforming_strings"
                | "datestyle"
        ) {
            return Err(TargetError::configuration(
                "unsupported target session variable",
            ));
        }
        if (name == "standard_conforming_strings" && value != "on")
            || (name == "timezone" && value != "UTC")
            || (name == "datestyle" && value != "ISO, YMD")
        {
            return Err(TargetError::configuration(
                "target decoding session settings must remain deterministic",
            ));
        }
    }
    session.insert("standard_conforming_strings".into(), "on".into());
    session.insert("timezone".into(), "UTC".into());
    session.insert("datestyle".into(), "ISO, YMD".into());
    let (client, task, cancel_tls) = match config.tls_mode {
        TlsMode::Disable => {
            if config.ca_file.is_some()
                || config.client_cert.is_some()
                || config.client_key.is_some()
            {
                return Err(TargetError::configuration("TLS files require verify_full"));
            }
            pg.ssl_mode(SslMode::Disable);
            let (client, connection) = pg
                .connect(NoTls)
                .await
                .map_err(|e| TargetError::database(e, CopyStage::Idle))?;
            (client, tokio::spawn(connection), CancelTls::Plain)
        }
        TlsMode::VerifyFull => {
            let mut tls = native_tls::TlsConnector::builder();
            if let Some(path) = &config.ca_file {
                tls.disable_built_in_roots(true);
                let pem = fs::read(path)
                    .map_err(|_| TargetError::configuration("cannot read target CA file"))?;
                tls.add_root_certificate(
                    native_tls::Certificate::from_pem(&pem)
                        .map_err(|_| TargetError::configuration("invalid target CA certificate"))?,
                );
            }
            match (&config.client_cert, &config.client_key) {
                (Some(cert), Some(key)) => {
                    let cert = fs::read(cert).map_err(|_| {
                        TargetError::configuration("cannot read target client certificate")
                    })?;
                    let key = fs::read(key)
                        .map_err(|_| TargetError::configuration("cannot read target client key"))?;
                    tls.identity(native_tls::Identity::from_pkcs8(&cert, &key).map_err(|_| {
                        TargetError::configuration("invalid PEM client certificate/key")
                    })?);
                }
                (None, None) => (),
                _ => {
                    return Err(TargetError::configuration(
                        "target client certificate/key must be supplied together",
                    ));
                }
            }
            pg.ssl_mode(SslMode::Require);
            let tls = postgres_native_tls::MakeTlsConnector::new(tls.build().map_err(|_| {
                TargetError::configuration("cannot build verified target TLS connector")
            })?);
            let (client, connection) = pg
                .connect(tls.clone())
                .await
                .map_err(|e| TargetError::database(e, CopyStage::Idle))?;
            (client, tokio::spawn(connection), CancelTls::Verified(tls))
        }
    };
    let connection = TargetConnection {
        client,
        task: Some(task),
        stage: StageHandle(Arc::new(AtomicU8::new(0))),
        cancel_tls,
    };
    for (name, value) in session {
        connection
            .client
            .query_one(
                "SELECT pg_catalog.set_config($1,$2,false)",
                &[&name, &value],
            )
            .await
            .map_err(|e| TargetError::database(e, CopyStage::Idle))?;
    }
    Ok(connection)
}

/// Inspect an idle owned connection in one coherent read-only catalog snapshot.
pub async fn inspect(client: &mut Client, schema: &str) -> Result<TargetCatalog, TargetError> {
    inspect_schemas(client, schema, &[]).await
}

/// Inspect all requested namespaces together; the primary summary retains its original meaning.
pub async fn inspect_schemas(
    client: &mut Client,
    schema: &str,
    additional: &[String],
) -> Result<TargetCatalog, TargetError> {
    let transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(|error| TargetError::database(error, CopyStage::Idle))?;
    let catalog = inspect_catalog(&transaction, schema, additional).await?;
    transaction
        .rollback()
        .await
        .map_err(|error| TargetError::database(error, CopyStage::Idle))?;
    Ok(catalog)
}

async fn inspect_catalog(
    client: &(impl GenericClient + Sync),
    schema: &str,
    additional: &[String],
) -> Result<TargetCatalog, TargetError> {
    let mut catalog = inspect_namespace(client, schema, true).await?;
    let requested: BTreeSet<_> = additional
        .iter()
        .filter(|name| name.as_str() != schema)
        .collect();
    for name in requested {
        let mut next = inspect_namespace(client, name, false).await?;
        catalog.tables.append(&mut next.tables);
        catalog.dependencies.append(&mut next.dependencies);
        catalog
            .external_dependencies
            .append(&mut next.external_dependencies);
        catalog
            .type_parts
            .as_mut()
            .expect("inspected types")
            .append(next.type_parts.as_mut().expect("inspected types"));
        catalog
            .namespaces
            .as_mut()
            .expect("inspected namespaces")
            .append(next.namespaces.as_mut().expect("inspected namespaces"));
    }
    catalog
        .tables
        .sort_by(|a, b| (&a.schema, &a.name).cmp(&(&b.schema, &b.name)));
    catalog
        .namespaces
        .as_mut()
        .expect("inspected namespaces")
        .sort_by(|a, b| a.schema.cmp(&b.schema));
    catalog.external_dependencies.sort();
    catalog.external_dependencies.dedup();
    let relations: Vec<_> = catalog
        .tables
        .iter()
        .filter_map(|table| table.observed.as_ref().map(|table| table.oid))
        .collect();
    let types: Vec<_> = catalog
        .type_parts
        .as_ref()
        .expect("inspected types")
        .iter()
        .filter_map(|part| part.type_oid)
        .collect();
    catalog.dependency_graph =
        Some(catalog_graph::inspect_dependency_graph(client, &relations, &types).await?);
    Ok(catalog)
}

async fn inspect_namespace(
    client: &(impl GenericClient + Sync),
    schema: &str,
    global: bool,
) -> Result<TargetCatalog, TargetError> {
    let decode_error = |error| TargetError::database(error, CopyStage::Idle);
    let row=client.query_one("SELECT current_setting('server_version_num')::integer,current_setting('server_version'),EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname=$1),has_database_privilege(current_database(),'CREATE'),COALESCE((SELECT has_schema_privilege(oid,'USAGE') FROM pg_catalog.pg_namespace WHERE nspname=$1),false),COALESCE((SELECT has_schema_privilege(oid,'CREATE') FROM pg_catalog.pg_namespace WHERE nspname=$1),false),(SELECT oid FROM pg_catalog.pg_namespace WHERE nspname=$1),current_setting('extra_float_digits')::integer,current_setting('IntervalStyle')",&[&schema]).await.map_err(|e|TargetError::database(e,CopyStage::Idle))?;
    let version: i32 = row.try_get(0).map_err(decode_error)?;
    if !(160000..190000).contains(&version) {
        return Err(TargetError::configuration(
            "supported PostgreSQL lines are 16, 17 and 18",
        ));
    }
    let mut catalog = TargetCatalog {
        server_version: row.try_get(1).map_err(decode_error)?,
        schema_exists: row.try_get(2).map_err(decode_error)?,
        can_create_schema: row.try_get(3).map_err(decode_error)?,
        can_use_schema: row.try_get(4).map_err(decode_error)?,
        can_create_objects: row.try_get(5).map_err(decode_error)?,
        tables: Vec::new(),
        enums: BTreeMap::new(),
        extensions: Vec::new(),
        external_dependencies: Vec::new(),
        dependencies: Vec::new(),
        occupied_names: BTreeMap::new(),
        occupied_types: Default::default(),
        event_triggers: if global {
            Some(observed::inspect_triggers(client, schema, None).await?)
        } else {
            None
        },
        type_parts: None,
        dependency_graph: None,
        namespaces: Some(vec![TargetNamespaceObserved {
            schema: schema.into(),
            oid: row.try_get(6).map_err(decode_error)?,
            schema_exists: row.try_get(2).map_err(decode_error)?,
            can_use: row.try_get(4).map_err(decode_error)?,
            can_create_objects: row.try_get(5).map_err(decode_error)?,
            occupied_names: BTreeMap::new(),
            occupied_types: BTreeSet::new(),
        }]),
        deparse: Some(TargetDeparseObserved {
            extra_float_digits: row.try_get(7).map_err(decode_error)?,
            interval_style: row.try_get(8).map_err(decode_error)?,
        }),
    };
    let namespace = &catalog.namespaces.as_ref().expect("inspected namespace")[0];
    if namespace.oid == Some(0) || namespace.schema_exists != namespace.oid.is_some() {
        return Err(TargetError::configuration(
            "contradictory target namespace observation",
        ));
    }
    for row in client.query("SELECT c.relname,c.relkind::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 ORDER BY c.relname COLLATE \"C\"", &[&schema]).await.map_err(|e| TargetError::database(e, CopyStage::Idle))? {
        catalog.occupied_names.insert(row.try_get(0).map_err(decode_error)?, row.try_get(1).map_err(decode_error)?);
    }
    for row in client.query("SELECT t.typname FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname=$1 ORDER BY t.typname COLLATE \"C\"", &[&schema]).await.map_err(|e| TargetError::database(e, CopyStage::Idle))? {
        catalog.occupied_types.insert(row.try_get(0).map_err(decode_error)?);
    }
    for row in client
        .query(include_str!("table_dependencies.sql"), &[&schema])
        .await
        .map_err(|e| TargetError::database(e, CopyStage::Idle))?
    {
        catalog.dependencies.push(TargetDependency {
            kind: row.try_get(0).map_err(decode_error)?,
            name: row.try_get(1).map_err(decode_error)?,
            dependent_schema: row.try_get(2).map_err(decode_error)?,
            dependent_table: row.try_get(3).map_err(decode_error)?,
            referenced_schema: row.try_get(4).map_err(decode_error)?,
            referenced_table: row.try_get(5).map_err(decode_error)?,
        });
    }
    catalog.dependencies.sort_by(|a, b| {
        (
            &a.kind,
            &a.dependent_schema,
            &a.dependent_table,
            &a.name,
            &a.referenced_schema,
            &a.referenced_table,
        )
            .cmp(&(
                &b.kind,
                &b.dependent_schema,
                &b.dependent_table,
                &b.name,
                &b.referenced_schema,
                &b.referenced_table,
            ))
    });
    for row in client
        .query(include_str!("tables.sql"), &[&schema])
        .await
        .map_err(|e| TargetError::database(e, CopyStage::Idle))?
    {
        let name: String = row.try_get(0).map_err(decode_error)?;
        let observed = observed::inspect(client, schema, &name).await?;
        let mut table = TargetTable {
            schema: schema.into(),
            name: name.clone(),
            columns: Vec::new(),
            row_count: None,
            ordinary_standalone: row.try_get(6).map_err(decode_error)?,
            can_insert: row.try_get(1).map_err(decode_error)?,
            can_select: row.try_get(2).map_err(decode_error)?,
            can_alter: row.try_get(3).map_err(decode_error)?,
            can_truncate: row.try_get(4).map_err(decode_error)?,
            can_lock: Some(row.try_get("can_lock").map_err(decode_error)?),
            row_security_active: row.try_get(5).map_err(decode_error)?,
            observed: None,
        };
        for column in &observed.columns {
            table.columns.push(TargetColumn {
                name: column.name.clone(),
                data_type: column.data_type.clone(),
                nullable: column.nullable,
                generated: !column.generated_kind.is_empty(),
                identity: !column.identity_mode.is_empty(),
            });
        }
        table.observed = Some(observed);
        catalog.tables.push(table);
    }
    let type_parts = catalog_graph::inspect_types(client, schema).await?;
    catalog.type_parts = Some(type_parts);
    for row in client
        .query(include_str!("enums.sql"), &[&schema])
        .await
        .map_err(|e| TargetError::database(e, CopyStage::Idle))?
    {
        catalog
            .enums
            .entry(row.try_get(0).map_err(decode_error)?)
            .or_default()
            .push(row.try_get(1).map_err(decode_error)?);
    }
    if global {
        for row in client
            .query(
                "SELECT extname FROM pg_catalog.pg_extension ORDER BY extname",
                &[],
            )
            .await
            .map_err(|e| TargetError::database(e, CopyStage::Idle))?
        {
            catalog
                .extensions
                .push(row.try_get(0).map_err(decode_error)?);
        }
    }
    for row in client
        .query(include_str!("dependencies.sql"), &[&schema])
        .await
        .map_err(|e| TargetError::database(e, CopyStage::Idle))?
    {
        catalog
            .external_dependencies
            .push(row.try_get(0).map_err(decode_error)?);
    }
    let namespace = &mut catalog.namespaces.as_mut().expect("inspected namespace")[0];
    namespace.occupied_names = catalog.occupied_names.clone();
    namespace.occupied_types = catalog.occupied_types.clone();
    Ok(catalog)
}

pub fn copy_sql(table: &TablePlan) -> Result<String, TargetError> {
    let columns = table
        .columns
        .iter()
        .filter(|column| column.copy)
        .map(|column| quote_ident(&column.target_name))
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return Err(TargetError::configuration(
            "COPY requires selected destination columns",
        ));
    }
    Ok(format!(
        "COPY {} ({}) FROM STDIN WITH (FORMAT text)",
        qualified(&table.target_schema, &table.target_name),
        columns.join(",")
    ))
}

pub async fn execute_ddl(client: &Client, sql: &str) -> Result<(), TargetError> {
    client
        .batch_execute(sql)
        .await
        .map_err(|e| TargetError::database(e, CopyStage::Idle))
}

pub async fn copy_bytes(
    conn: &mut TargetConnection,
    table: &TablePlan,
    bytes: Bytes,
    expected_rows: u64,
) -> Result<u64, TargetError> {
    if conn.stage.canceled() {
        return Err(TargetError::configuration(
            "target connection has been canceled",
        ));
    }
    let sql = copy_sql(table)?;
    let stage = conn.stage.clone();
    stage.set(CopyStage::Begin);
    let tx = conn
        .client
        .transaction()
        .await
        .map_err(|e| TargetError::database(e, CopyStage::Begin))?;
    let copied = async {
        stage.set(CopyStage::CopyInit);
        let sink = tx
            .copy_in(&sql)
            .await
            .map_err(|e| TargetError::database(e, CopyStage::CopyInit))?;
        tokio::pin!(sink);
        stage.set(CopyStage::CopyData);
        sink.send(bytes)
            .await
            .map_err(|e| TargetError::database(e, CopyStage::CopyData))?;
        stage.set(CopyStage::CopyFinish);
        sink.finish()
            .await
            .map_err(|e| TargetError::database(e, CopyStage::CopyFinish))
    }
    .await;
    let rows = match copied {
        Ok(rows) if rows == expected_rows => rows,
        result => {
            let failure = result.err().unwrap_or(TargetError {
                stage: CopyStage::CopyFinish,
                kind: FailureKind::Operational,
                sqlstate: None,
                message: "COPY row count differs from retained batch",
                cause: None,
            });
            stage.set(CopyStage::Rollback);
            if let Err(error) = tx.rollback().await {
                let mut rollback = TargetError::database(error, CopyStage::Rollback);
                rollback.cause = Some(Box::new(failure));
                conn.task.as_ref().expect("live target").abort();
                return Err(rollback);
            }
            stage.set(CopyStage::Idle);
            return Err(failure);
        }
    };
    if !stage.enter_commit() {
        stage.set(CopyStage::Rollback);
        if let Err(error) = tx.rollback().await {
            conn.task.as_ref().expect("live target").abort();
            return Err(TargetError::database(error, CopyStage::Rollback));
        }
        stage.set(CopyStage::Idle);
        return Err(TargetError {
            stage: CopyStage::CopyFinish,
            kind: FailureKind::Operational,
            sqlstate: None,
            message: "batch canceled before COMMIT",
            cause: None,
        });
    }
    if let Err(error) = tx.commit().await {
        let failure = TargetError::database(error, CopyStage::CommitAttempt);
        if failure.kind != FailureKind::Indeterminate {
            stage.set(CopyStage::Rollback);
            if let Err(error) = conn.client.batch_execute("ROLLBACK").await {
                let mut rollback = TargetError::database(error, CopyStage::Rollback);
                rollback.cause = Some(Box::new(failure));
                conn.task.as_ref().expect("live target").abort();
                return Err(rollback);
            }
            stage.set(CopyStage::Idle);
        }
        return Err(failure);
    }
    stage.set(CopyStage::Complete);
    Ok(rows)
}

pub async fn copy_batch(
    conn: &mut TargetConnection,
    table: &TablePlan,
    batch: &EncodedBatch,
) -> Result<u64, TargetError> {
    let bytes = if let (Some(first), Some(last)) = (batch.rows.first(), batch.rows.last()) {
        if first.start > last.end
            || last.end > batch.storage.bytes.len()
            || batch
                .rows
                .windows(2)
                .any(|rows| rows[0].end != rows[1].start)
            || batch.rows.iter().any(|row| row.start > row.end)
        {
            return Err(TargetError::configuration(
                "invalid retained COPY row offsets",
            ));
        }
        batch.storage.bytes.slice(first.start..last.end)
    } else {
        Bytes::new()
    };
    copy_bytes(conn, table, bytes, batch.rows.len() as u64).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires owned native PostgreSQL TLS fixture"]
    async fn native_catalog_union_uses_one_read_only_repeatable_snapshot() {
        let required = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| panic!("required native fixture {name}"))
        };
        let schema = format!("t11_snapshot_{}", std::process::id());
        let other = format!("{schema}_other");
        let config: TargetConfig = serde_json::from_value(serde_json::json!({
            "url_env":"MY2PG_POSTGRES_URL", "schema":schema, "ca_file":required("MY2PG_TLS_CA")
        }))
        .unwrap();
        let mut reader = connect(&config, &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap();
        let admin = connect(&config, &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap();
        admin
            .client
            .batch_execute(&format!(
                "CREATE SCHEMA {};CREATE SCHEMA {};CREATE TABLE {}(id integer)",
                quote_ident(&schema),
                quote_ident(&other),
                qualified(&schema, "original")
            ))
            .await
            .unwrap();
        let transaction = reader
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await
            .unwrap();
        let count: i64 = transaction
            .query_one(
                "SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname=$1 OR nspname=$2",
                &[&schema, &other],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 2, "establish the owned catalog snapshot");
        admin
            .client
            .batch_execute(&format!(
                "CREATE TABLE {}(id integer);CREATE TYPE {} AS ENUM ('after')",
                qualified(&other, "late"),
                qualified(&other, "late_type")
            ))
            .await
            .unwrap();
        let catalog = inspect_catalog(&transaction, &schema, std::slice::from_ref(&other))
            .await
            .unwrap();
        assert_eq!(catalog.tables.len(), 1);
        let secondary = catalog
            .namespaces
            .as_ref()
            .unwrap()
            .iter()
            .find(|n| n.schema == other)
            .unwrap();
        assert!(
            secondary.schema_exists
                && secondary.occupied_names.is_empty()
                && secondary.occupied_types.is_empty(),
            "later namespace queries must retain the original snapshot"
        );
        assert!(
            !catalog
                .type_parts
                .as_ref()
                .unwrap()
                .iter()
                .any(|p| p.type_name.as_deref() == Some("late_type"))
        );
        let error = transaction
            .batch_execute(&format!(
                "CREATE TABLE {}(id integer)",
                qualified(&other, "forbidden")
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code().unwrap().code(), "25006");
        transaction.rollback().await.unwrap();
        let current = inspect_schemas(&mut reader.client, &schema, std::slice::from_ref(&other))
            .await
            .unwrap();
        assert_eq!(
            current.tables.len(),
            2,
            "a new inspection sees the committed catalog change"
        );
        assert!(
            current
                .type_parts
                .as_ref()
                .unwrap()
                .iter()
                .any(|p| p.type_name.as_deref() == Some("late_type"))
        );
        admin
            .client
            .batch_execute(&format!(
                "DROP SCHEMA {} CASCADE;DROP SCHEMA {} CASCADE",
                quote_ident(&other),
                quote_ident(&schema)
            ))
            .await
            .unwrap();
        reader.close().await.unwrap();
        admin.close().await.unwrap();
    }
    #[test]
    fn identifiers_and_passfile_escaping_are_exact() {
        assert_eq!(
            qualified("odd\"schema", "users"),
            "\"odd\"\"schema\".\"users\""
        );
        assert_eq!(
            passfields(r"host:5432:db:user:pa\:ss\\word").unwrap()[4],
            "pa:ss\\word"
        );
        assert!(passfields("bad:file").is_none());
    }
    #[test]
    fn cancellation_and_commit_have_an_atomic_order() {
        let before = StageHandle(Arc::new(AtomicU8::new(CopyStage::CopyFinish as u8)));
        before.request_cancel();
        assert!(!before.enter_commit());
        assert_eq!(before.get(), CopyStage::CopyFinish);
        let after = StageHandle(Arc::new(AtomicU8::new(CopyStage::CopyFinish as u8)));
        assert!(after.enter_commit());
        after.request_cancel();
        assert_eq!(after.get(), CopyStage::CommitAttempt);
    }
}
