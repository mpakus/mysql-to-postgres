//! Characterizes server-side SingleSnapshot query lifetime under external MDL.
use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{
    env,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required native fixture variable {name}"))
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

struct Case {
    config: MigrationConfig,
    target: postgres::TargetConnection,
    source_admin: mysql::SourceConnection,
    schema: String,
    table: String,
    app: String,
}

impl Case {
    async fn new() -> Self {
        assert_eq!(required("MY2PG_INTEGRATION"), "1");
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let schema = format!(
            "t13snapmdl_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let table = format!("{schema}_source");
        let app = format!("{schema}_run");
        let config: MigrationConfig = serde_json::from_value(serde_json::json!({
            "version": 1,
            "source": {
                "url_env": "MY2PG_MYSQL_URL",
                "ca_file": required("MY2PG_TLS_CA"),
                "consistency": "single_snapshot"
            },
            "target": {
                "url_env": "MY2PG_POSTGRES_URL",
                "ca_file": required("MY2PG_TLS_CA"),
                "schema": schema,
                "session": {"application_name": app}
            },
            "migration": {
                "table_workers": 1,
                "index_workers": 1,
                "queue_batches": 1,
                "batch_rows": 1,
                "batch_bytes": 1024,
                "max_row_bytes": 1024,
                "memory_bytes": 1048576,
                "reset_sequences": false
            },
            "tables": {"include": [table]},
            "report": {
                "directory": PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&schema),
                "progress": "never"
            }
        }))
        .unwrap();
        let credentials = resolve_credentials(&config).unwrap();
        let mut target_config = config.target.clone();
        target_config
            .session
            .insert("application_name".into(), format!("{schema}_admin"));
        let target = postgres::connect(&target_config, credentials.target.expose())
            .await
            .unwrap();
        let mut source_admin =
            mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1_048_576)
                .await
                .unwrap();
        source_admin
            .query_drop(format!(
                "CREATE TABLE {} (id INT NOT NULL PRIMARY KEY, value TEXT NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
                mysql::quote_ident(&table)
            ))
            .await
            .unwrap();
        source_admin
            .query_drop(format!(
                "INSERT INTO {} VALUES (1, 'first'), (2, 'second')",
                mysql::quote_ident(&table)
            ))
            .await
            .unwrap();
        Self {
            config,
            target,
            source_admin,
            schema,
            table,
            app,
        }
    }

    async fn wait_target_ddl_gate(&self, gate_app: &str) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                self.target
                    .client
                    .batch_execute("SELECT pg_stat_clear_snapshot()")
                    .await
                    .map_err(|error| error.to_string())?;
                let count: i64 = self
                    .target
                    .client
                    .query_one(
                        "SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND wait_event='advisory' AND query LIKE 'CREATE TABLE %'",
                        &[&gate_app],
                    )
                    .await
                    .map_err(|error| error.to_string())?
                    .get(0);
                if count == 1 {
                    return Ok::<(), String>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "SingleSnapshot did not reach the owned target DDL gate".to_owned())??;
        Ok(())
    }

    async fn cleanup(
        mut self,
        mut lock_holder: mysql::SourceConnection,
        gate: i64,
        gate_name: &str,
    ) -> Result<(), String> {
        let mut failures = Vec::new();
        if let Err(error) = lock_holder.query_drop("UNLOCK TABLES").await {
            failures.push(format!("release fixture source lock: {error}"));
        }
        if let Err(error) = self
            .target
            .client
            .batch_execute(&format!(
                "SELECT pg_advisory_unlock({gate}); DROP EVENT TRIGGER IF EXISTS {}; DROP FUNCTION IF EXISTS public.{}()",
                postgres::quote_ident(gate_name),
                postgres::quote_ident(gate_name)
            ))
            .await
        {
            failures.push(format!("remove target event gate: {error}"));
        }
        if let Err(error) = self
            .target
            .client
            .batch_execute(&format!(
                "DROP SCHEMA IF EXISTS {} CASCADE",
                postgres::quote_ident(&self.schema)
            ))
            .await
        {
            failures.push(format!("remove target fixture schema: {error}"));
        }
        if let Err(error) = self
            .source_admin
            .query_drop(format!(
                "DROP TABLE IF EXISTS {}",
                mysql::quote_ident(&self.table)
            ))
            .await
        {
            failures.push(format!("remove source fixture table: {error}"));
        }
        if let Err(error) = self.source_admin.disconnect().await {
            failures.push(format!("close source fixture admin: {error}"));
        }
        if let Err(error) = lock_holder.disconnect().await {
            failures.push(format!("close external lock holder: {error}"));
        }
        if let Err(error) = self.target.close().await {
            failures.push(format!("close target fixture connection: {error}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}

#[derive(Debug)]
struct BlockedReader {
    id: u64,
    state: Option<String>,
    query: String,
}

async fn blocked_reader(
    observer: &mut mysql::SourceConnection,
    user: &str,
    table: &str,
    lock_holder_id: u64,
    observer_id: u64,
) -> Result<Option<BlockedReader>, String> {
    observer
        .exec_first::<(u64, Option<String>, Option<String>), _, _>(
            "SELECT ID, STATE, INFO FROM information_schema.processlist WHERE USER=? AND DB=DATABASE() AND INFO LIKE ? AND ID NOT IN (?,?)",
            (user, format!("%{table}%"), lock_holder_id, observer_id),
        )
        .await
        .map_err(|error| error.to_string())
        .map(|row| {
            row.map(|(id, state, query)| BlockedReader {
                id,
                state,
                query: query.unwrap_or_default(),
            })
        })
}

async fn process_state(
    observer: &mut mysql::SourceConnection,
    id: u64,
) -> Result<Option<String>, String> {
    observer
        .exec_first(
            "SELECT STATE FROM information_schema.processlist WHERE ID=?",
            (id,),
        )
        .await
        .map_err(|error| error.to_string())
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PostgreSQL16 verified TLS fixture"]
async fn cancellation_terminates_single_snapshot_select_waiting_on_external_mdl() {
    let case = Case::new().await;
    let gate = 81_380_000i64 + i64::from(std::process::id());
    let gate_name = format!("{}_gate", case.schema);
    let gate_function = format!("public.{}", postgres::quote_ident(&gate_name));
    case.target
        .client
        .batch_execute(&format!(
            "CREATE FUNCTION {gate_function}() RETURNS event_trigger LANGUAGE plpgsql AS $$BEGIN IF current_setting('application_name')={} AND TG_TAG='CREATE TABLE' THEN PERFORM pg_advisory_xact_lock({gate}); END IF; END$$; CREATE EVENT TRIGGER {} ON ddl_command_start EXECUTE FUNCTION {gate_function}(); SELECT pg_advisory_lock({gate})",
            literal(&case.app),
            postgres::quote_ident(&gate_name)
        ))
        .await
        .unwrap();

    let root_url = required("MY2PG_MYSQL_ROOT_URL");
    let mut lock_holder = mysql::connect(&case.config.source, &root_url, 1_048_576)
        .await
        .unwrap();
    let lock_holder_id = u64::from(lock_holder.id());
    let mut observer = mysql::connect(&case.config.source, &root_url, 1_048_576)
        .await
        .unwrap();
    let observer_id = u64::from(observer.id());
    let user = mysql_async::Opts::from_url(&required("MY2PG_MYSQL_URL"))
        .unwrap()
        .user()
        .unwrap()
        .to_owned();
    let table_ident = mysql::quote_ident(&case.table);
    let (sender, receiver) = watch::channel(false);
    let run_config = case.config.clone();
    let run_credentials = resolve_credentials(&case.config).unwrap();
    let worker_sender = sender.clone();
    let run_finished = Arc::new(AtomicBool::new(false));
    let task_finished = run_finished.clone();
    let mut running = tokio::spawn(async move {
        let args = Args::try_parse_from(["my2pg", "run", "native.toml", "--quiet"]).unwrap();
        let mut console = Console::new(&run_config, &args);
        let result =
            crate::test_pipeline::run(&run_config, &run_credentials, &mut console, receiver).await;
        task_finished.store(true, Ordering::Release);
        drop(worker_sender);
        result
    });

    let observe = async {
        let observation = async {
            case.wait_target_ddl_gate(&case.app).await?;
            lock_holder
                .query_drop(format!("LOCK TABLES {table_ident} WRITE"))
                .await
                .map_err(|error| error.to_string())?;
            case.target
                .client
                .batch_execute(&format!("SELECT pg_advisory_unlock({gate})"))
                .await
                .map_err(|error| error.to_string())?;

            let reader = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let Some(reader) = blocked_reader(
                        &mut observer,
                        &user,
                        &case.table,
                        lock_holder_id,
                        observer_id,
                    )
                    .await?
                        && reader.state.as_deref().is_some_and(|state| {
                            state.to_ascii_lowercase().contains("metadata lock")
                        })
                    {
                        if !reader.query.to_ascii_uppercase().starts_with("SELECT") {
                            return Err(
                                "blocked source command was not the real table SELECT".into(),
                            );
                        }
                        return Ok::<BlockedReader, String>(reader);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| "original source SELECT never waited on fixture MDL".to_owned())??;

            // The migration must terminate its exact blocked server session,
            // even though the fixture continues to hold the external MDL.
            sender
                .send(true)
                .map_err(|_| "migration cancellation receiver disappeared".to_owned())?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let reader_state = process_state(&mut observer, reader.id).await?;
                    let holder_state = process_state(&mut observer, lock_holder_id).await?;
                    if holder_state.is_none() {
                        return Err("fixture-owned external MDL holder disappeared before release".into());
                    }
                    if reader_state.is_none() && run_finished.load(Ordering::Acquire) {
                        return Ok::<(), String>(());
                    }
                    if run_finished.load(Ordering::Acquire) && reader_state.is_some() {
                        return Err(format!(
                            "SingleSnapshot run returned before its blocked source backend exited: {reader_state:?}"
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| "SingleSnapshot cancellation did not terminate its blocked backend while fixture MDL remained held".to_owned())??;
            lock_holder
                .query_drop("UNLOCK TABLES")
                .await
                .map_err(|error| format!("release fixture MDL: {error}"))?;
            Ok::<BlockedReader, String>(reader)
        }
        .await;
        if observation.is_err() {
            let _ = sender.send(true);
            let _ = case
                .target
                .client
                .batch_execute(&format!("SELECT pg_advisory_unlock({gate})"))
                .await;
            let _ = lock_holder.query_drop("UNLOCK TABLES").await;
        }
        observation
    };

    let (run_result, observation) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(20), &mut running),
        observe
    );
    if run_result.is_err() {
        running.abort();
        let _ = tokio::time::timeout(Duration::from_secs(5), &mut running).await;
    }
    let report = run_result
        .map_err(|_| "SingleSnapshot run did not finish after releasing the lock".to_owned())
        .and_then(|result| result.map_err(|error| format!("runner task failed: {error}")))
        .and_then(|result| result.map_err(|error| format!("migration failed: {error:?}")));
    let report_validation = report.as_ref().map_err(Clone::clone).and_then(|report| {
        if report.exit_code() != 130 {
            return Err(format!("expected cancellation exit 130: {report:?}"));
        }
        if !report.tables.iter().all(|table| {
            table.rows_read == 0
                && table.committed_rows == 0
                && table.indeterminate_rows == 0
                && table.unresolved_rows == 0
        }) {
            return Err(format!("cancellation acknowledged source rows: {report:?}"));
        }
        if !report.failed_steps.is_empty() {
            return Err(format!("cancelled run recorded failed steps: {report:?}"));
        }
        let disk: RunReport = serde_json::from_slice(
            &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json"))
                .map_err(|error| format!("read persisted report: {error}"))?,
        )
        .map_err(|error| format!("parse persisted report: {error}"))?;
        if disk.exit_code() != 130 {
            return Err(format!(
                "persisted report did not preserve cancellation: {disk:?}"
            ));
        }
        if disk.status != report.status
            || disk.failed_steps != report.failed_steps
            || disk.tables.len() != report.tables.len()
            || disk
                .tables
                .iter()
                .zip(&report.tables)
                .any(|(persisted, memory)| {
                    persisted.id != memory.id
                        || persisted.status != memory.status
                        || persisted.rows_read != memory.rows_read
                        || persisted.committed_rows != memory.committed_rows
                        || persisted.committed_bytes != memory.committed_bytes
                        || persisted.indeterminate_rows != memory.indeterminate_rows
                        || persisted.unresolved_rows != memory.unresolved_rows
                        || persisted.rejected_rows != memory.rejected_rows
                })
        {
            return Err(
                "persisted cancellation accounting differs from the returned report".into(),
            );
        }
        Ok(())
    });
    let observation = observation.map_err(|error| error.to_string());
    let cleanup = case.cleanup(lock_holder, gate, &gate_name).await;
    observer.disconnect().await.unwrap();
    cleanup.unwrap();
    report_validation
        .unwrap_or_else(|error| panic!("{error}; fixture observation: {observation:?}"));
    let blocked = observation.unwrap();
    assert!(
        blocked
            .state
            .as_deref()
            .is_some_and(|state| { state.to_ascii_lowercase().contains("metadata lock") })
            && blocked.query.to_ascii_uppercase().starts_with("SELECT"),
        "expected the real source SELECT to be blocked before cancellation: {blocked:?}"
    );
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PostgreSQL16 verified TLS fixture"]
async fn cancellation_terminates_single_snapshot_reader_blocked_during_preflight() {
    let mut case = Case::new().await;
    case.source_admin
        .query_drop(format!(
            "ALTER TABLE {} ADD COLUMN memo VARCHAR(40) NOT NULL DEFAULT 'seed'",
            mysql::quote_ident(&case.table)
        ))
        .await
        .unwrap();
    let mut lock_holder = mysql::connect(
        &case.config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        1_048_576,
    )
    .await
    .unwrap();
    let lock_holder_id = u64::from(lock_holder.id());
    lock_holder
        .query_drop(format!(
            "LOCK TABLES {} WRITE",
            mysql::quote_ident(&case.table)
        ))
        .await
        .unwrap();
    let mut observer = mysql::connect(
        &case.config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        1_048_576,
    )
    .await
    .unwrap();
    let observer_id = u64::from(observer.id());
    let user = mysql_async::Opts::from_url(&required("MY2PG_MYSQL_URL"))
        .unwrap()
        .user()
        .unwrap()
        .to_owned();
    let (sender, receiver) = watch::channel(false);
    let run_config = case.config.clone();
    let run_credentials = resolve_credentials(&case.config).unwrap();
    let worker_sender = sender.clone();
    let run_finished = Arc::new(AtomicBool::new(false));
    let task_finished = run_finished.clone();
    let mut running = tokio::spawn(async move {
        let args = Args::try_parse_from(["my2pg", "run", "native.toml", "--quiet"]).unwrap();
        let mut console = Console::new(&run_config, &args);
        let result =
            crate::test_pipeline::run(&run_config, &run_credentials, &mut console, receiver).await;
        task_finished.store(true, Ordering::Release);
        drop(worker_sender);
        result
    });

    let observe = async {
        let observation = async {
            let reader = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let Some(reader) = blocked_reader(
                        &mut observer,
                        &user,
                        &case.table,
                        lock_holder_id,
                        observer_id,
                    )
                    .await?
                        && reader.state.as_deref().is_some_and(|state| {
                            state.to_ascii_lowercase().contains("metadata lock")
                        })
                    {
                        if !reader.query.to_ascii_uppercase().starts_with("SELECT") {
                            return Err("preflight source command was not a SELECT".into());
                        }
                        return Ok::<BlockedReader, String>(reader);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| "preflight source query never waited on fixture MDL".to_owned())??;

            sender
                .send(true)
                .map_err(|_| "preflight cancellation receiver disappeared".to_owned())?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let reader_state = process_state(&mut observer, reader.id).await?;
                    if process_state(&mut observer, lock_holder_id)
                        .await?
                        .is_none()
                    {
                        return Err("fixture-owned preflight MDL holder disappeared before release".into());
                    }
                    if reader_state.is_none() && run_finished.load(Ordering::Acquire) {
                        return Ok::<(), String>(());
                    }
                    if run_finished.load(Ordering::Acquire) && reader_state.is_some() {
                        return Err(format!(
                            "preflight returned before its blocked source backend exited: {reader_state:?}"
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| "preflight cancellation did not terminate its backend while fixture MDL remained held".to_owned())??;
            Ok::<BlockedReader, String>(reader)
        }
        .await;
        if observation.is_err() {
            let _ = sender.send(true);
            let _ = lock_holder.query_drop("UNLOCK TABLES").await;
        }
        observation
    };

    let (run_result, observation) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(20), &mut running),
        observe
    );
    if run_result.is_err() {
        running.abort();
        let _ = tokio::time::timeout(Duration::from_secs(5), &mut running).await;
    }
    let preflight_result = run_result
        .map_err(|_| "preflight cancellation did not finish".to_owned())
        .and_then(|result| result.map_err(|error| format!("runner task failed: {error}")));
    let observation = observation.map_err(|error| error.to_string());
    let cleanup = case
        .cleanup(
            lock_holder,
            81_380_000 + i64::from(std::process::id()),
            "preflight_gate",
        )
        .await;
    observer.disconnect().await.unwrap();
    cleanup.unwrap();
    let error = preflight_result.unwrap().unwrap_err();
    assert_eq!(
        error.exit_code(),
        130,
        "expected cancelled preflight: {error}"
    );
    let blocked = observation.unwrap();
    assert!(
        blocked
            .state
            .as_deref()
            .is_some_and(|state| { state.to_ascii_lowercase().contains("metadata lock") })
            && blocked.query.to_ascii_uppercase().starts_with("SELECT"),
        "expected a source SELECT blocked during preflight: {blocked:?}"
    );
}
