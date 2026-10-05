//! Pure acceptance cases for the bounded content multiset primitive.

use my2pg::verify::content::{ContentComparison, compare_multisets};
use my2pg::{
    config::{Consistency, ExistingPolicy, MigrationMode, VerificationMode},
    model::{
        ColumnPlan, MigrationPlan, RunStatus, TablePlan, TableReport, ValueKind, VerificationStatus,
    },
    mysql, postgres,
    verify::content::{ContentVerificationOptions, capture_append_baseline, verify_content},
};
use mysql_async::prelude::Queryable;
use std::{collections::BTreeMap, env};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing integration variable {name}"))
}

fn text_column(name: &str, transform: Option<&str>) -> ColumnPlan {
    ColumnPlan {
        source_name: name.into(),
        target_name: name.into(),
        source_type: "varchar(64)".into(),
        target_type: "text".into(),
        kind: ValueKind::Text,
        nullable: false,
        default_sql: None,
        identity: false,
        generated_expression: None,
        copy: true,
        transform: transform.map(str::to_owned),
        charset: Some("utf8mb4".into()),
        enum_labels: Vec::new(),
        set_labels: Vec::new(),
        comment: None,
    }
}

fn live_plan(existing: ExistingPolicy, transform: Option<&str>) -> MigrationPlan {
    MigrationPlan {
        version: 1,
        source_database: "source".into(),
        source_version: "8.4".into(),
        target_schema: "t18_live_content".into(),
        target_version: "16".into(),
        consistency: Consistency::Frozen,
        mode: MigrationMode::Full,
        on_existing: existing,
        reset_sequences: true,
        verification: VerificationMode::Content,
        tables: vec![TablePlan {
            id: "t18_content_table".into(),
            source_name: "t18_live_content".into(),
            target_schema: "t18_live_content".into(),
            target_name: "rows".into(),
            engine: "InnoDB".into(),
            is_view: false,
            estimated_rows: None,
            next_auto_increment: None,
            columns: vec![text_column("value", transform)],
            primary_key: Vec::new(),
            structure: Default::default(),
        }],
        ddl: Vec::new(),
        diagnostics: Vec::new(),
        exclusions: Vec::new(),
        transformations: Vec::new(),
        hooks: Vec::new(),
    }
}

fn table_report(rows: u64) -> TableReport {
    TableReport {
        id: "t18_content_table".into(),
        source_name: "t18_live_content".into(),
        target_schema: "t18_live_content".into(),
        target_name: "rows".into(),
        status: RunStatus::Complete,
        rows_read: rows,
        committed_rows: rows,
        committed_bytes: 0,
        copy_elapsed_millis: 0,
        rejected_rows: 0,
        unresolved_rows: 0,
        indeterminate_rows: 0,
        transformations: BTreeMap::new(),
    }
}

fn live_config() -> my2pg::config::MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {"url_env":"MY2PG_MYSQL_URL", "consistency":"frozen", "ca_file":required("MY2PG_TLS_CA")},
        "target": {"url_env":"MY2PG_POSTGRES_URL", "schema":"t18_live_content", "ca_file":required("MY2PG_TLS_CA")}
    })).unwrap()
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn live_content_streams_transformed_rows_and_reports_equal_count_corruption() {
    let config = live_config();
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t18_live_content")
        .await
        .unwrap();
    admin
        .query_drop(
            "CREATE TABLE source.t18_live_content(value VARCHAR(64) NOT NULL) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    admin
        .query_drop(
            "INSERT INTO source.t18_live_content VALUES ('tail   '),('duplicate'),('duplicate')",
        )
        .await
        .unwrap();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute("DROP SCHEMA IF EXISTS t18_live_content CASCADE; CREATE SCHEMA t18_live_content; CREATE TABLE t18_live_content.rows(value text NOT NULL); INSERT INTO t18_live_content.rows VALUES ('tail'),('duplicate'),('duplicate')").await.unwrap();

    let plan = live_plan(ExistingPolicy::Error, Some("right-trim"));
    let passing = verify_content(
        &mut source,
        &mut target,
        &plan,
        &[table_report(3)],
        ContentVerificationOptions {
            memory_bytes: 2 * 256 * 1024,
            max_row_bytes: 1024,
            source_snapshot_already_pinned: false,
            append_baseline: None,
        },
    )
    .await;
    assert_eq!(
        passing.status,
        VerificationStatus::Complete,
        "{:?}",
        passing.differences
    );
    assert_eq!(passing.tables_checked, 1);

    source.disconnect().await.unwrap();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    target
        .client
        .batch_execute("UPDATE t18_live_content.rows SET value='corrupted' WHERE value='tail'")
        .await
        .unwrap();
    let different = verify_content(
        &mut source,
        &mut target,
        &plan,
        &[table_report(3)],
        ContentVerificationOptions {
            memory_bytes: 2 * 256 * 1024,
            max_row_bytes: 1024,
            source_snapshot_already_pinned: false,
            append_baseline: None,
        },
    )
    .await;
    assert_eq!(
        different.status,
        VerificationStatus::Different,
        "equal counts must not hide changed values: {:?}",
        different.differences
    );

    source.disconnect().await.unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA t18_live_content CASCADE")
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE source.t18_live_content")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn append_content_baseline_unites_existing_rows_and_migrated_rows_exactly() {
    let config = live_config();
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t18_live_content")
        .await
        .unwrap();
    admin
        .query_drop(
            "CREATE TABLE source.t18_live_content(value VARCHAR(64) NOT NULL) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute("DROP SCHEMA IF EXISTS t18_live_content CASCADE; CREATE SCHEMA t18_live_content; CREATE TABLE t18_live_content.rows(value text NOT NULL); INSERT INTO t18_live_content.rows VALUES ('preexisting'),('preexisting')").await.unwrap();
    let plan = live_plan(ExistingPolicy::Append, None);
    let baseline = capture_append_baseline(&mut target, &plan, 2 * 256 * 1024, 1024)
        .await
        .unwrap();

    admin
        .query_drop("INSERT INTO source.t18_live_content VALUES ('migrated')")
        .await
        .unwrap();
    target
        .client
        .batch_execute("INSERT INTO t18_live_content.rows VALUES ('migrated')")
        .await
        .unwrap();
    let result = verify_content(
        &mut source,
        &mut target,
        &plan,
        &[table_report(1)],
        ContentVerificationOptions {
            memory_bytes: 2 * 256 * 1024,
            max_row_bytes: 1024,
            source_snapshot_already_pinned: false,
            append_baseline: Some(baseline),
        },
    )
    .await;
    assert_eq!(
        result.status,
        VerificationStatus::Complete,
        "{:?}",
        result.differences
    );

    source.disconnect().await.unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA t18_live_content CASCADE")
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE source.t18_live_content")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[test]
fn equal_counts_do_not_hide_value_or_duplicate_multiplicity_changes() {
    let changed_value = compare_multisets(
        [b"key=1,value=old".to_vec(), b"key=2,value=x".to_vec()],
        [b"key=1,value=new".to_vec(), b"key=2,value=x".to_vec()],
        256 * 1024,
    )
    .unwrap();
    assert_eq!(changed_value.source_rows, changed_value.target_rows);
    assert!(!changed_value.equal);

    let changed_multiplicity = compare_multisets(
        [b"same".to_vec(), b"same".to_vec(), b"other".to_vec()],
        [b"same".to_vec(), b"other".to_vec(), b"other".to_vec()],
        256 * 1024,
    )
    .unwrap();
    assert_eq!(changed_multiplicity.source_rows, 3);
    assert_eq!(changed_multiplicity.target_rows, 3);
    assert!(!changed_multiplicity.equal);
}

#[test]
fn comparison_is_order_independent_and_merges_bounded_runs() {
    let source: Vec<_> = (0..9000)
        .map(|value| format!("row-{value:05}").into_bytes())
        .collect();
    let target: Vec<_> = source.iter().rev().cloned().collect();
    let result = compare_multisets(source, target, 256 * 1024).unwrap();
    assert_eq!(
        result,
        ContentComparison {
            equal: true,
            source_rows: 9000,
            target_rows: 9000,
        }
    );
}
