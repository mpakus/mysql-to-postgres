use bytes::Bytes;
use my2pg::{
    config::TargetConfig,
    model::{ColumnPlan, TablePlan, ValueKind},
    postgres::{self, CopyStage, FailureKind},
};
use std::env;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}
fn config() -> TargetConfig {
    serde_json::from_value(serde_json::json!({"url_env":"MY2PG_POSTGRES_URL","schema":"legacy","ca_file":required("MY2PG_TLS_CA")})).unwrap()
}
fn table(schema: &str, name: &str) -> TablePlan {
    let columns = [
        ("n", true),
        ("odd\"text", true),
        ("bytes", true),
        ("generated", false),
    ]
    .into_iter()
    .map(|(name, copy)| ColumnPlan {
        source_name: format!("original {name}"),
        target_name: name.into(),
        source_type: "".into(),
        target_type: "".into(),
        kind: ValueKind::Text,
        nullable: true,
        default_sql: None,
        identity: false,
        generated_expression: None,
        copy,
        transform: None,
        charset: None,
        comment: None,
        enum_labels: Vec::new(),
        set_labels: Vec::new(),
    })
    .collect();
    TablePlan {
        id: "t07".into(),
        source_name: "original table".into(),
        target_schema: schema.into(),
        target_name: name.into(),
        engine: "InnoDB".into(),
        is_view: false,
        estimated_rows: None,
        next_auto_increment: None,
        columns,
        primary_key: Vec::new(),
        structure: Default::default(),
    }
}

async fn gone(probe: &postgres::TargetConnection, pid: i32) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned canceled target backend must disappear");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture"]
async fn startup_failure_closes_connection_without_claiming_rollback() {
    let probe = postgres::connect(&config(), &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let mut conn = postgres::connect(&config(), &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let pid: i32 = conn
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let missing = postgres::copy_bytes(
        &mut conn,
        &table("public", "t07_missing_table"),
        Bytes::new(),
        0,
    )
    .await
    .unwrap_err();
    assert_eq!(missing.stage, CopyStage::Rollback);
    assert_eq!(missing.kind, FailureKind::Rollback);
    let cause = missing.cause.as_ref().expect("initial COPY error retained");
    assert_eq!(cause.stage, CopyStage::CopyInit);
    assert_eq!(cause.sqlstate.as_deref(), Some("42P01"));
    drop(conn);
    gone(&probe, pid).await;
    probe.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture"]
async fn cancellation_before_and_during_commit_preserves_stage() {
    let probe = postgres::connect(&config(), &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    probe.client.batch_execute("DROP SCHEMA IF EXISTS t07_cancel CASCADE; CREATE SCHEMA t07_cancel; CREATE TABLE t07_cancel.during_copy(n integer); CREATE TABLE t07_cancel.during_commit(n integer); CREATE FUNCTION t07_cancel.slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(30); RETURN NEW; END $$; CREATE TRIGGER slow_copy BEFORE INSERT ON t07_cancel.during_copy FOR EACH ROW EXECUTE FUNCTION t07_cancel.slow(); CREATE CONSTRAINT TRIGGER slow_commit AFTER INSERT ON t07_cancel.during_commit DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION t07_cancel.slow()").await.unwrap();
    for (name, expected) in [
        ("during_copy", CopyStage::CopyFinish),
        ("during_commit", CopyStage::CommitAttempt),
    ] {
        let mut conn = postgres::connect(&config(), &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap();
        let pid: i32 = conn
            .client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let shutdown = conn.shutdown_handle();
        let mut plan = table("t07_cancel", name);
        plan.columns.truncate(1);
        {
            let batch = postgres::copy_bytes(&mut conn, &plan, Bytes::from_static(b"1\n"), 1);
            tokio::pin!(batch);
            let sleeping = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let event: Option<String> = probe
                        .client
                        .query_one(
                            "SELECT wait_event FROM pg_stat_activity WHERE pid=$1",
                            &[&pid],
                        )
                        .await
                        .unwrap()
                        .get(0);
                    if shutdown.stage.get() == expected && event.as_deref() == Some("PgSleep") {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            });
            tokio::select! {
                result=&mut batch => panic!("COPY completed before cancellation: {result:?}"),
                result=sleeping => result.expect("COPY must reach the intended server wait stage"),
            }
            assert_eq!(shutdown.cancel().await.unwrap(), expected);
            // The batch future has not acknowledged COMMIT. Its drop cannot add
            // committed rows; CommitAttempt requires coordinator uncertainty.
        }
        drop(conn);
        gone(&probe, pid).await;
        assert_eq!(shutdown.stage.get(), expected);
        if expected == CopyStage::CopyFinish {
            assert_eq!(
                probe
                    .client
                    .query_one("SELECT count(*) FROM t07_cancel.during_copy", &[])
                    .await
                    .unwrap()
                    .get::<_, i64>(0),
                0
            );
        }
    }
    probe
        .client
        .batch_execute("DROP SCHEMA t07_cancel CASCADE")
        .await
        .unwrap();
    probe.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture"]
async fn copy_acknowledgement_rollback_metadata_and_qualification() {
    let mut conn = postgres::connect(&config(), &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    conn.client.batch_execute("DROP SCHEMA IF EXISTS t07_copy CASCADE; CREATE SCHEMA t07_copy; CREATE TABLE t07_copy.\"odd table\"(n integer PRIMARY KEY,\"odd\"\"text\" text, bytes bytea,generated integer GENERATED ALWAYS AS (n*2) STORED); CREATE TYPE t07_copy.state AS ENUM ('b','a,b'); CREATE VIEW t07_copy.display AS SELECT n FROM t07_copy.\"odd table\"; CREATE SEQUENCE t07_copy.sequence").await.unwrap();
    let plan = table("t07_copy", "odd table");
    let payload =
        Bytes::from_static(b"1\tliteral\\\\slash\\ttab\\nline\t\\\\x0000ff\n2\t\\N\t\\N\n");
    let stage = conn.stage_handle();
    assert_eq!(
        postgres::copy_bytes(&mut conn, &plan, payload, 2)
            .await
            .unwrap(),
        2
    );
    assert_eq!(stage.get(), CopyStage::Complete);
    let rows = conn
        .client
        .query(
            "SELECT n,\"odd\"\"text\",bytes,generated FROM t07_copy.\"odd table\" ORDER BY n",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, String>(1), "literal\\slash\ttab\nline");
    assert_eq!(rows[0].get::<_, Vec<u8>>(2), vec![0, 0, 255]);
    assert_eq!(rows[0].get::<_, i32>(3), 2);
    assert!(rows[1].get::<_, Option<String>>(1).is_none());
    let duplicate = postgres::copy_bytes(
        &mut conn,
        &plan,
        Bytes::from_static(b"3\tnew\t\\N\n1\tduplicate-secret\t\\N\n"),
        2,
    )
    .await
    .unwrap_err();
    assert_eq!(duplicate.kind, FailureKind::RowData);
    assert_eq!(duplicate.sqlstate.as_deref(), Some("23505"));
    assert!(!duplicate.to_string().contains("duplicate-secret"));
    let mismatch = postgres::copy_bytes(&mut conn, &plan, Bytes::from_static(b"3\tnew\t\\N\n"), 2)
        .await
        .unwrap_err();
    assert_eq!(mismatch.kind, FailureKind::Operational);
    assert_eq!(
        conn.client
            .query_one("SELECT count(*) FROM t07_copy.\"odd table\"", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        2
    );
    let catalog = postgres::inspect(&mut conn.client, "t07_copy")
        .await
        .unwrap();
    assert!(catalog.schema_exists && catalog.can_use_schema && catalog.can_create_objects);
    assert_eq!(
        catalog.tables[0]
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["n", "odd\"text", "bytes", "generated"]
    );
    assert!(catalog.tables[0].columns[3].generated);
    assert_eq!(catalog.enums["state"], ["b", "a,b"]);
    assert_eq!(catalog.occupied_names["odd table"], "r");
    assert_eq!(catalog.occupied_names["display"], "v");
    assert_eq!(catalog.occupied_names["sequence"], "S");
    assert_eq!(
        catalog.tables.len(),
        1,
        "views/sequences cannot be ordinary COPY tables"
    );
    assert!(catalog.dependencies.iter().any(|dep| dep.kind == "view"
        && dep.dependent_table == "display"
        && dep.referenced_table == "odd table"));
    let sentinel: String = conn
        .client
        .query_one("SELECT sentinel FROM public.users WHERE id=999", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sentinel, "must-survive");
    conn.client
        .batch_execute("DROP SCHEMA t07_copy CASCADE")
        .await
        .unwrap();
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture"]
async fn restricted_role_privileges_and_external_dependency_are_visible() {
    let admin = postgres::connect(&config(), &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    admin.client.batch_execute("DROP SCHEMA IF EXISTS t07_priv CASCADE; DROP SCHEMA IF EXISTS t07_external CASCADE; DROP ROLE IF EXISTS t07_reader; CREATE ROLE t07_reader LOGIN PASSWORD 'integration-only'; CREATE SCHEMA t07_priv; CREATE SCHEMA t07_external; CREATE TABLE t07_priv.parent(n integer PRIMARY KEY); CREATE TABLE t07_external.child(n integer REFERENCES t07_priv.parent); ALTER TABLE t07_priv.parent ENABLE ROW LEVEL SECURITY; CREATE POLICY reader ON t07_priv.parent USING(true); GRANT USAGE ON SCHEMA t07_priv TO t07_reader; GRANT SELECT ON t07_priv.parent TO t07_reader").await.unwrap();
    let mut reader_url = url::Url::parse(&required("MY2PG_POSTGRES_URL")).unwrap();
    reader_url.set_username("t07_reader").unwrap();
    let mut reader = postgres::connect(&config(), reader_url.as_str())
        .await
        .unwrap();
    let catalog = postgres::inspect(&mut reader.client, "t07_priv")
        .await
        .unwrap();
    assert!(catalog.can_use_schema);
    assert!(!catalog.can_create_objects);
    assert!(!catalog.can_create_schema);
    assert!(catalog.tables[0].can_select);
    assert!(!catalog.tables[0].can_insert && !catalog.tables[0].can_alter);
    assert!(!catalog.tables[0].can_truncate);
    assert!(catalog.tables[0].row_security_active);
    let mut restricted = table("t07_priv", "parent");
    restricted.columns.truncate(1);
    let denied = postgres::copy_bytes(&mut reader, &restricted, Bytes::from_static(b"1\n"), 1)
        .await
        .unwrap_err();
    assert_eq!(denied.stage, CopyStage::Rollback, "{denied:?}");
    assert_eq!(denied.kind, FailureKind::Rollback);
    let original = denied
        .cause
        .as_ref()
        .expect("startup cause must be retained");
    assert_eq!(original.stage, CopyStage::CopyInit);
    assert_eq!(original.sqlstate.as_deref(), Some("42501"));
    assert_eq!(original.kind, FailureKind::Operational);
    assert!(
        catalog
            .external_dependencies
            .iter()
            .any(|name| name.contains("child")),
        "incoming foreign key dependency must be visible"
    );
    assert!(
        catalog
            .dependencies
            .iter()
            .any(|dep| dep.kind == "foreign_key"
                && dep.dependent_schema == "t07_external"
                && dep.dependent_table == "child"
                && dep.referenced_table == "parent")
    );
    drop(reader);
    admin
        .client
        .batch_execute(
            "DROP SCHEMA t07_external CASCADE; DROP SCHEMA t07_priv CASCADE; DROP ROLE t07_reader",
        )
        .await
        .unwrap();
    admin.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires TLS-enabled isolated PostgreSQL fixture"]
async fn tls_password_file_and_dropped_guard_are_real() {
    let source = config();
    let url = required("MY2PG_POSTGRES_URL");
    let mut bad = source.clone();
    bad.ca_file = Some(required("MY2PG_TLS_BAD_CA").into());
    assert!(postgres::connect(&bad, &url).await.is_err());
    let mut bad_url = url::Url::parse(&url).unwrap();
    bad_url.set_query(Some("sslmode=disable"));
    assert!(postgres::connect(&source, bad_url.as_str()).await.is_err());
    let mut password_url = url::Url::parse(&url).unwrap();
    password_url.set_password(None).unwrap();
    let path = std::path::PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t07.pgpass");
    std::fs::write(
        &path,
        format!(
            "*:5432:*:*:wrong\n{}:{}:target:my2pg:integration-only\n",
            password_url.host_str().unwrap(),
            password_url.port().unwrap()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut pass = source.clone();
    pass.passfile = Some(path.clone());
    let conn = postgres::connect(&pass, password_url.as_str())
        .await
        .unwrap();
    assert!(
        conn.client
            .query_one(
                "SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let pid: i32 = conn
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let shutdown = conn.shutdown_handle();
    assert_eq!(shutdown.abort(), CopyStage::Idle);
    drop(conn);
    let probe = postgres::connect(&source, &url).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("aborted target guard must close the backend promptly");
    probe.close().await.unwrap();
    std::fs::remove_file(path).unwrap();
}
