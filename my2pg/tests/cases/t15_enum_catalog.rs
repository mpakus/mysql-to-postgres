use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, pipeline, postgres, report::Console, verify};
use mysql_async::prelude::Queryable;
use std::{
    collections::BTreeMap,
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing owned integration variable {name}"))
}

async fn snapshot(target: &postgres::TargetConnection, schema: &str) -> (u32, String, i64, String) {
    let relation = postgres::qualified(schema, "items");
    let row = target
        .client
        .query_one(
            &format!(
                "SELECT a.atttypid,pg_catalog.pg_get_expr(d.adbin,d.adrelid),
            (SELECT COUNT(*) FROM {relation}),(SELECT state::text FROM {relation} WHERE id=1)
            FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
            JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
            JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
            WHERE n.nspname=$1 AND c.relname='items' AND a.attname='state'"
            ),
            &[&schema],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2), row.get(3))
}

async fn check(
    source: &mut mysql::SourceConnection,
    target: &postgres::TargetConnection,
    plan: &MigrationPlan,
    report: &RunReport,
) -> VerificationReport {
    verify::counts_and_schema(source, target, plan, &report.tables, &BTreeMap::new())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires coordinator-owned disposable MySQL/PostgreSQL TLS harness"]
async fn live_enum_label_addition_preserves_oid_count_default_but_fails_schema_verification() {
    let name = format!(
        "t15_enum_{:x}_{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let artifact_dir = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&name);
    let config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":name,"ca_file":required("MY2PG_TLS_CA")},
        "tables":{"include":[name],"rename":[{"source":name,"target":"items"}]},
        "report":{"directory":artifact_dir,"progress":"never"}
    })).unwrap();
    let credentials = resolve_credentials(&config).unwrap();
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    let source_table = format!(
        "{}.{}",
        mysql::quote_ident("source"),
        mysql::quote_ident(&name)
    );
    // Fresh unique name; no DROP of pre-existing or shared fixture objects.
    admin.query_drop(format!("CREATE TABLE {source_table}(id INT PRIMARY KEY,state ENUM('red','green','comma,label','quote''label','雪','') NOT NULL DEFAULT 'red') ENGINE=InnoDB")).await.unwrap();
    admin
        .query_drop(format!("INSERT INTO {source_table}(id) VALUES(1)"))
        .await
        .unwrap();
    let plan = pipeline::plan(&config, &credentials).await.unwrap();
    assert_eq!(plan.tables.len(), 1);
    let column = plan.tables[0]
        .columns
        .iter()
        .find(|column| column.source_name == "state")
        .unwrap();
    assert_eq!(column.kind, ValueKind::Enum);
    assert_eq!(
        column.enum_labels,
        ["red", "green", "comma,label", "quote'label", "雪", ""]
    );
    let enum_type = column.target_type.clone();
    let args = Args::try_parse_from(["my2pg", "run", "synthetic.toml", "--quiet"]).unwrap();
    let (_sender, cancel) = watch::channel(false);
    let report = crate::test_pipeline::run(
        &config,
        &credentials,
        &mut Console::new(&config, &args),
        cancel,
    )
    .await
    .unwrap();
    assert_eq!(report.exit_code(), 0, "{:?}", report.verification);
    let target = postgres::connect(&config.target, credentials.target.expose())
        .await
        .unwrap();
    let mut source = mysql::connect(
        &config.source,
        credentials.source.expose(),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    let unchanged = check(&mut source, &target, &plan, &report).await;
    assert_eq!(
        unchanged.status,
        VerificationStatus::Complete,
        "{:?}",
        unchanged.differences
    );
    let before = snapshot(&target, &name).await;
    assert_eq!(before.2, 1);
    assert_eq!(before.3, "red");
    target
        .client
        .batch_execute(&format!(
            "ALTER TYPE {enum_type} ADD VALUE 'added' BEFORE 'green'"
        ))
        .await
        .unwrap();
    assert_eq!(
        snapshot(&target, &name).await,
        before,
        "DDL changed OID, row count, default or loaded value"
    );
    let added = check(&mut source, &target, &plan, &report).await;
    assert_eq!(
        added.status,
        VerificationStatus::Different,
        "{:?}",
        added.differences
    );
    assert!(
        added
            .differences
            .iter()
            .any(|difference| difference.contains("ordered ENUM labels")),
        "{:?}",
        added.differences
    );

    // A complete matching expectation passes; matching labels in a different order do not.
    // This is an expectation-order test, not a claim of unsupported same-OID ALTER reorder.
    let mut matching = plan.clone();
    let labels = &mut matching.tables[0]
        .columns
        .iter_mut()
        .find(|column| column.kind == ValueKind::Enum)
        .unwrap()
        .enum_labels;
    labels.insert(1, "added".into());
    assert_eq!(
        check(&mut source, &target, &matching, &report).await.status,
        VerificationStatus::Complete
    );
    let mut reordered = matching.clone();
    reordered.tables[0]
        .columns
        .iter_mut()
        .find(|column| column.kind == ValueKind::Enum)
        .unwrap()
        .enum_labels
        .swap(0, 1);
    assert_eq!(
        check(&mut source, &target, &reordered, &report)
            .await
            .status,
        VerificationStatus::Different
    );
    let mut missing = matching;
    missing.tables[0]
        .columns
        .iter_mut()
        .find(|column| column.kind == ValueKind::Enum)
        .unwrap()
        .enum_labels
        .clear();
    let unknown = check(&mut source, &target, &missing, &report).await;
    assert_eq!(
        unknown.status,
        VerificationStatus::Unsupported,
        "{:?}",
        unknown.differences
    );
    assert_eq!(
        admin
            .query_first::<u64, _>(format!("SELECT COUNT(*) FROM {source_table}"))
            .await
            .unwrap(),
        Some(1)
    );

    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&name)
        ))
        .await
        .unwrap();
    admin
        .query_drop(format!("DROP TABLE {source_table}"))
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    std::fs::remove_dir_all(artifact_dir).unwrap();
}
