//! Cancellation must terminate the actual MySQL worker blocked on external MDL.
use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{
    env,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
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
    creds: ResolvedCredentials,
    target: postgres::TargetConnection,
    source: mysql::SourceConnection,
    table: String,
    app: String,
}

impl Case {
    async fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let schema = format!(
            "t13mdl_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let app = format!("{schema}_run");
        let table = format!("{schema}_source");
        let config: MigrationConfig = serde_json::from_value(serde_json::json!({
            "version":1,
            "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},
            "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":required("MY2PG_TLS_CA"),"session":{"application_name":app}},
            "migration":{"table_workers":1,"index_workers":1,"queue_batches":1,"batch_rows":1,"batch_bytes":1024,"max_row_bytes":1024,"memory_bytes":1048576,"reset_sequences":false},
            "tables":{"include":[table]},
            "report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&schema),"progress":"never"}
        }))
        .unwrap();
        let creds = resolve_credentials(&config).unwrap();
        let mut target_admin = config.target.clone();
        target_admin
            .session
            .insert("application_name".into(), format!("{schema}_admin"));
        let target = postgres::connect(&target_admin, creds.target.expose())
            .await
            .unwrap();
        let mut source =
            mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1_048_576)
                .await
                .unwrap();
        source
            .query_drop(format!(
                "CREATE TABLE {} (id INT NOT NULL PRIMARY KEY, value TEXT NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
                mysql::quote_ident(&table)
            ))
            .await
            .unwrap();
        source
            .query_drop(format!(
                "INSERT INTO {} VALUES (1, 'before-cancel'), (2, 'also-before-cancel')",
                mysql::quote_ident(&table)
            ))
            .await
            .unwrap();
        Self {
            config,
            creds,
            target,
            source,
            table,
            app,
        }
    }

    fn console(&self) -> Console {
        Console::new(
            &self.config,
            &Args::try_parse_from(["my2pg", "run", "native.toml", "--quiet"]).unwrap(),
        )
    }

    async fn wait_target_ddl_gate(&self, app: &str) -> Result<(), String> {
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
                        &[&app],
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
        .map_err(|_| "migration never reached its real target CREATE TABLE barrier".to_owned())??;
        Ok(())
    }

    async fn cleanup(
        mut self,
        mut lock_holder: mysql::SourceConnection,
        gate: i64,
        gate_name: &str,
    ) -> Result<(), String> {
        // Always release the fixture-owned MDL before dropping the table.
        let mut failures = Vec::new();
        if let Err(error) = lock_holder.query_drop("UNLOCK TABLES").await {
            failures.push(format!("release source table lock: {error}"));
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
                postgres::quote_ident(&self.config.target.schema)
            ))
            .await
        {
            failures.push(format!("remove target fixture schema: {error}"));
        }
        if let Err(error) = self
            .source
            .query_drop(format!(
                "DROP TABLE IF EXISTS {}",
                mysql::quote_ident(&self.table)
            ))
            .await
        {
            failures.push(format!("remove MySQL source fixture table: {error}"));
        }
        if let Err(error) = self.source.disconnect().await {
            failures.push(format!("close fixture source connection: {error}"));
        }
        if let Err(error) = lock_holder.disconnect().await {
            failures.push(format!("close metadata-lock holder: {error}"));
        }
        if let Err(error) = self.target.close().await {
            failures.push(format!("close fixture target connection: {error}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}

async fn process_row(
    observer: &mut mysql::SourceConnection,
    id: u64,
) -> Result<Option<u64>, String> {
    observer
        .exec_first(
            "SELECT ID FROM information_schema.processlist WHERE ID=?",
            (id,),
        )
        .await
        .map_err(|error| error.to_string())
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PostgreSQL16 verified TLS fixture"]
async fn cancellation_kills_reader_waiting_on_external_mysql_metadata_lock() {
    let case = Case::new().await;
    let gate = 81_370_000i64 + i64::from(std::process::id());
    let gate_name = format!("{}_gate", case.config.target.schema);
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
    let source_user = mysql_async::Opts::from_url(&required("MY2PG_MYSQL_URL"))
        .unwrap()
        .user()
        .unwrap()
        .to_owned();
    let table_ident = mysql::quote_ident(&case.table);
    let (sender, receiver) = watch::channel(false);
    let mut console = case.console();

    let mut run = Box::pin(crate::test_pipeline::run(
        &case.config,
        &case.creds,
        &mut console,
        receiver,
    ));
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

            let id = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let Some((Some(state), info)) = observer
                        .exec_first::<(Option<String>, Option<String>), _, _>(
                            "SELECT STATE, INFO FROM information_schema.processlist WHERE USER=? AND DB=DATABASE() AND INFO LIKE ? AND ID<>?",
                            (&source_user, format!("%{}%", case.table), lock_holder_id),
                        )
                        .await
                        .map_err(|error| error.to_string())?
                        && state.to_ascii_lowercase().contains("metadata lock")
                    {
                        if !info
                            .unwrap_or_default()
                            .to_ascii_uppercase()
                            .starts_with("SELECT")
                        {
                            return Err(
                                "the metadata-lock wait was not the migration table SELECT"
                                    .into(),
                            );
                        }
                        let id: Option<u64> = observer
                            .exec_first(
                                "SELECT ID FROM information_schema.processlist WHERE USER=? AND DB=DATABASE() AND INFO LIKE ? AND STATE LIKE '%metadata lock%' AND ID<>?",
                                (&source_user, format!("%{}%", case.table), lock_holder_id),
                            )
                            .await
                            .map_err(|error| error.to_string())?;
                        if let Some(id) = id {
                            return Ok::<u64, String>(id);
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .map_err(|_| "migration SELECT never appeared waiting on the fixture-owned metadata lock".to_owned())??;

            sender
                .send(true)
                .map_err(|_| "migration cancellation receiver disappeared".to_owned())?;
            tokio::time::timeout(Duration::from_secs(12), async {
                loop {
                    if process_row(&mut observer, id).await?.is_none() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                if process_row(&mut observer, lock_holder_id)
                    .await?
                    .is_none()
                {
                    return Err("metadata-lock owner disappeared before the test released it".into());
                }
                Ok::<(), String>(())
            })
            .await
            .map_err(|_| "KILL CONNECTION did not remove the blocked worker within the shutdown bound".to_owned())??;
            Ok::<u64, String>(id)
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
        tokio::time::timeout(Duration::from_secs(20), &mut run),
        observe
    );
    drop(run);
    // Teardown has to release both fixture-owned blockers even if an oracle failed.
    let report = run_result
        .map_err(|_| "migration did not finish within 20 seconds".to_owned())
        .and_then(|result| result.map_err(|fault| format!("migration runner failed: {fault:?}")));
    let report_validation = report.as_ref().map_err(Clone::clone).and_then(|report| {
        if report.exit_code() != 130 {
            return Err(format!("expected cancelled exit 130, got {report:?}"));
        }
        if !report.failed_steps.is_empty() {
            return Err(format!(
                "cancelled migration reported failed steps: {report:?}"
            ));
        }
        if !report.tables.iter().all(|table| {
            table.rows_read == 0
                && table.committed_rows == 0
                && table.indeterminate_rows == 0
                && table.unresolved_rows == 0
        }) {
            return Err(format!(
                "blocked source table was unexpectedly acknowledged: {report:?}"
            ));
        }
        let bytes = std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json"))
            .map_err(|error| format!("could not read persisted report: {error}"))?;
        let disk: RunReport = serde_json::from_slice(&bytes)
            .map_err(|error| format!("could not parse persisted report: {error}"))?;
        if disk.exit_code() != 130 {
            return Err(format!(
                "persisted report differs from cancellation: {disk:?}"
            ));
        }
        Ok(())
    });

    let observation = observation.map_err(|error| error.to_string());
    let cleanup = case.cleanup(lock_holder, gate, &gate_name).await;
    observer.disconnect().await.unwrap();
    cleanup.unwrap();
    report_validation.unwrap();
    report.unwrap();
    observation.unwrap();
}
