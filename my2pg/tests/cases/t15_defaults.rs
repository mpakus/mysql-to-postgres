use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, pipeline, postgres, report::Console, verify};
use mysql_async::prelude::Queryable;
use std::{collections::BTreeMap, env, path::PathBuf};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing fixture variable {name}"))
}
fn config(schema: &str, table: &str) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":schema,"ca_file":required("MY2PG_TLS_CA")},
        "tables":{"include":[table]},
        "report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(schema),"progress":"never"}
    })).unwrap()
}
async fn source(config: &MigrationConfig, admin: bool) -> mysql::SourceConnection {
    mysql::connect(
        &config.source,
        &required(if admin {
            "MY2PG_MYSQL_ROOT_URL"
        } else {
            "MY2PG_MYSQL_URL"
        }),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap()
}
async fn run(config: &MigrationConfig) -> RunReport {
    let args = Args::try_parse_from(["my2pg", "run", "synthetic.toml", "--quiet"]).unwrap();
    let (_sender, receiver) = watch::channel(false);
    crate::test_pipeline::run(
        config,
        &resolve_credentials(config).unwrap(),
        &mut Console::new(config, &args),
        receiver,
    )
    .await
    .unwrap()
}
async fn checked(
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
#[ignore = "requires owned disposable MySQL8.4/PostgreSQL16 TLS fixture"]
async fn planner_pipeline_core_defaults_and_mutations_use_actual_catalog() {
    let mut config = config("t15_core_defaults", "t15_core_defaults");
    config.cast.push(CastRule {
        source_table: Some("t15_core_defaults".into()),
        source_column: Some("flag".into()),
        target_type: Some("boolean".into()),
        transform: Some("tinyint-to-boolean".into()),
        ..Default::default()
    });
    let mut admin = source(&config, true).await;
    admin
        .query_drop("DROP TABLE IF EXISTS source.t15_core_defaults")
        .await
        .unwrap();
    admin.query_drop(r"CREATE TABLE source.t15_core_defaults(
        id INT PRIMARY KEY,
        signed_value INT DEFAULT -3,
        unsigned_value BIGINT UNSIGNED DEFAULT 18446744073709551615,
        exact_value DECIMAL(65,30) DEFAULT 12345678901234567890123456789012345.123456789012345678901234567890,
        float_value FLOAT DEFAULT 0.1,
        double_value DOUBLE DEFAULT 1.7976931348623157e308,
        flag TINYINT(1) DEFAULT 0,
        empty_value VARCHAR(20) DEFAULT '',
        text_value VARCHAR(30) DEFAULT 'it''s::ok',
        bits BIT(8) DEFAULT b'101',
        bytes_value BINARY(4) DEFAULT 0x6100,
        date_value DATE DEFAULT '2024-02-29',
        datetime_value DATETIME(6) DEFAULT '2024-02-29 01:02:03.123456',
        timestamp_value TIMESTAMP(6) DEFAULT '2024-02-29 01:02:03.123456',
        duration TIME(6) DEFAULT '-838:59:58.123456',
        now_zero DATETIME(0) DEFAULT CURRENT_TIMESTAMP(0),
        now_six TIMESTAMP(6) DEFAULT CURRENT_TIMESTAMP(6),
        state ENUM('','a,b','quote''label','slash\\label') DEFAULT 'a,b',
        memberships SET('alpha','NULL','quote''label','slash\\label') DEFAULT 'alpha,NULL'
    ) ENGINE=InnoDB").await.unwrap();
    admin
        .query_drop("INSERT INTO source.t15_core_defaults(id) VALUES(1)")
        .await
        .unwrap();
    let creds = resolve_credentials(&config).unwrap();
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t15_core_defaults CASCADE")
        .await
        .unwrap();
    let plan = pipeline::plan(&config, &creds).await.unwrap();
    assert_eq!(plan.tables.len(), 1);
    let report = run(&config).await;
    assert_eq!(report.exit_code(), 0, "{:?}", report.verification);
    assert_eq!(report.verification.status, VerificationStatus::Complete);
    assert_eq!(report.tables[0].committed_rows, 1);
    let stored: RunReport = serde_json::from_slice(
        &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(stored.verification.status, VerificationStatus::Complete);
    let catalog = target.client.query(
        "SELECT a.attname,pg_catalog.pg_get_expr(d.adbin,d.adrelid) FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE n.nspname='t15_core_defaults' AND c.relname='t15_core_defaults' ORDER BY a.attnum",&[]
    ).await.unwrap();
    assert_eq!(catalog.len(), 18);
    for row in catalog {
        let name: String = row.get(0);
        let expression: String = row.get(1);
        println!("native default {name}: {expression}");
    }
    // Independent native default inserts prove actual values, not just rendering.
    let native = target.client.query_one(
        "INSERT INTO t15_core_defaults.t15_core_defaults(id) VALUES(2) RETURNING unsigned_value::text,exact_value::text,float_value::double precision,double_value,encode(bytes_value,'hex'),bits::text,state::text,memberships,EXTRACT(EPOCH FROM duration)::text",&[]
    ).await.unwrap();
    assert_eq!(native.get::<_, String>(0), "18446744073709551615");
    assert_eq!(
        native.get::<_, String>(1),
        "12345678901234567890123456789012345.123456789012345678901234567890"
    );
    assert_eq!(native.get::<_, f64>(2), 0.10000000149011612);
    assert_eq!(native.get::<_, f64>(3), f64::MAX);
    assert_eq!(native.get::<_, String>(4), "61000000");
    assert_eq!(native.get::<_, String>(5), "00000101");
    assert_eq!(native.get::<_, String>(6), "a,b");
    assert_eq!(native.get::<_, Vec<String>>(7), ["alpha", "NULL"]);
    assert_eq!(native.get::<_, String>(8), "-3020398.123456");
    target
        .client
        .execute(
            "DELETE FROM t15_core_defaults.t15_core_defaults WHERE id=2",
            &[],
        )
        .await
        .unwrap();
    let mut source = source(&config, false).await;
    mysql::start_snapshot(&mut source).await.unwrap();
    for (name, changed) in [
        ("signed_value", "-4"),
        ("unsigned_value", "1"),
        ("exact_value", "2.50"),
        ("float_value", "0.2"),
        ("double_value", "0.2"),
        ("flag", "true"),
        ("empty_value", "'changed'"),
        ("text_value", "'changed'"),
        ("bits", "B'00000100'"),
        ("bytes_value", r"'\xff'::bytea"),
        ("date_value", "'2024-03-01'"),
        ("datetime_value", "'2024-02-29 01:02:04.123456'"),
        ("timestamp_value", "'2024-02-29 01:02:04.123456+00'"),
        ("duration", "'-838:59:57.123456'"),
        ("now_zero", "CURRENT_TIMESTAMP(1)"),
        ("now_six", "CURRENT_TIMESTAMP(0)"),
        ("state", "'quote''label'"),
        ("memberships", "'{NULL}'::text[]"),
    ] {
        target.client.batch_execute(&format!("ALTER TABLE t15_core_defaults.t15_core_defaults ALTER COLUMN {} SET DEFAULT {changed}",postgres::quote_ident(name))).await.unwrap();
        let result = checked(&mut source, &target, &plan, &report).await;
        assert_eq!(
            result.status,
            VerificationStatus::Different,
            "{name}: {:?}",
            result.differences
        );
        let restore = plan.tables[0]
            .columns
            .iter()
            .find(|c| c.target_name == name)
            .unwrap()
            .default_sql
            .as_deref()
            .unwrap();
        target.client.batch_execute(&format!("ALTER TABLE t15_core_defaults.t15_core_defaults ALTER COLUMN {} SET DEFAULT {restore}",postgres::quote_ident(name))).await.unwrap();
    }
    target
        .client
        .batch_execute("SET extra_float_digits=0")
        .await
        .unwrap();
    let rounded = checked(&mut source, &target, &plan, &report).await;
    assert_eq!(
        rounded.status,
        VerificationStatus::Unsupported,
        "{:?}",
        rounded.differences
    );
    target
        .client
        .batch_execute("SET extra_float_digits=1")
        .await
        .unwrap();
    assert_eq!(
        checked(&mut source, &target, &plan, &report).await.status,
        VerificationStatus::Complete
    );
    source.disconnect().await.unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA t15_core_defaults CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
    admin
        .query_drop("DROP TABLE source.t15_core_defaults")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    std::fs::remove_dir_all(report.artifact_dir).unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL8.4/PostgreSQL16 TLS fixture"]
async fn unknown_identical_default_does_not_execute_its_side_effect() {
    let config = config("t15_no_eval", "t15_no_eval");
    let mut admin = source(&config, true).await;
    admin
        .query_drop("DROP TABLE IF EXISTS source.t15_no_eval")
        .await
        .unwrap();
    admin
        .query_drop(
            "CREATE TABLE source.t15_no_eval(id INT PRIMARY KEY,value INT DEFAULT 1) ENGINE=InnoDB",
        )
        .await
        .unwrap();
    admin
        .query_drop("INSERT INTO source.t15_no_eval(id) VALUES(1)")
        .await
        .unwrap();
    let creds = resolve_credentials(&config).unwrap();
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t15_no_eval CASCADE")
        .await
        .unwrap();
    let mut plan = pipeline::plan(&config, &creds).await.unwrap();
    let report = run(&config).await;
    assert_eq!(report.exit_code(), 0);
    target.client.batch_execute(
        "CREATE SEQUENCE t15_no_eval.calls; CREATE FUNCTION t15_no_eval.side_effect() RETURNS integer LANGUAGE SQL VOLATILE AS $$ SELECT nextval('t15_no_eval.calls')::integer $$; ALTER TABLE t15_no_eval.t15_no_eval ALTER COLUMN value SET DEFAULT t15_no_eval.side_effect()"
    ).await.unwrap();
    // Deliberately identical unknown strings must no longer bypass the parser.
    plan.tables[0]
        .columns
        .iter_mut()
        .find(|c| c.target_name == "value")
        .unwrap()
        .default_sql = Some("t15_no_eval.side_effect()".into());
    let mut source = source(&config, false).await;
    mysql::start_snapshot(&mut source).await.unwrap();
    let before = target
        .client
        .query_one("SELECT last_value,is_called FROM t15_no_eval.calls", &[])
        .await
        .unwrap();
    let result = checked(&mut source, &target, &plan, &report).await;
    assert_eq!(
        result.status,
        VerificationStatus::Unsupported,
        "{:?}",
        result.differences
    );
    let after = target
        .client
        .query_one("SELECT last_value,is_called FROM t15_no_eval.calls", &[])
        .await
        .unwrap();
    assert_eq!(before.get::<_, i64>(0), after.get::<_, i64>(0));
    assert!(!before.get::<_, bool>(1));
    assert!(!after.get::<_, bool>(1));
    source.disconnect().await.unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA t15_no_eval CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
    admin
        .query_drop("DROP TABLE source.t15_no_eval")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    std::fs::remove_dir_all(report.artifact_dir).unwrap();
}
