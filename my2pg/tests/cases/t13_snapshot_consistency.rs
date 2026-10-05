//! SingleSnapshot must retain one InnoDB read view across later table reads.
use clap::Parser;
use my2pg::{
    cli::Args,
    config::{ExistingPolicy, MigrationConfig, MigrationMode, resolve_credentials},
    mysql, postgres,
    report::Console,
};
use mysql_async::prelude::Queryable;
use std::{env, path::PathBuf, time::Duration};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing owned integration variable {name}"))
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn actual_run_keeps_the_original_source_snapshot_for_later_tables() {
    assert_eq!(required("MY2PG_INTEGRATION"), "1");
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let suffix = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let schema = format!("t13_snapshot_{}_{}", std::process::id(), suffix);
    let first = format!("{schema}_a");
    let later = format!("{schema}_b");
    let app = format!("{schema}_runner");
    let artifact_dir = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&schema);
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
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
        "tables": {"include": [first, later]},
        "report": {"directory": artifact_dir, "console": "json", "progress": "never"}
    }))
    .unwrap();
    let credentials = resolve_credentials(&config).unwrap();
    let mut admin = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 4096)
        .await
        .unwrap();
    let target_admin = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let qualified_first = postgres::qualified(&schema, &first);
    let qualified_later = postgres::qualified(&schema, &later);

    admin
        .query_drop(format!(
            "DROP TABLE IF EXISTS {}",
            mysql::quote_ident(&first)
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!(
            "DROP TABLE IF EXISTS {}",
            mysql::quote_ident(&later)
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!(
            "CREATE TABLE {} (id INT NOT NULL PRIMARY KEY, value VARCHAR(40) NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
            mysql::quote_ident(&first)
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!(
            "CREATE TABLE {} (id INT NOT NULL PRIMARY KEY, value VARCHAR(40) NOT NULL) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
            mysql::quote_ident(&later)
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!(
            "INSERT INTO {} VALUES (1,'first-table'); INSERT INTO {} VALUES (1,'snapshot-value')",
            mysql::quote_ident(&first),
            mysql::quote_ident(&later)
        ))
        .await
        .unwrap();

    // Materialize only the compatible target structures. The data-only run
    // below then exercises the original run's one read-only source snapshot.
    config.migration.mode = MigrationMode::SchemaOnly;
    let (_, schema_cancel) = watch::channel(false);
    let mut schema_console = Console::new(
        &config,
        &Args::try_parse_from(["my2pg", "run", "fixture.toml", "--quiet"]).unwrap(),
    );
    let schema_report =
        crate::test_pipeline::run(&config, &credentials, &mut schema_console, schema_cancel)
            .await
            .unwrap();
    assert_eq!(
        schema_report.exit_code(),
        0,
        "schema setup: {schema_report:?}"
    );
    config.migration.mode = MigrationMode::DataOnly;
    config.target.on_existing = ExistingPolicy::Append;

    let locker = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let lock_key = 81_350_000i64 + i64::from(std::process::id());
    let gate_name = format!("{schema}_row_gate");
    let function = format!("public.{}", postgres::quote_ident(&gate_name));
    let app_literal = format!("'{}'", app.replace('\'', "''"));
    locker
        .client
        .batch_execute(&format!(
            "CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN IF current_setting('application_name')={} THEN \
             PERFORM pg_catalog.pg_advisory_xact_lock({lock_key}); END IF; RETURN NEW; END $$; \
             CREATE TRIGGER {} BEFORE INSERT ON {qualified_first} FOR EACH ROW EXECUTE FUNCTION {function}(); \
             SELECT pg_catalog.pg_advisory_lock({lock_key})",
            app_literal,
            postgres::quote_ident(&gate_name),
        ))
        .await
        .unwrap();
    let (cancel, receiver) = watch::channel(false);
    let run_config = config.clone();
    let run_credentials = credentials;
    let running = tokio::spawn(async move {
        let args = Args::try_parse_from(["my2pg", "run", "fixture.toml", "--quiet"]).unwrap();
        let mut console = Console::new(&run_config, &args);
        let result =
            crate::test_pipeline::run(&run_config, &run_credentials, &mut console, receiver).await;
        drop(cancel);
        result
    });

    let waiting = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let state = target_admin
                .client
                .query_opt(
                    "SELECT query, wait_event_type FROM pg_catalog.pg_stat_activity \
                     WHERE application_name=$1 AND state='active' AND query ILIKE 'COPY %' \
                       AND wait_event_type='Lock'",
                    &[&app],
                )
                .await
                .unwrap();
            if let Some(row) = state {
                let query: String = row.get(0);
                assert!(
                    query.contains(&first),
                    "expected first table COPY, got {query}"
                );
                break;
            }
            assert!(
                !running.is_finished(),
                "migration exited before first COPY lock"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if let Err(timeout) = waiting {
        let states: Vec<(String, String, Option<String>)> = target_admin
            .client
            .query(
                "SELECT state, query, wait_event_type FROM pg_catalog.pg_stat_activity WHERE application_name=$1",
                &[&app],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1), row.get(2)))
            .collect();
        panic!("first COPY did not wait on the owned target lock: {timeout}; {states:?}");
    }

    // This committed source update happens after the runner has established
    // its repeatable-read view but before it starts reading the later table.
    admin
        .query_drop(format!(
            "UPDATE {} SET value='committed-after-snapshot' WHERE id=1",
            mysql::quote_ident(&later)
        ))
        .await
        .unwrap();
    let current_source: String = admin
        .exec_first(
            format!(
                "SELECT value FROM {} WHERE id=1",
                mysql::quote_ident(&later)
            ),
            (),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current_source, "committed-after-snapshot");
    let unlocked: bool = locker
        .client
        .query_one("SELECT pg_catalog.pg_advisory_unlock($1)", &[&lock_key])
        .await
        .unwrap()
        .get(0);
    assert!(unlocked, "test must release its own barrier lock");

    let report = tokio::time::timeout(Duration::from_secs(30), running)
        .await
        .expect("single-snapshot run completes after releasing COPY")
        .unwrap()
        .unwrap();
    assert_eq!(report.exit_code(), 0, "migration report: {report:?}");
    assert_eq!(report.tables.len(), 2);
    assert!(report.tables.iter().all(|table| table.committed_rows == 1));

    let first_rows: Vec<(i32, String)> = target_admin
        .client
        .query(
            &format!("SELECT id,value FROM {qualified_first} ORDER BY id"),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    let later_rows: Vec<(i32, String)> = target_admin
        .client
        .query(
            &format!("SELECT id,value FROM {qualified_later} ORDER BY id"),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(first_rows, [(1, "first-table".into())]);
    assert_eq!(
        later_rows,
        [(1, "snapshot-value".into())],
        "the later table must use the same source snapshot as the blocked first table"
    );

    locker
        .client
        .batch_execute(&format!(
            "DROP TRIGGER {} ON {qualified_first}; DROP FUNCTION {function}()",
            postgres::quote_ident(&gate_name)
        ))
        .await
        .unwrap();
    target_admin
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&first)))
        .await
        .unwrap();
    admin
        .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&later)))
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    locker.close().await.unwrap();
    target_admin.close().await.unwrap();
    let _ = std::fs::remove_dir_all(artifact_dir);
}
