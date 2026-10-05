use bytes::Bytes;
use my2pg::{
    config::{RowErrorPolicy, TargetConfig},
    model::{
        BatchStorage, ColumnPlan, EncodedBatch, RowLocator, RowPosition, TablePlan, ValueKind,
    },
    pipeline::recovery::{self, RecoveryContext, RecoveryFailure, RecoveryProgress, RejectBudget},
    postgres::{self, CopyStage, FailureKind, TargetConnection},
    report::RunArtifacts,
};
use std::{
    env, fs, io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}
async fn connect() -> TargetConnection {
    let config: TargetConfig = serde_json::from_value(serde_json::json!({"url_env":"MY2PG_POSTGRES_URL","schema":"legacy","ca_file":required("MY2PG_TLS_CA")})).unwrap();
    postgres::connect(&config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap()
}
fn table(schema: &str, names: &[&str]) -> TablePlan {
    TablePlan {
        id: "t_0000000000000012".into(),
        source_name: "source".into(),
        target_schema: schema.into(),
        target_name: "rows".into(),
        engine: "InnoDB".into(),
        is_view: false,
        estimated_rows: None,
        next_auto_increment: None,
        primary_key: vec![],
        structure: Default::default(),
        columns: names
            .iter()
            .map(|name| ColumnPlan {
                source_name: (*name).into(),
                target_name: (*name).into(),
                source_type: "".into(),
                target_type: "".into(),
                kind: ValueKind::Text,
                nullable: true,
                default_sql: None,
                identity: false,
                generated_expression: None,
                copy: true,
                transform: None,
                charset: None,
                comment: None,
                enum_labels: vec![],
                set_labels: vec![],
            })
            .collect(),
    }
}
async fn fixture(definition: &str, names: &[&str]) -> (TargetConnection, TablePlan, RunArtifacts) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let schema = format!(
        "t12_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let conn = connect().await;
    conn.client
        .batch_execute(&format!(
            "CREATE SCHEMA {};CREATE TABLE {} ({definition})",
            postgres::quote_ident(&schema),
            postgres::qualified(&schema, "rows")
        ))
        .await
        .unwrap();
    let artifacts =
        RunArtifacts::create(&PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t12")).unwrap();
    (conn, table(&schema, names), artifacts)
}
async fn encoded_batch(lines: &[&[u8]]) -> (EncodedBatch, Arc<Semaphore>) {
    let mut bytes = Vec::new();
    let mut rows = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let start = bytes.len();
        bytes.extend_from_slice(line);
        rows.push(RowPosition {
            start,
            end: bytes.len(),
            locator: RowLocator {
                ordinal: index as u64 + 1,
                key: None,
            },
        });
    }
    let semaphore = Arc::new(Semaphore::new(bytes.len().max(1)));
    let permit = semaphore
        .clone()
        .acquire_many_owned(bytes.len().max(1) as u32)
        .await
        .unwrap();
    (
        EncodedBatch {
            storage: Arc::new(BatchStorage {
                bytes: Bytes::from(bytes),
                permit,
            }),
            rows,
        },
        semaphore,
    )
}
async fn run(
    conn: &mut TargetConnection,
    table: &TablePlan,
    batch: &EncodedBatch,
    artifacts: &RunArtifacts,
    budget: &RejectBudget,
    policy: RowErrorPolicy,
) -> Result<RecoveryProgress, recovery::RecoveryError> {
    let mut observer = |_| Ok(());
    recovery::copy_with_recovery(
        conn,
        table,
        batch,
        &mut RecoveryContext {
            policy,
            artifacts,
            reject_budget: budget,
            observer: &mut observer,
        },
    )
    .await
}
async fn count(conn: &TargetConnection, table: &TablePlan) -> i64 {
    conn.client
        .query_one(
            &format!(
                "SELECT count(*) FROM {}",
                postgres::qualified(&table.target_schema, &table.target_name)
            ),
            &[],
        )
        .await
        .unwrap()
        .get(0)
}
async fn cleanup(conn: &TargetConnection, table: &TablePlan) {
    conn.client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&table.target_schema)
        ))
        .await
        .unwrap();
}
fn rejects(artifacts: &RunArtifacts) -> Vec<serde_json::Value> {
    let path = artifacts.directory.join("t_0000000000000012.reject.jsonl");
    if !path.exists() {
        return vec![];
    }
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn bad_row_positions_exact_bytes_and_left_first_duplicates() {
    for bad in [vec![0], vec![2], vec![4], vec![1, 3], vec![0, 1, 2, 3, 4]] {
        let (mut conn, table, artifacts) = fixture(
            "n integer PRIMARY KEY CHECK(n>=0), value text, bytes bytea",
            &["n", "value", "bytes"],
        )
        .await;
        let mut lines = Vec::new();
        for index in 0..5 {
            lines.push(
                format!(
                    "{}\t{}\t\\\\x00ff09\n",
                    if bad.contains(&index) {
                        -((index + 1) as i32)
                    } else {
                        index as i32
                    },
                    if index == 0 {
                        "\\N"
                    } else if index == 1 {
                        ""
                    } else if index == 2 {
                        "\\\\N"
                    } else {
                        "hé\\t雪\\n"
                    }
                )
                .into_bytes(),
            );
        }
        let refs: Vec<&[u8]> = lines.iter().map(Vec::as_slice).collect();
        let (batch, semaphore) = encoded_batch(&refs).await;
        let progress = run(
            &mut conn,
            &table,
            &batch,
            &artifacts,
            &RejectBudget::new(5),
            RowErrorPolicy::Reject,
        )
        .await
        .unwrap();
        assert_eq!(progress.committed_rows, 5 - bad.len() as u64);
        assert_eq!(progress.rejected_rows, bad.len() as u64);
        assert_eq!(progress.pending_rows, 0);
        assert_eq!(count(&conn, &table).await, progress.committed_rows as i64);
        assert_eq!(semaphore.available_permits(), 0);
        let recorded = rejects(&artifacts);
        assert_eq!(
            recorded
                .iter()
                .map(|record| record["locator"]["ordinal"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            bad.iter()
                .map(|index| *index as u64 + 1)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            fs::read(artifacts.directory.join("t_0000000000000012.reject.copy")).unwrap(),
            bad.iter()
                .flat_map(|index| lines[*index].clone())
                .collect::<Vec<_>>()
        );
        let rows = conn
            .client
            .query(
                &format!(
                    "SELECT n,value,bytes FROM {} ORDER BY n",
                    postgres::qualified(&table.target_schema, "rows")
                ),
                &[],
            )
            .await
            .unwrap();
        for row in rows {
            let n: i32 = row.get(0);
            let value: Option<String> = row.get(1);
            assert_eq!(row.get::<_, Vec<u8>>(2), [0, 255, 9]);
            assert_eq!(
                value,
                match n {
                    0 => None,
                    1 => Some("".into()),
                    2 => Some("\\N".into()),
                    _ => Some("hé\t雪\n".into()),
                }
            );
        }
        let bytes = batch.storage.bytes.len();
        drop(batch);
        assert_eq!(semaphore.available_permits(), bytes);
        cleanup(&conn, &table).await;
    }
    let (mut conn, table, artifacts) =
        fixture("n integer PRIMARY KEY,value text", &["n", "value"]).await;
    let (first, _) = encoded_batch(&[b"8\tearlier\n"]).await;
    run(
        &mut conn,
        &table,
        &first,
        &artifacts,
        &RejectBudget::new(10),
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap();
    let (duplicate, _) = encoded_batch(&[
        b"1\tfirst\n",
        b"2\tok\n",
        b"1\tlater\n",
        b"8\tconflict\n",
        b"3\tlast\n",
    ])
    .await;
    let progress = run(
        &mut conn,
        &table,
        &duplicate,
        &artifacts,
        &RejectBudget::new(10),
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap();
    assert_eq!((progress.committed_rows, progress.rejected_rows), (3, 2));
    let values = conn
        .client
        .query(
            &format!(
                "SELECT value FROM {} ORDER BY n",
                postgres::qualified(&table.target_schema, "rows")
            ),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();
    assert_eq!(values, ["first", "ok", "last", "earlier"]);
    assert_eq!(rejects(&artifacts).len(), 2);
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn ordinary_error_codes_limits_stop_and_artifact_failure() {
    for (definition, bytes, state) in [
        ("n integer NOT NULL", b"\\N\n".as_slice(), "23502"),
        ("n varchar(2)", b"long\n", "22001"),
        ("n smallint", b"40000\n", "22003"),
        ("n integer", b"no\n", "22P02"),
        ("n date", b"not-date\n", "22007"),
    ] {
        let (mut conn, table, artifacts) = fixture(definition, &["n"]).await;
        let (batch, _) = encoded_batch(&[bytes]).await;
        let progress = run(
            &mut conn,
            &table,
            &batch,
            &artifacts,
            &RejectBudget::new(1),
            RowErrorPolicy::Reject,
        )
        .await
        .unwrap();
        assert_eq!((progress.committed_rows, progress.rejected_rows), (0, 1));
        assert!(
            rejects(&artifacts)[0]["reason"]["message"]
                .as_str()
                .unwrap()
                .contains(state)
        );
        cleanup(&conn, &table).await;
    }
    let (mut conn, table, artifacts) = fixture("n integer CHECK(n>=0)", &["n"]).await;
    let budget = RejectBudget::new(2);
    let (first, _) = encoded_batch(&[b"-1\n", b"1\n"]).await;
    run(
        &mut conn,
        &table,
        &first,
        &artifacts,
        &budget,
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap();
    let (second, _) = encoded_batch(&[b"-2\n", b"2\n"]).await;
    run(
        &mut conn,
        &table,
        &second,
        &artifacts,
        &budget,
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap();
    assert_eq!(budget.used(), 2);
    let (third, _) = encoded_batch(&[b"-3\n", b"3\n"]).await;
    let failure = run(
        &mut conn,
        &table,
        &third,
        &artifacts,
        &budget,
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.kind, RecoveryFailure::RejectLimit);
    assert_eq!(rejects(&artifacts).len(), 2);
    assert_eq!(count(&conn, &table).await, 2);
    let (stopped, _) = encoded_batch(&[b"4\n", b"-4\n", b"5\n"]).await;
    let failure = run(
        &mut conn,
        &table,
        &stopped,
        &artifacts,
        &budget,
        RowErrorPolicy::Stop,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.progress.pending_rows, 3);
    assert_eq!(failure.progress.active_rows, 0);
    assert_eq!(count(&conn, &table).await, 2);
    assert_eq!(rejects(&artifacts).len(), 2);
    let broken =
        RunArtifacts::create(&PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t12")).unwrap();
    fs::create_dir(broken.directory.join("t_0000000000000012.reject.copy")).unwrap();
    let fresh_budget = RejectBudget::new(1);
    let failure = run(
        &mut conn,
        &table,
        &third,
        &broken,
        &fresh_budget,
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.kind, RecoveryFailure::RejectIo);
    assert_eq!(failure.progress.rejected_rows, 0);
    assert_eq!(fresh_budget.used(), 0);
    assert!(broken.flush().is_err());
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn safety_guard_blocks_custom_behavior_and_preserves_safe_enums_arrays() {
    let (mut conn, table, artifacts) = fixture("n integer CHECK(n>=0)", &["n"]).await;
    let schema = postgres::quote_ident(&table.target_schema);
    let name = postgres::qualified(&table.target_schema, "rows");
    conn.client.batch_execute(&format!("CREATE FUNCTION {schema}.spoof() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RAISE EXCEPTION 'private row content' USING ERRCODE='23514'; END$$; CREATE TRIGGER user_spoof BEFORE INSERT ON {name} FOR EACH ROW EXECUTE FUNCTION {schema}.spoof() ")).await.unwrap();
    let (batch, _) = encoded_batch(&[b"1\n", b"-1\n"]).await;
    let failure = run(
        &mut conn,
        &table,
        &batch,
        &artifacts,
        &RejectBudget::new(10),
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.target.unwrap().kind, FailureKind::Configuration);
    assert_eq!(count(&conn, &table).await, 0);
    assert!(rejects(&artifacts).is_empty());
    conn.client
        .batch_execute(&format!("ALTER TABLE {name} DISABLE TRIGGER user_spoof"))
        .await
        .unwrap();
    recovery::check_recovery_safety(&conn, &table)
        .await
        .unwrap();
    conn.client.batch_execute(&format!("CREATE FUNCTION {schema}.custom_check(integer) RETURNS boolean LANGUAGE plpgsql IMMUTABLE AS $$BEGIN RAISE EXCEPTION 'private' USING ERRCODE='23514'; END$$; ALTER TABLE {name} ADD CHECK ({schema}.custom_check(n)) NOT VALID")).await.unwrap();
    assert!(
        recovery::check_recovery_safety(&conn, &table)
            .await
            .is_err()
    );
    cleanup(&conn, &table).await;
    let (conn, mut table, _) = fixture("n integer", &["n"]).await;
    let schema = postgres::quote_ident(&table.target_schema);
    let name = postgres::qualified(&table.target_schema, "rows");
    conn.client.batch_execute(&format!("CREATE TYPE {schema}.labels AS ENUM ('a,b','雪');ALTER TABLE {name} ADD COLUMN label {schema}.labels, ADD COLUMN labels text[]")).await.unwrap();
    table.columns.extend(crate_columns(&["label", "labels"]));
    recovery::check_recovery_safety(&conn, &table)
        .await
        .unwrap();
    conn.client.batch_execute(&format!("CREATE DOMAIN {schema}.unsafe_domain AS integer CHECK(VALUE>0);ALTER TABLE {name} ADD COLUMN custom {schema}.unsafe_domain")).await.unwrap();
    assert!(
        recovery::check_recovery_safety(&conn, &table)
            .await
            .is_err()
    );
    cleanup(&conn, &table).await;
    for ddl in [
        "ADD COLUMN generated integer GENERATED ALWAYS AS (n+1) STORED",
        "ADD COLUMN omitted double precision DEFAULT random()",
        "ADD COLUMN omitted_identity bigint GENERATED BY DEFAULT AS IDENTITY",
        "ADD CHECK(random()>0)",
        "",
    ] {
        let (conn, table, _) = fixture("n integer", &["n"]).await;
        let name = postgres::qualified(&table.target_schema, "rows");
        if ddl.is_empty() {
            conn.client
                .batch_execute(&format!("CREATE INDEX expression ON {name} ((n+1))"))
                .await
                .unwrap();
        } else {
            conn.client
                .batch_execute(&format!("ALTER TABLE {name} {ddl}"))
                .await
                .unwrap();
        }
        assert!(
            recovery::check_recovery_safety(&conn, &table)
                .await
                .is_err(),
            "{ddl}"
        );
        cleanup(&conn, &table).await;
    }
}
fn crate_columns(names: &[&str]) -> Vec<ColumnPlan> {
    table("unused", names).columns
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn real_deadlock_rolls_back_retries_identical_slice_and_commits_once() {
    let (mut conn, table, artifacts) = fixture("n integer PRIMARY KEY", &["n"]).await;
    conn.client
        .batch_execute("SET deadlock_timeout='50ms'")
        .await
        .unwrap();
    let pid: i32 = conn
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let rival = connect().await;
    let probe = connect().await;
    let name = postgres::qualified(&table.target_schema, "rows");
    rival
        .client
        .batch_execute(&format!("BEGIN;INSERT INTO {name} VALUES(2)"))
        .await
        .unwrap();
    let (batch, semaphore) = encoded_batch(&[b"1\n", b"2\n"]).await;
    let budget = RejectBudget::new(10);
    let mut submitted = 0;
    let mut observer = |progress: RecoveryProgress| {
        if progress.active_rows == 2 {
            submitted += 1;
            assert_eq!(semaphore.available_permits(), 0);
        }
        Ok(())
    };
    let mut context = RecoveryContext {
        policy: RowErrorPolicy::Stop,
        artifacts: &artifacts,
        reject_budget: &budget,
        observer: &mut observer,
    };
    let copy = recovery::copy_with_recovery(&mut conn, &table, &batch, &mut context);
    let compete = async {
        tokio::time::timeout(Duration::from_secs(3),async {loop {let waiting:bool=probe.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')",&[&pid]).await.unwrap().get(0);if waiting{break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
        rival
            .client
            .batch_execute(&format!("INSERT INTO {name} VALUES(1);ROLLBACK"))
            .await
            .unwrap();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(copy, compete)
    })
    .await
    .unwrap();
    let progress = result.unwrap();
    assert_eq!(progress.committed_rows, 2);
    assert_eq!(
        submitted, 2,
        "actual deadlock must cause exactly one bounded retry"
    );
    assert_eq!(count(&conn, &table).await, 2);
    assert!(rejects(&artifacts).is_empty());
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn malformed_batch_observer_stop_and_fatal_constraint_have_no_replay() {
    let (mut conn, table, artifacts) = fixture("n integer PRIMARY KEY", &["n"]).await;
    let (mut batch, _) = encoded_batch(&[b"1\n", b"2\n"]).await;
    batch.rows[1].start = 0;
    let budget = RejectBudget::new(10);
    assert_eq!(
        run(
            &mut conn,
            &table,
            &batch,
            &artifacts,
            &budget,
            RowErrorPolicy::Reject
        )
        .await
        .unwrap_err()
        .kind,
        RecoveryFailure::InvalidBatch
    );
    assert_eq!(count(&conn, &table).await, 0);
    batch.rows[1].start = 2;
    let mut observer = |_| Err(io::Error::other("cancel request"));
    let failure = recovery::copy_with_recovery(
        &mut conn,
        &table,
        &batch,
        &mut RecoveryContext {
            policy: RowErrorPolicy::Reject,
            artifacts: &artifacts,
            reject_budget: &budget,
            observer: &mut observer,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(failure.kind, RecoveryFailure::ObserverIo);
    assert_eq!(count(&conn, &table).await, 0);
    cleanup(&conn, &table).await;
    let (mut conn, table, artifacts) =
        fixture("n integer UNIQUE DEFERRABLE INITIALLY DEFERRED", &["n"]).await;
    let (batch, _) = encoded_batch(&[b"1\n", b"1\n"]).await;
    let failure = run(
        &mut conn,
        &table,
        &batch,
        &artifacts,
        &budget,
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap_err();
    let target = failure.target.unwrap();
    assert_eq!(target.stage, CopyStage::CommitAttempt);
    assert_eq!(target.sqlstate.as_deref(), Some("23505"));
    assert_eq!(count(&conn, &table).await, 0);
    assert!(rejects(&artifacts).is_empty());
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn cancellation_after_left_ack_and_unknown_right_commit_preserves_progress() {
    let (mut conn, table, artifacts) = fixture(
        "n integer UNIQUE DEFERRABLE INITIALLY DEFERRED CHECK(n>=0)",
        &["n"],
    )
    .await;
    let rival = connect().await;
    let probe = connect().await;
    let name = postgres::qualified(&table.target_schema, "rows");
    rival
        .client
        .batch_execute(&format!("BEGIN;INSERT INTO {name} VALUES(2)"))
        .await
        .unwrap();
    let pid: i32 = conn
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let stage = conn.stage_handle();
    let shutdown = conn.shutdown_handle();
    let (batch, semaphore) = encoded_batch(&[b"1\n", b"-1\n", b"2\n"]).await;
    let budget = RejectBudget::new(1);
    let progress = Arc::new(std::sync::Mutex::new(RecoveryProgress::default()));
    let captured = progress.clone();
    let mut observer = move |value| {
        *captured.lock().unwrap() = value;
        Ok(())
    };
    let mut context = RecoveryContext {
        policy: RowErrorPolicy::Reject,
        artifacts: &artifacts,
        reject_budget: &budget,
        observer: &mut observer,
    };
    let copy = recovery::copy_with_recovery(&mut conn, &table, &batch, &mut context);
    let interrupt = async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let waiting: bool = probe.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')", &[&pid]).await.unwrap().get(0);
                if stage.get()==CopyStage::CommitAttempt && waiting {break;}
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        assert_eq!(
            *progress.lock().unwrap(),
            RecoveryProgress {
                committed_rows: 1,
                rejected_rows: 1,
                active_rows: 1,
                pending_rows: 0
            }
        );
        assert_eq!(semaphore.available_permits(), 0);
        shutdown.cancel().await.ok();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(copy, interrupt)
    })
    .await
    .unwrap();
    let failure = result.unwrap_err();
    assert_eq!(failure.target.unwrap().kind, FailureKind::Indeterminate);
    assert_eq!(
        failure.progress,
        RecoveryProgress {
            committed_rows: 1,
            rejected_rows: 1,
            active_rows: 1,
            pending_rows: 0
        }
    );
    assert_eq!(rejects(&artifacts).len(), 1);
    assert_eq!(budget.used(), 1);
    rival.client.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(count(&probe, &table).await, 1);
    cleanup(&probe, &table).await;

    let (mut conn, table, artifacts) = fixture("n integer CHECK(n>=0)", &["n"]).await;
    let (batch, _) = encoded_batch(&[b"1\n", b"-1\n", b"2\n"]).await;
    let mut observer = |progress: RecoveryProgress| {
        if progress.committed_rows == 1 {
            Err(io::Error::other("stop after acknowledged left slice"))
        } else {
            Ok(())
        }
    };
    let failure = recovery::copy_with_recovery(
        &mut conn,
        &table,
        &batch,
        &mut RecoveryContext {
            policy: RowErrorPolicy::Reject,
            artifacts: &artifacts,
            reject_budget: &budget,
            observer: &mut observer,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(failure.kind, RecoveryFailure::ObserverIo);
    assert_eq!(failure.progress.committed_rows, 1);
    assert_eq!(failure.progress.pending_rows, 2);
    assert_eq!(count(&conn, &table).await, 1);
    assert!(rejects(&artifacts).is_empty());
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture and child-only file quota"]
async fn real_partial_reject_write_failure_never_claims_durable_reject() {
    const CHILD: &str = "MY2PG_T12_FILE_QUOTA_CHILD";
    if env::var_os(CHILD).is_none() {
        // Apply a real OS file-size quota only to this disposable child, never the harness.
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "trap '' XFSZ; ulimit -f 1; exec \"$@\"",
                "t12-file-quota",
            ])
            .arg(env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "t12_recovery::real_partial_reject_write_failure_never_claims_durable_reject",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "quota child failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (mut conn, table, artifacts) =
        fixture("n integer CHECK(n>=0),value text", &["n", "value"]).await;
    let line = format!("-1\t{}\n", "x".repeat(8192));
    let (batch, _) = encoded_batch(&[line.as_bytes()]).await;
    let budget = RejectBudget::new(1);
    let failure = run(
        &mut conn,
        &table,
        &batch,
        &artifacts,
        &budget,
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.kind, RecoveryFailure::RejectIo);
    assert_eq!(failure.progress.rejected_rows, 0);
    assert_eq!(budget.used(), 0);
    assert_eq!(count(&conn, &table).await, 0);
    let partial = fs::metadata(artifacts.directory.join("t_0000000000000012.reject.copy"))
        .unwrap()
        .len();
    assert!(
        partial > 0 && partial < line.len() as u64,
        "must prove actual partial data write"
    );
    assert!(rejects(&artifacts).is_empty());
    assert!(artifacts.flush().is_err());
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn genuine_repeated_deadlocks_exhaust_three_attempts_without_committing() {
    let (mut conn, table, artifacts) = fixture("n integer PRIMARY KEY", &["n"]).await;
    conn.client
        .batch_execute("SET deadlock_timeout='50ms'")
        .await
        .unwrap();
    let pid: i32 = conn
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let rival = connect().await;
    let probe = connect().await;
    let name = postgres::qualified(&table.target_schema, "rows");
    rival
        .client
        .batch_execute(&format!("BEGIN;INSERT INTO {name} VALUES(2)"))
        .await
        .unwrap();
    let (batch, _) = encoded_batch(&[b"1\n", b"2\n"]).await;
    let budget = RejectBudget::new(1);
    let mut submitted = 0;
    let mut observer = |progress: RecoveryProgress| {
        if progress.active_rows == 2 {
            submitted += 1;
        }
        Ok(())
    };
    let mut context = RecoveryContext {
        policy: RowErrorPolicy::Stop,
        artifacts: &artifacts,
        reject_budget: &budget,
        observer: &mut observer,
    };
    let copy = recovery::copy_with_recovery(&mut conn, &table, &batch, &mut context);
    let compete = async {
        for attempt in 0..3 {
            tokio::time::timeout(Duration::from_secs(2),async {loop {let waiting:bool=probe.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')",&[&pid]).await.unwrap().get(0);if waiting{break;}tokio::time::sleep(Duration::from_millis(2)).await;}}).await.unwrap();
            rival
                .client
                .batch_execute(&format!("INSERT INTO {name} VALUES(1);ROLLBACK"))
                .await
                .unwrap();
            if attempt < 2 {
                rival
                    .client
                    .batch_execute(&format!("BEGIN;INSERT INTO {name} VALUES(2)"))
                    .await
                    .unwrap();
            }
        }
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(copy, compete)
    })
    .await
    .unwrap();
    let failure = result.unwrap_err();
    assert_eq!(failure.kind, RecoveryFailure::RetryExhausted);
    assert_eq!(failure.target.unwrap().sqlstate.as_deref(), Some("40P01"));
    assert_eq!(submitted, 3);
    assert_eq!(
        failure.progress,
        RecoveryProgress {
            pending_rows: 2,
            ..Default::default()
        }
    );
    assert_eq!(count(&conn, &table).await, 0);
    assert!(rejects(&artifacts).is_empty());
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn forged_retry_code_custom_operators_and_classes_are_not_replayed() {
    let (mut conn, table, artifacts) = fixture("n integer", &["n"]).await;
    let schema = postgres::quote_ident(&table.target_schema);
    let name = postgres::qualified(&table.target_schema, "rows");
    conn.client.batch_execute(&format!("CREATE SEQUENCE {schema}.side_effect;CREATE FUNCTION {schema}.forged_retry() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM nextval('{schema}.side_effect');RAISE EXCEPTION 'private data' USING ERRCODE='40001';END$$;CREATE TRIGGER forged BEFORE INSERT ON {name} FOR EACH ROW EXECUTE FUNCTION {schema}.forged_retry()")).await.unwrap();
    let (batch, _) = encoded_batch(&[b"1\n"]).await;
    let mut submissions = 0;
    let mut observer = |progress: RecoveryProgress| {
        if progress.active_rows > 0 {
            submissions += 1;
        }
        Ok(())
    };
    let budget = RejectBudget::new(1);
    let failure = recovery::copy_with_recovery(
        &mut conn,
        &table,
        &batch,
        &mut RecoveryContext {
            policy: RowErrorPolicy::Stop,
            artifacts: &artifacts,
            reject_budget: &budget,
            observer: &mut observer,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(submissions, 1);
    assert_eq!(failure.target.unwrap().kind, FailureKind::Configuration);
    let sequence = conn
        .client
        .query_one(
            &format!("SELECT last_value,is_called FROM {schema}.side_effect"),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(sequence.get::<_, i64>(0), 1);
    assert!(sequence.get::<_, bool>(1));
    assert!(rejects(&artifacts).is_empty());
    cleanup(&conn, &table).await;
    let (conn, table, _) = fixture("n integer", &["n"]).await;
    let schema = postgres::quote_ident(&table.target_schema);
    let name = postgres::qualified(&table.target_schema, "rows");
    conn.client.batch_execute(&format!("CREATE FUNCTION {schema}.operator_check(integer,integer) RETURNS boolean LANGUAGE sql IMMUTABLE AS 'SELECT $1 >= $2';CREATE OPERATOR {schema}.## (FUNCTION={schema}.operator_check,LEFTARG=integer,RIGHTARG=integer);ALTER TABLE {name} ADD CHECK(n OPERATOR({schema}.##) 0)")).await.unwrap();
    assert!(
        recovery::check_recovery_safety(&conn, &table)
            .await
            .is_err()
    );
    cleanup(&conn, &table).await;
    let (conn, table, _) = fixture("n integer", &["n"]).await;
    let schema = postgres::quote_ident(&table.target_schema);
    let name = postgres::qualified(&table.target_schema, "rows");
    conn.client.batch_execute(&format!("CREATE OPERATOR CLASS {schema}.custom_int_ops FOR TYPE integer USING btree AS OPERATOR 1 <,OPERATOR 2 <=,OPERATOR 3 =,OPERATOR 4 >=,OPERATOR 5 >,FUNCTION 1 btint4cmp(integer,integer);CREATE INDEX custom_class ON {name} USING btree(n {schema}.custom_int_ops)")).await.unwrap();
    assert!(
        recovery::check_recovery_safety(&conn, &table)
            .await
            .is_err()
    );
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn typed_set_membership_check_is_safe_but_changed_or_custom_casts_block() {
    use my2pg::model::{CheckExpectation, SetMembershipExpectation};
    let (mut conn, mut table, artifacts) = fixture(
        "n integer,labels text[] CONSTRAINT membership CHECK(labels <@ ARRAY['','a,b','雪'])",
        &["n", "labels"],
    )
    .await;
    table.structure.checks.push(CheckExpectation {
        name: "membership".into(),
        expression: "".into(),
        set_membership: Some(SetMembershipExpectation {
            column: "labels".into(),
            labels: vec!["".into(), "a,b".into(), "雪".into()],
        }),
    });
    recovery::check_recovery_safety(&conn, &table)
        .await
        .unwrap();
    let (batch, _) =
        encoded_batch(&[b"1\t{\"a,b\"}\n", b"2\t{bad}\n", b"3\t{}\n", b"4\t\\N\n"]).await;
    let progress = run(
        &mut conn,
        &table,
        &batch,
        &artifacts,
        &RejectBudget::new(1),
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap();
    assert_eq!((progress.committed_rows, progress.rejected_rows), (3, 1));
    assert_eq!(rejects(&artifacts)[0]["locator"]["ordinal"], 2);
    table.structure.checks[0]
        .set_membership
        .as_mut()
        .unwrap()
        .labels
        .push("unexpected".into());
    assert!(
        recovery::check_recovery_safety(&conn, &table)
            .await
            .is_err()
    );
    cleanup(&conn, &table).await;
    let (conn, table, _) = fixture("n integer", &["n"]).await;
    let schema = postgres::quote_ident(&table.target_schema);
    let name = postgres::qualified(&table.target_schema, "rows");
    conn.client.batch_execute(&format!("CREATE DOMAIN {schema}.\"integer\" AS integer CHECK(VALUE>=0);ALTER TABLE {name} ADD CHECK((n::{schema}.\"integer\")>=0)")).await.unwrap();
    assert!(
        recovery::check_recovery_safety(&conn, &table)
            .await
            .is_err()
    );
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn real_connection_loss_after_partial_progress_and_foreign_keys_never_replay() {
    let (mut conn, table, artifacts) = fixture("n integer UNIQUE CHECK(n>=0)", &["n"]).await;
    let rival = connect().await;
    let probe = connect().await;
    let name = postgres::qualified(&table.target_schema, "rows");
    rival
        .client
        .batch_execute(&format!("BEGIN;INSERT INTO {name} VALUES(2)"))
        .await
        .unwrap();
    let pid: i32 = conn
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let stage = conn.stage_handle();
    let (batch, _) = encoded_batch(&[b"1\n", b"-1\n", b"2\n"]).await;
    let budget = RejectBudget::new(1);
    let mut observer = |_| Ok(());
    let mut context = RecoveryContext {
        policy: RowErrorPolicy::Reject,
        artifacts: &artifacts,
        reject_budget: &budget,
        observer: &mut observer,
    };
    let copy = recovery::copy_with_recovery(&mut conn, &table, &batch, &mut context);
    let terminate = async {
        tokio::time::timeout(Duration::from_secs(3),async {loop {let waiting:bool=probe.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')",&[&pid]).await.unwrap().get(0);if stage.get()==CopyStage::CopyFinish && waiting{break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
        assert!(
            probe
                .client
                .query_one("SELECT pg_terminate_backend($1)", &[&pid])
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(copy, terminate)
    })
    .await
    .unwrap();
    let failure = result.unwrap_err();
    let target = failure.target.unwrap();
    assert_eq!(target.kind, FailureKind::Rollback);
    assert!(target.cause.is_some());
    assert_eq!(
        failure.progress,
        RecoveryProgress {
            committed_rows: 1,
            rejected_rows: 1,
            active_rows: 1,
            pending_rows: 0
        }
    );
    assert_eq!(rejects(&artifacts).len(), 1);
    rival.client.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(count(&probe, &table).await, 1);
    cleanup(&probe, &table).await;
    let (mut conn, table, artifacts) = fixture("n integer", &["n"]).await;
    let schema = postgres::quote_ident(&table.target_schema);
    let name = postgres::qualified(&table.target_schema, "rows");
    conn.client.batch_execute(&format!("CREATE TABLE {schema}.parent(n integer PRIMARY KEY);INSERT INTO {schema}.parent VALUES(1);ALTER TABLE {name} ADD FOREIGN KEY(n) REFERENCES {schema}.parent(n)")).await.unwrap();
    let (batch, _) = encoded_batch(&[b"1\n", b"99\n"]).await;
    let failure = run(
        &mut conn,
        &table,
        &batch,
        &artifacts,
        &RejectBudget::new(10),
        RowErrorPolicy::Reject,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.target.unwrap().sqlstate.as_deref(), Some("23503"));
    assert_eq!(
        failure.progress,
        RecoveryProgress {
            pending_rows: 2,
            ..Default::default()
        }
    );
    assert_eq!(count(&conn, &table).await, 0);
    assert!(rejects(&artifacts).is_empty());
    cleanup(&conn, &table).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn real_serialization_conflict_is_observed_then_retried_with_one_ack() {
    for recover in [false, true] {
        let (mut conn, table, artifacts) = fixture("n integer", &["n"]).await;
        let schema = postgres::quote_ident(&table.target_schema);
        let name = postgres::qualified(&table.target_schema, "rows");
        conn.client.batch_execute(&format!("CREATE TABLE {schema}.parent(n integer PRIMARY KEY);INSERT INTO {schema}.parent VALUES(1);ALTER TABLE {name} ADD FOREIGN KEY(n) REFERENCES {schema}.parent(n);SET default_transaction_isolation='serializable'")).await.unwrap();
        let pid: i32 = conn
            .client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let rival = connect().await;
        let probe = connect().await;
        rival
            .client
            .batch_execute(&format!(
                "BEGIN;DELETE FROM {schema}.parent WHERE n=1;INSERT INTO {schema}.parent VALUES(1)"
            ))
            .await
            .unwrap();
        let interfere = async {
            tokio::time::timeout(Duration::from_secs(3),async {loop {let waiting:bool=probe.client.query_one("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')",&[&pid]).await.unwrap().get(0);if waiting{break;}tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
            rival.client.batch_execute("COMMIT").await.unwrap();
        };
        if !recover {
            let copy = postgres::copy_bytes(&mut conn, &table, Bytes::from_static(b"1\n"), 1);
            let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(copy, interfere)
            })
            .await
            .unwrap();
            let failure = result.unwrap_err();
            assert_eq!(failure.sqlstate.as_deref(), Some("40001"));
            assert_eq!(failure.kind, FailureKind::TransactionRetry);
            assert_eq!(failure.stage, CopyStage::CopyFinish);
            assert!(failure.cause.is_none());
            assert_eq!(conn.stage_handle().get(), CopyStage::Idle);
            assert_eq!(count(&conn, &table).await, 0);
        } else {
            let (batch, semaphore) = encoded_batch(&[b"1\n"]).await;
            let budget = RejectBudget::new(1);
            let mut submissions = 0;
            let mut observer = |progress: RecoveryProgress| {
                if progress.active_rows == 1 {
                    submissions += 1;
                    assert_eq!(semaphore.available_permits(), 0);
                }
                Ok(())
            };
            let mut context = RecoveryContext {
                policy: RowErrorPolicy::Reject,
                artifacts: &artifacts,
                reject_budget: &budget,
                observer: &mut observer,
            };
            let copy = recovery::copy_with_recovery(&mut conn, &table, &batch, &mut context);
            let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(copy, interfere)
            })
            .await
            .unwrap();
            assert_eq!(
                result.unwrap(),
                RecoveryProgress {
                    committed_rows: 1,
                    ..Default::default()
                }
            );
            assert_eq!(submissions, 2);
            assert_eq!(count(&conn, &table).await, 1);
            assert_eq!(budget.used(), 0);
            assert!(rejects(&artifacts).is_empty());
        }
        cleanup(&conn, &table).await;
    }
}

async fn backend_gone(probe: &TargetConnection, pid: i32) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let alive: bool = probe
                .client
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1)",
                    &[&pid],
                )
                .await
                .unwrap()
                .get(0);
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("failed startup must close its backend promptly");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture and owned restricted roles"]
async fn init_permission_missing_relation_and_columns_are_never_bisected() {
    for scenario in [
        "permission",
        "missing_table_stop",
        "missing_table_reject",
        "missing_column",
        "no_copy_columns",
    ] {
        let (mut conn, mut table, artifacts) = fixture("n integer", &["n"]).await;
        let original_table = table.clone();
        let probe = connect().await;
        let pid: i32 = conn
            .client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let role = format!("{}_reader", table.target_schema);
        if scenario == "permission" {
            probe
                .client
                .batch_execute(&format!(
                    "CREATE ROLE {} NOLOGIN;GRANT USAGE ON SCHEMA {} TO {}",
                    postgres::quote_ident(&role),
                    postgres::quote_ident(&table.target_schema),
                    postgres::quote_ident(&role)
                ))
                .await
                .unwrap();
            conn.client
                .batch_execute(&format!("SET ROLE {}", postgres::quote_ident(&role)))
                .await
                .unwrap();
        } else if scenario.starts_with("missing_table") {
            table.target_name = "actually_missing".into();
        } else if scenario == "missing_column" {
            table.columns[0].target_name = "actually_missing".into();
        } else {
            table.columns[0].copy = false;
        }
        let policy = if scenario == "missing_table_stop" {
            RowErrorPolicy::Stop
        } else {
            RowErrorPolicy::Reject
        };
        let (batch, _) = encoded_batch(&[b"1\n", b"2\n", b"3\n"]).await;
        let budget = RejectBudget::new(3);
        let mut submissions = 0;
        let mut observer = |progress: RecoveryProgress| {
            if progress.active_rows > 0 {
                submissions += 1;
            }
            Ok(())
        };
        let failure = recovery::copy_with_recovery(
            &mut conn,
            &table,
            &batch,
            &mut RecoveryContext {
                policy,
                artifacts: &artifacts,
                reject_budget: &budget,
                observer: &mut observer,
            },
        )
        .await
        .unwrap_err();
        let target = failure.target.unwrap();
        assert_eq!(failure.kind, RecoveryFailure::Database);
        assert_eq!(failure.progress.committed_rows, 0);
        assert_eq!(failure.progress.rejected_rows, 0);
        assert_eq!(count(&probe, &original_table).await, 0);
        assert_eq!(budget.used(), 0);
        assert!(rejects(&artifacts).is_empty());
        if scenario == "missing_table_reject" {
            assert_eq!(target.kind, FailureKind::Configuration);
            assert_eq!(submissions, 0);
        } else if scenario == "no_copy_columns" {
            assert_eq!(target.kind, FailureKind::Configuration);
            assert_eq!(target.stage, CopyStage::Idle);
            assert_eq!(submissions, 1);
        } else {
            assert_eq!(submissions, 1);
            assert_eq!(target.kind, FailureKind::Rollback);
            assert_eq!(target.stage, CopyStage::Rollback);
            let cause = target.cause.unwrap();
            assert_eq!(cause.stage, CopyStage::CopyInit);
            assert_eq!(
                cause.sqlstate.as_deref(),
                Some(match scenario {
                    "permission" => "42501",
                    "missing_column" => "42703",
                    _ => "42P01",
                })
            );
            drop(conn);
            backend_gone(&probe, pid).await;
        }
        cleanup(&probe, &original_table).await;
        if scenario == "permission" {
            probe
                .client
                .batch_execute(&format!("DROP ROLE {}", postgres::quote_ident(&role)))
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL TLS fixture"]
async fn inheritance_parent_child_and_partition_members_are_blocked_without_mutation() {
    for partitioned in [false, true] {
        let (mut conn, mut planned, artifacts) = fixture("n integer", &["n"]).await;
        let parent = postgres::qualified(&planned.target_schema, "rows");
        let child = postgres::qualified(&planned.target_schema, "child");
        let plain = postgres::qualified(&planned.target_schema, "plain");
        let sql = if partitioned {
            format!(
                "DROP TABLE {parent};CREATE TABLE {parent}(n integer) PARTITION BY RANGE(n);CREATE TABLE {child} PARTITION OF {parent} FOR VALUES FROM(0) TO(10);INSERT INTO {child} VALUES(2)"
            )
        } else {
            format!(
                "CREATE TABLE {child}() INHERITS({parent});INSERT INTO {parent} VALUES(1);INSERT INTO {child} VALUES(2)"
            )
        };
        conn.client.batch_execute(&sql).await.unwrap();
        let (batch, _) = encoded_batch(&[b"3\n"]).await;
        let budget = RejectBudget::new(1);
        let mut submissions = 0;
        for name in ["rows", "child"] {
            planned.target_name = name.into();
            let failure = recovery::check_recovery_safety(&conn, &planned)
                .await
                .unwrap_err();
            assert_eq!(failure.kind, FailureKind::Configuration);
            let mut observer = |_| {
                submissions += 1;
                Ok(())
            };
            let failure = recovery::copy_with_recovery(
                &mut conn,
                &planned,
                &batch,
                &mut RecoveryContext {
                    policy: RowErrorPolicy::Reject,
                    artifacts: &artifacts,
                    reject_budget: &budget,
                    observer: &mut observer,
                },
            )
            .await
            .unwrap_err();
            assert_eq!(failure.target.unwrap().kind, FailureKind::Configuration);
            assert_eq!(conn.stage_handle().get(), CopyStage::Idle);
        }
        assert_eq!(submissions, 0, "guard must run before initial COPY");
        assert_eq!(budget.used(), 0);
        assert!(rejects(&artifacts).is_empty());
        for (relation, expected) in [
            (&parent, if partitioned { vec![] } else { vec![1] }),
            (&child, vec![2]),
        ] {
            let actual: Vec<i32> = conn
                .client
                .query(&format!("SELECT n FROM ONLY {relation} ORDER BY n"), &[])
                .await
                .unwrap()
                .iter()
                .map(|row| row.get(0))
                .collect();
            assert_eq!(actual, expected, "physical sentinel rows are preserved");
        }
        conn.client
            .batch_execute(&format!("CREATE TABLE {plain}(n integer)"))
            .await
            .unwrap();
        planned.target_name = "plain".into();
        recovery::check_recovery_safety(&conn, &planned)
            .await
            .unwrap();
        cleanup(&conn, &planned).await;
        conn.close().await.unwrap();
    }
}
