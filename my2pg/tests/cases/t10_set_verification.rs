use my2pg::{config::*, model::*, mysql, postgres, verify};
use std::{collections::BTreeMap, env};

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixture"]
async fn typed_set_membership_matches_catalog_and_detects_changed_or_unvalidated_checks() {
    let source: SourceConfig = serde_json::from_value(serde_json::json!({"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":env::var("MY2PG_TLS_CA").unwrap()})).unwrap();
    let target: TargetConfig = serde_json::from_value(serde_json::json!({"url_env":"MY2PG_POSTGRES_URL","schema":"t10_set_verify","ca_file":env::var("MY2PG_TLS_CA").unwrap()})).unwrap();
    let mut source = mysql::connect(&source, &env::var("MY2PG_MYSQL_URL").unwrap(), 65536)
        .await
        .unwrap();
    let target = postgres::connect(&target, &env::var("MY2PG_POSTGRES_URL").unwrap())
        .await
        .unwrap();
    let mut plan: MigrationPlan =
        serde_json::from_str(include_str!("../contracts/plan.json")).unwrap();
    plan.mode = MigrationMode::SchemaOnly;
    plan.tables.truncate(1);
    let table = &mut plan.tables[0];
    table.target_schema = "t10_set_verify".into();
    table.target_name = "rows".into();
    table.primary_key.clear();
    table.columns.truncate(1);
    let column = &mut table.columns[0];
    column.target_name = "odd\"SET".into();
    column.kind = ValueKind::Set;
    column.target_type = "text[]".into();
    column.identity = false;
    column.nullable = true;
    column.default_sql = None;
    column.comment = None;
    let membership = SetMembershipExpectation {
        column: column.target_name.clone(),
        labels: vec![
            "".into(),
            "a,b".into(),
            "O'Reilly".into(),
            "back\\slash".into(),
            "Ω".into(),
        ],
    };
    table.structure = SchemaExpectations {
        complete: true,
        checks: vec![CheckExpectation {
            name: "member_check".into(),
            expression: String::new(),
            set_membership: Some(membership.clone()),
        }],
        ..Default::default()
    };
    let mut fixture: RunReport =
        serde_json::from_str(include_str!("../contracts/report.json")).unwrap();
    let report = &mut fixture.tables[0];
    report.id = table.id.clone();
    report.source_name = table.source_name.clone();
    report.target_schema = table.target_schema.clone();
    report.target_name = table.target_name.clone();
    report.rows_read = 0;
    report.committed_rows = 0;
    let values = membership
        .labels
        .iter()
        .map(|label| format!("E'{}'", label.replace('\\', "\\\\").replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    target.client.batch_execute(&format!("DROP SCHEMA IF EXISTS t10_set_verify CASCADE; CREATE SCHEMA t10_set_verify; CREATE TABLE t10_set_verify.rows(\"odd\"\"SET\" text[]); ALTER TABLE t10_set_verify.rows ADD CONSTRAINT member_check CHECK (\"odd\"\"SET\" <@ ARRAY[{values}]::text[])")).await.unwrap();
    let actual: String = target.client.query_one("SELECT pg_get_expr(conbin,conrelid) FROM pg_constraint WHERE conrelid='t10_set_verify.rows'::regclass AND conname='member_check'", &[]).await.unwrap().get(0);
    let result = verify::counts_and_schema(
        &mut source,
        &target,
        &plan,
        &fixture.tables,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        result.status,
        VerificationStatus::Complete,
        "{actual}: {result:?}"
    );
    target.client.batch_execute("ALTER TABLE t10_set_verify.rows DROP CONSTRAINT member_check; ALTER TABLE t10_set_verify.rows ADD CONSTRAINT member_check CHECK (\"odd\"\"SET\" <@ ARRAY['different']::text[])").await.unwrap();
    let result = verify::counts_and_schema(
        &mut source,
        &target,
        &plan,
        &fixture.tables,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.status, VerificationStatus::Different);
    assert!(
        result
            .differences
            .iter()
            .any(|difference| difference.contains("SET membership"))
    );
    target.client.batch_execute(&format!("ALTER TABLE t10_set_verify.rows DROP CONSTRAINT member_check; ALTER TABLE t10_set_verify.rows ADD CONSTRAINT member_check CHECK (\"odd\"\"SET\" <@ ARRAY[{values}]::text[]) NOT VALID")).await.unwrap();
    let result = verify::counts_and_schema(
        &mut source,
        &target,
        &plan,
        &fixture.tables,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.status, VerificationStatus::Different);
    target
        .client
        .batch_execute("DROP TABLE t10_set_verify.rows; DROP SCHEMA t10_set_verify")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
