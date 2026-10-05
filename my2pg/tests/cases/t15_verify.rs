use bytes::Bytes;
use my2pg::{config::*, model::*, mysql, postgres, verify};
use mysql_async::prelude::Queryable;
use std::{collections::BTreeMap, env};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing integration variable {name}"))
}

fn table() -> TablePlan {
    TablePlan {
        id: "t_0123456789abcdef".into(),
        source_name: "all_bytes".into(),
        target_schema: "t15_verify".into(),
        target_name: "renamed bytes".into(),
        engine: "InnoDB".into(),
        is_view: false,
        estimated_rows: None,
        next_auto_increment: None,
        columns: [
            ("id", "integer", ValueKind::SignedInteger, false),
            ("value", "bytea", ValueKind::Binary, true),
        ]
        .into_iter()
        .map(|(name, kind, value_kind, nullable)| ColumnPlan {
            source_name: name.into(),
            target_name: name.into(),
            source_type: if name == "id" { "int" } else { "blob" }.into(),
            target_type: kind.into(),
            kind: value_kind,
            nullable,
            default_sql: None,
            identity: false,
            generated_expression: None,
            copy: true,
            transform: None,
            charset: None,
            enum_labels: vec![],
            set_labels: vec![],
            comment: None,
        })
        .collect(),
        primary_key: vec!["id".into()],
        structure: SchemaExpectations {
            complete: true,
            indexes: vec![IndexExpectation {
                name: "t15_pk".into(),
                columns: vec!["id".into()],
                expressions: vec![None],
                unique: true,
                primary: true,
                descending: vec![false],
            }],
            foreign_keys: vec![],
            checks: vec![],
            sequences: vec![],
            comment: Some("verifier fixture".into()),
        },
    }
}
fn plan() -> MigrationPlan {
    MigrationPlan {
        version: 1,
        source_database: "source".into(),
        source_version: "8.4".into(),
        target_schema: "t15_verify".into(),
        target_version: "16".into(),
        consistency: Consistency::Frozen,
        mode: MigrationMode::Full,
        on_existing: ExistingPolicy::Error,
        reset_sequences: true,
        verification: VerificationMode::CountsAndSchema,
        tables: vec![table()],
        ddl: vec![],
        diagnostics: vec![],
        exclusions: vec![],
        transformations: vec![],
        hooks: vec![],
    }
}
fn report() -> TableReport {
    TableReport {
        committed_bytes: 0,
        copy_elapsed_millis: 0,
        id: "t_0123456789abcdef".into(),
        source_name: "all_bytes".into(),
        target_schema: "t15_verify".into(),
        target_name: "renamed bytes".into(),
        status: RunStatus::Complete,
        rows_read: 1,
        committed_rows: 1,
        rejected_rows: 0,
        unresolved_rows: 0,
        indeterminate_rows: 0,
        transformations: BTreeMap::new(),
    }
}

async fn sequence_result(
    source: &mut mysql::SourceConnection,
    target: &postgres::TargetConnection,
    plan: &MigrationPlan,
    report: &TableReport,
) -> VerificationReport {
    verify::counts_and_schema(
        source,
        target,
        plan,
        std::slice::from_ref(report),
        &BTreeMap::new(),
    )
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated MySQL/PostgreSQL TLS fixtures"]
async fn safe_typed_default_literals_match_real_catalog_and_detect_changes() {
    let config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,"source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},"target":{"url_env":"MY2PG_POSTGRES_URL","schema":"t15_defaults","ca_file":required("MY2PG_TLS_CA")}})).unwrap();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute(r"DROP SCHEMA IF EXISTS t15_defaults CASCADE; CREATE SCHEMA t15_defaults; CREATE TABLE t15_defaults.rows(id integer NOT NULL,value bytea,empty varchar(20) DEFAULT '',label text DEFAULT E'it''s::ok\\path',number numeric(5,2) DEFAULT 1.50,flag boolean DEFAULT false,negative integer DEFAULT -3,CONSTRAINT t15_pk PRIMARY KEY(id)); COMMENT ON TABLE t15_defaults.rows IS 'verifier fixture'").await.unwrap();
    let mut checked = plan();
    checked.mode = MigrationMode::SchemaOnly;
    checked.target_schema = "t15_defaults".into();
    let table = &mut checked.tables[0];
    table.target_schema = "t15_defaults".into();
    table.target_name = "rows".into();
    for (name, kind, value_kind, default) in [
        ("empty", "varchar(20)", ValueKind::Text, "E''"),
        ("label", "text", ValueKind::Text, r"E'it''s::ok\\path'"),
        ("number", "numeric(5,2)", ValueKind::Decimal, "+01.500"),
        ("flag", "boolean", ValueKind::Boolean, "FALSE"),
        ("negative", "integer", ValueKind::SignedInteger, "-3"),
    ] {
        let mut column = table.columns[1].clone();
        column.source_name = name.into();
        column.target_name = name.into();
        column.target_type = kind.into();
        column.kind = value_kind;
        column.default_sql = Some(default.into());
        table.columns.push(column);
    }
    let mut accounting = report();
    accounting.rows_read = 0;
    accounting.committed_rows = 0;
    accounting.target_schema = "t15_defaults".into();
    accounting.target_name = "rows".into();
    let good = sequence_result(&mut source, &target, &checked, &accounting).await;
    assert_eq!(
        good.status,
        VerificationStatus::Complete,
        "{:?}",
        good.differences
    );
    for (column, changed, restore) in [
        ("empty", "'changed'", "''"),
        ("label", "'changed'", r"E'it''s::ok\\path'"),
        ("number", "2.50", "1.50"),
        ("flag", "true", "false"),
        ("negative", "-4", "-3"),
    ] {
        target
            .client
            .batch_execute(&format!(
                "ALTER TABLE t15_defaults.rows ALTER COLUMN {column} SET DEFAULT {changed}"
            ))
            .await
            .unwrap();
        let different = sequence_result(&mut source, &target, &checked, &accounting).await;
        assert_eq!(
            different.status,
            VerificationStatus::Different,
            "{column}: {:?}",
            different.differences
        );
        target
            .client
            .batch_execute(&format!(
                "ALTER TABLE t15_defaults.rows ALTER COLUMN {column} SET DEFAULT {restore}"
            ))
            .await
            .unwrap();
    }
    target
        .client
        .batch_execute("ALTER TABLE t15_defaults.rows ALTER COLUMN number SET DEFAULT random()")
        .await
        .unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked, &accounting)
            .await
            .status,
        VerificationStatus::Unsupported,
        "volatile expressions cannot be evaluated for verification"
    );
    target
        .client
        .batch_execute("DROP SCHEMA t15_defaults CASCADE")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated MySQL/PostgreSQL TLS fixtures"]
async fn explicit_identity_generation_state_and_exhaustion_are_verified_without_consumption() {
    let config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,"source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},"target":{"url_env":"MY2PG_POSTGRES_URL","schema":"t15_sequences","ca_file":required("MY2PG_TLS_CA")}})).unwrap();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute("DROP SCHEMA IF EXISTS t15_sequences CASCADE; CREATE SCHEMA t15_sequences; CREATE TABLE t15_sequences.rows(id bigint GENERATED BY DEFAULT AS IDENTITY,value bytea,CONSTRAINT t15_pk PRIMARY KEY(id)); COMMENT ON TABLE t15_sequences.rows IS 'verifier fixture'; INSERT INTO t15_sequences.rows VALUES(1,NULL); SELECT setval('t15_sequences.rows_id_seq',2,false)").await.unwrap();
    let mut checked_plan = plan();
    checked_plan.target_schema = "t15_sequences".into();
    let table = &mut checked_plan.tables[0];
    table.target_schema = "t15_sequences".into();
    table.target_name = "rows".into();
    table.columns[0].target_type = "bigint".into();
    table.columns[0].identity = true;
    table.structure.sequences.push(SequenceExpectation {
        column: "id".into(),
        increment: 1,
        min_value: 1,
        max_value: i64::MAX,
        cycle: false,
        next_minimum: 2,
        preserved_state: None,
    });
    let mut checked_report = report();
    checked_report.target_schema = "t15_sequences".into();
    checked_report.target_name = "rows".into();
    let good = sequence_result(&mut source, &target, &checked_plan, &checked_report).await;
    assert_eq!(
        good.status,
        VerificationStatus::Complete,
        "{:?}",
        good.differences
    );
    let before = target
        .client
        .query_one(
            "SELECT last_value,is_called FROM t15_sequences.rows_id_seq",
            &[],
        )
        .await
        .unwrap();
    sequence_result(&mut source, &target, &checked_plan, &checked_report).await;
    let after = target
        .client
        .query_one(
            "SELECT last_value,is_called FROM t15_sequences.rows_id_seq",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        (before.get::<_, i64>(0), before.get::<_, bool>(1)),
        (after.get::<_, i64>(0), after.get::<_, bool>(1)),
        "verification must not call nextval/setval"
    );
    for (change, restore, label) in [
        ("INCREMENT BY 2", "INCREMENT BY 1", "increment"),
        ("INCREMENT BY -1", "INCREMENT BY 1", "negative increment"),
        ("CYCLE", "NO CYCLE", "cycle"),
        ("MAXVALUE 100", "MAXVALUE 9223372036854775807", "maximum"),
        ("MINVALUE 0", "MINVALUE 1", "minimum"),
    ] {
        target
            .client
            .batch_execute(&format!(
                "ALTER SEQUENCE t15_sequences.rows_id_seq {change}"
            ))
            .await
            .unwrap();
        let actual = sequence_result(&mut source, &target, &checked_plan, &checked_report).await;
        assert_eq!(
            actual.status,
            VerificationStatus::Different,
            "{label}: {:?}",
            actual.differences
        );
        assert!(
            actual
                .differences
                .iter()
                .any(|difference| difference.contains("sequence")),
            "{label}"
        );
        target
            .client
            .batch_execute(&format!(
                "ALTER SEQUENCE t15_sequences.rows_id_seq {restore}"
            ))
            .await
            .unwrap();
    }
    target
        .client
        .batch_execute("ALTER SEQUENCE t15_sequences.rows_id_seq CACHE 4")
        .await
        .unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Unsupported
    );
    target.client.batch_execute("ALTER SEQUENCE t15_sequences.rows_id_seq CACHE 1; SELECT setval('t15_sequences.rows_id_seq',1,true)").await.unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Complete,
        "is_called adds increment without consuming"
    );
    checked_plan.tables[0].next_auto_increment = Some(42);
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Different
    );
    target
        .client
        .batch_execute("SELECT setval('t15_sequences.rows_id_seq',42,false)")
        .await
        .unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Complete
    );
    checked_plan.tables[0].structure.sequences[0].next_minimum = 50;
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Different
    );
    target
        .client
        .batch_execute("SELECT setval('t15_sequences.rows_id_seq',50,false)")
        .await
        .unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Complete
    );
    checked_plan.tables[0].next_auto_increment = None;
    checked_plan.tables[0].structure.sequences[0].next_minimum = 1;
    target.client.batch_execute("UPDATE t15_sequences.rows SET id=-5; SELECT setval('t15_sequences.rows_id_seq',1,false)").await.unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Complete,
        "negative loaded max must not lower ascending minimum below1"
    );
    checked_plan.mode = MigrationMode::SchemaOnly;
    checked_report.rows_read = 0;
    checked_report.committed_rows = 0;
    target
        .client
        .batch_execute("TRUNCATE t15_sequences.rows")
        .await
        .unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Complete,
        "empty sequence starts at1"
    );
    target
        .client
        .batch_execute("SELECT setval('t15_sequences.rows_id_seq',9223372036854775807,true)")
        .await
        .unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Different,
        "called signed maximum is exhausted"
    );
    target.client.batch_execute("INSERT INTO t15_sequences.rows VALUES(9223372036854775807,NULL); SELECT setval('t15_sequences.rows_id_seq',9223372036854775807,false)").await.unwrap();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Different,
        "loaded max+1 exceeds signed range"
    );
    target
        .client
        .batch_execute(
            "TRUNCATE t15_sequences.rows; SELECT setval('t15_sequences.rows_id_seq',1,false)",
        )
        .await
        .unwrap();
    checked_plan.tables[0].next_auto_increment = Some(u64::MAX);
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Different,
        "unsigned sourceNext cannot fit a signed sequence"
    );
    checked_plan.tables[0].next_auto_increment = None;
    checked_plan.tables[0].structure.sequences.clear();
    assert_eq!(
        sequence_result(&mut source, &target, &checked_plan, &checked_report)
            .await
            .status,
        VerificationStatus::Unsupported,
        "absent expectation cannot certify generation"
    );
    target
        .client
        .batch_execute("DROP SCHEMA t15_sequences CASCADE")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
#[tokio::test]
#[ignore = "requires isolated MySQL/PostgreSQL TLS fixtures"]
async fn actual_counts_columns_keys_validity_and_honest_scope() {
    let config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,"source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},"target":{"url_env":"MY2PG_POSTGRES_URL","schema":"t15_verify","ca_file":required("MY2PG_TLS_CA")}})).unwrap();
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
    target.client.batch_execute("DROP SCHEMA IF EXISTS t15_verify CASCADE; CREATE SCHEMA t15_verify; CREATE TABLE t15_verify.\"renamed bytes\"(id integer NOT NULL,value bytea,CONSTRAINT t15_pk PRIMARY KEY(id)); COMMENT ON TABLE t15_verify.\"renamed bytes\" IS 'verifier fixture'").await.unwrap();
    let plan = plan();
    let hex: String = (0..=255u8).map(|byte| format!("{byte:02x}")).collect();
    postgres::copy_bytes(
        &mut target,
        &plan.tables[0],
        Bytes::from(format!("1\t\\\\x{hex}\n")),
        1,
    )
    .await
    .unwrap();
    let reports = [report()];
    let checked =
        verify::counts_and_schema(&mut source, &target, &plan, &reports, &BTreeMap::new())
            .await
            .unwrap();
    assert_eq!(
        checked.status,
        VerificationStatus::Complete,
        "{:?}",
        checked.differences
    );
    assert_eq!(checked.tables_checked, 1);
    let mut incomplete = plan.clone();
    incomplete.tables[0].structure.complete = false;
    assert_eq!(
        verify::counts_and_schema(
            &mut source,
            &target,
            &incomplete,
            &reports,
            &BTreeMap::new()
        )
        .await
        .unwrap()
        .status,
        VerificationStatus::Unsupported
    );
    target.client.batch_execute("CREATE TABLE t15_verify.\"identity table\"(id bigint GENERATED BY DEFAULT AS IDENTITY,value bytea,CONSTRAINT identity_pk PRIMARY KEY(id)); INSERT INTO t15_verify.\"identity table\" VALUES(1,NULL); COMMENT ON TABLE t15_verify.\"identity table\" IS 'verifier fixture'; SELECT setval(pg_get_serial_sequence('t15_verify.\"identity table\"','id'),2,false)").await.unwrap();
    let mut identity = plan.clone();
    identity.tables[0].target_name = "identity table".into();
    identity.tables[0].columns[0].target_type = "bigint".into();
    identity.tables[0].columns[0].identity = true;
    identity.tables[0].structure.indexes[0].name = "identity_pk".into();
    let mut identity_report = report();
    identity_report.target_name = "identity table".into();
    let unsupported_identity = verify::counts_and_schema(
        &mut source,
        &target,
        &identity,
        &[identity_report.clone()],
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        unsupported_identity.status,
        VerificationStatus::Unsupported,
        "{:?}",
        unsupported_identity.differences
    );
    assert!(
        unsupported_identity
            .differences
            .iter()
            .any(|message| message.contains("sequence"))
    );
    let mut known_identity = identity.clone();
    known_identity.tables[0]
        .structure
        .sequences
        .push(SequenceExpectation {
            column: "id".into(),
            increment: 1,
            min_value: 1,
            max_value: i64::MAX,
            cycle: false,
            next_minimum: 2,
            preserved_state: None,
        });
    let known = verify::counts_and_schema(
        &mut source,
        &target,
        &known_identity,
        &[identity_report.clone()],
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        known.status,
        VerificationStatus::Complete,
        "{:?}",
        known.differences
    );
    target
        .client
        .batch_execute(
            "SELECT setval(pg_get_serial_sequence('t15_verify.\"identity table\"','id'),1,false)",
        )
        .await
        .unwrap();
    assert_eq!(
        verify::counts_and_schema(
            &mut source,
            &target,
            &identity,
            &[identity_report],
            &BTreeMap::new()
        )
        .await
        .unwrap()
        .status,
        VerificationStatus::Different
    );
    target.client.batch_execute("CREATE TABLE t15_verify.parents(id integer PRIMARY KEY); INSERT INTO t15_verify.parents VALUES(1); ALTER TABLE t15_verify.\"renamed bytes\" ADD CONSTRAINT t15_fk FOREIGN KEY(id) REFERENCES t15_verify.parents(id) ON UPDATE CASCADE ON DELETE RESTRICT; CREATE UNIQUE INDEX t15_unique ON t15_verify.\"renamed bytes\"(id DESC)").await.unwrap();
    let mut constraints = plan.clone();
    constraints.tables[0]
        .structure
        .indexes
        .push(IndexExpectation {
            name: "t15_unique".into(),
            columns: vec!["id".into()],
            expressions: vec![None],
            unique: true,
            primary: false,
            descending: vec![true],
        });
    constraints.tables[0]
        .structure
        .foreign_keys
        .push(ForeignKeyExpectation {
            name: "t15_fk".into(),
            columns: vec!["id".into()],
            referenced_schema: "t15_verify".into(),
            referenced_table: "parents".into(),
            referenced_columns: vec!["id".into()],
            on_update: "CASCADE".into(),
            on_delete: "RESTRICT".into(),
        });
    let matching = verify::counts_and_schema(
        &mut source,
        &target,
        &constraints,
        &reports,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        matching.status,
        VerificationStatus::Complete,
        "{:?}",
        matching.differences
    );
    target
        .client
        .batch_execute(
            "ALTER TABLE t15_verify.\"renamed bytes\" ALTER CONSTRAINT t15_fk DEFERRABLE",
        )
        .await
        .unwrap();
    assert_eq!(
        verify::counts_and_schema(
            &mut source,
            &target,
            &constraints,
            &reports,
            &BTreeMap::new()
        )
        .await
        .unwrap()
        .status,
        VerificationStatus::Different
    );
    target.client.batch_execute("ALTER TABLE t15_verify.\"renamed bytes\" ALTER CONSTRAINT t15_fk NOT DEFERRABLE; DROP INDEX t15_verify.t15_unique").await.unwrap();
    let missing_index = verify::counts_and_schema(
        &mut source,
        &target,
        &constraints,
        &reports,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(missing_index.status, VerificationStatus::Different);
    assert!(
        missing_index
            .differences
            .iter()
            .any(|message| message.contains("index"))
    );
    target
        .client
        .batch_execute("ALTER TABLE t15_verify.\"renamed bytes\" DROP CONSTRAINT t15_fk")
        .await
        .unwrap();
    // Equal counts with changed bytes stay within counts/schema scope. This is
    // deliberately not a content-equivalence assertion.
    target
        .client
        .batch_execute("UPDATE t15_verify.\"renamed bytes\" SET value='\\x00'")
        .await
        .unwrap();
    assert_eq!(
        verify::counts_and_schema(&mut source, &target, &plan, &reports, &BTreeMap::new())
            .await
            .unwrap()
            .status,
        VerificationStatus::Complete
    );
    let mut content = plan.clone();
    content.verification = VerificationMode::Content;
    assert_eq!(
        verify::counts_and_schema(&mut source, &target, &content, &reports, &BTreeMap::new())
            .await
            .unwrap()
            .status,
        VerificationStatus::Unsupported
    );
    target
        .client
        .batch_execute("INSERT INTO t15_verify.\"renamed bytes\" VALUES(2,NULL)")
        .await
        .unwrap();
    assert_eq!(
        verify::counts_and_schema(&mut source, &target, &plan, &reports, &BTreeMap::new())
            .await
            .unwrap()
            .status,
        VerificationStatus::Different
    );
    let mut append = plan.clone();
    append.mode = MigrationMode::DataOnly;
    append.on_existing = ExistingPolicy::Append;
    let baseline = BTreeMap::from([(plan.tables[0].id.clone(), 1)]);
    assert_eq!(
        verify::counts_and_schema(&mut source, &target, &append, &reports, &baseline)
            .await
            .unwrap()
            .status,
        VerificationStatus::Complete
    );
    assert_eq!(
        verify::counts_and_schema(&mut source, &target, &append, &reports, &BTreeMap::new())
            .await
            .unwrap()
            .status,
        VerificationStatus::Unsupported
    );
    target.client.batch_execute("DELETE FROM t15_verify.\"renamed bytes\" WHERE id=2; ALTER TABLE t15_verify.\"renamed bytes\" DROP CONSTRAINT t15_pk").await.unwrap();
    let missing_key =
        verify::counts_and_schema(&mut source, &target, &plan, &reports, &BTreeMap::new())
            .await
            .unwrap();
    assert_eq!(missing_key.status, VerificationStatus::Different);
    assert!(
        missing_key
            .differences
            .iter()
            .any(|message| message.contains("primary key"))
    );
    let mut omitted_primary = plan.clone();
    omitted_primary.tables[0].structure.indexes.clear();
    assert_eq!(
        omitted_primary.tables[0].primary_key,
        ["id"],
        "reader identity must remain original"
    );
    let omitted = verify::counts_and_schema(
        &mut source,
        &target,
        &omitted_primary,
        &reports,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        omitted.status,
        VerificationStatus::Complete,
        "explicit absent target PK: {:?}",
        omitted.differences
    );
    target.client.batch_execute("ALTER TABLE t15_verify.\"renamed bytes\" ADD CONSTRAINT t15_pk PRIMARY KEY(id); ALTER TABLE t15_verify.\"renamed bytes\" ADD CONSTRAINT t15_invalid CHECK(id>0) NOT VALID").await.unwrap();
    let invalid =
        verify::counts_and_schema(&mut source, &target, &plan, &reports, &BTreeMap::new())
            .await
            .unwrap();
    assert_eq!(invalid.status, VerificationStatus::Different);
    assert!(
        invalid
            .differences
            .iter()
            .any(|message| message.contains("unvalidated"))
    );
    target.client.batch_execute("ALTER TABLE t15_verify.\"renamed bytes\" DROP CONSTRAINT t15_invalid; ALTER TABLE t15_verify.\"renamed bytes\" DROP COLUMN value").await.unwrap();
    assert_eq!(
        verify::counts_and_schema(&mut source, &target, &plan, &reports, &BTreeMap::new())
            .await
            .unwrap()
            .status,
        VerificationStatus::Different
    );
    target.client.batch_execute("ALTER TABLE t15_verify.\"renamed bytes\" ADD COLUMN value bytea; TRUNCATE t15_verify.\"renamed bytes\"").await.unwrap();
    let mut schema = plan.clone();
    schema.mode = MigrationMode::SchemaOnly;
    let mut empty = report();
    empty.rows_read = 0;
    empty.committed_rows = 0;
    assert_eq!(
        verify::counts_and_schema(&mut source, &target, &schema, &[empty], &BTreeMap::new())
            .await
            .unwrap()
            .status,
        VerificationStatus::Complete
    );
    let mut none = plan.clone();
    none.verification = VerificationMode::None;
    let skipped = verify::counts_and_schema(&mut source, &target, &none, &[], &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(skipped.status, VerificationStatus::NotRun);
    assert_eq!(skipped.tables_checked, 0);
    target
        .client
        .batch_execute("DROP SCHEMA t15_verify CASCADE")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL fixture with synthetic admin"]
async fn verification_uses_the_supplied_single_snapshot_instead_of_a_new_connection() {
    let config:MigrationConfig=serde_json::from_value(serde_json::json!({"version":1,"source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},"target":{"url_env":"MY2PG_POSTGRES_URL","schema":"t15_snapshot","ca_file":required("MY2PG_TLS_CA")}})).unwrap();
    // Source DDL/writes are synthetic fixture setup through an explicit admin.
    // Product verifier uses the separately authenticated read-only connection.
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t15_snapshot")
        .await
        .unwrap();
    admin
        .query_drop("CREATE TABLE source.t15_snapshot(id INT PRIMARY KEY,value BLOB) ENGINE=InnoDB")
        .await
        .unwrap();
    admin
        .query_drop("INSERT INTO source.t15_snapshot VALUES(1,NULL)")
        .await
        .unwrap();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    mysql::start_snapshot(&mut source).await.unwrap();
    admin
        .query_drop("INSERT INTO source.t15_snapshot VALUES(2,NULL)")
        .await
        .unwrap();
    let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute("DROP SCHEMA IF EXISTS t15_snapshot CASCADE; CREATE SCHEMA t15_snapshot; CREATE TABLE t15_snapshot.rows(id integer NOT NULL,value bytea,CONSTRAINT t15_pk PRIMARY KEY(id)); INSERT INTO t15_snapshot.rows VALUES(1,NULL); COMMENT ON TABLE t15_snapshot.rows IS 'verifier fixture'").await.unwrap();
    let mut snapshot = plan();
    snapshot.consistency = Consistency::SingleSnapshot;
    snapshot.target_schema = "t15_snapshot".into();
    snapshot.tables[0].source_name = "t15_snapshot".into();
    snapshot.tables[0].target_schema = "t15_snapshot".into();
    snapshot.tables[0].target_name = "rows".into();
    let mut accounting = report();
    accounting.source_name = "t15_snapshot".into();
    accounting.target_schema = "t15_snapshot".into();
    accounting.target_name = "rows".into();
    let checked = verify::counts_and_schema(
        &mut source,
        &target,
        &snapshot,
        &[accounting.clone()],
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        checked.status,
        VerificationStatus::Complete,
        "{:?}",
        checked.differences
    );
    source.query_drop("COMMIT").await.unwrap();
    let fresh = verify::counts_and_schema(
        &mut source,
        &target,
        &snapshot,
        &[accounting],
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fresh.status, VerificationStatus::Different);
    assert!(
        fresh
            .differences
            .iter()
            .any(|message| message.contains("source count"))
    );
    target
        .client
        .batch_execute("DROP SCHEMA t15_snapshot CASCADE")
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE source.t15_snapshot")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
