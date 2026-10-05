use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, pipeline, postgres, report::Console, verify};
use mysql_async::prelude::Queryable;
use std::{collections::BTreeMap, env, path::PathBuf};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing fixture {name}"))
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
#[ignore = "requires owned disposable MySQL8.4/PostgreSQL16 TLS fixture"]
async fn complete_supported_defaults_survive_native_styles_identity_and_mutations() {
    let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"single_snapshot","ca_file":required("MY2PG_TLS_CA")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":"t15_complete","ca_file":required("MY2PG_TLS_CA")},
        "tables":{"include":["t15_complete"]},
        "report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t15_complete"),"progress":"never"}
    })).unwrap();
    for (name, target) in [
        ("payload", "jsonb"),
        ("reserved", "jsonb"),
        ("huge", "jsonb"),
        ("tiny", "jsonb"),
        ("ip6", "inet"),
        ("ip4", "inet"),
    ] {
        config.cast.push(CastRule {
            source_table: Some("t15_complete".into()),
            source_column: Some(name.into()),
            target_type: Some(target.into()),
            ..Default::default()
        });
    }
    let creds = resolve_credentials(&config).unwrap();
    let mut admin = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t15_complete")
        .await
        .unwrap();
    admin.query_drop(r#"CREATE TABLE source.t15_complete(
        id INT PRIMARY KEY,
        payload VARCHAR(1000) DEFAULT '{"z":0,"a":[1.00,1e3,null,true,"\\ud83d\\ude00"],"z":2,"n":12345678901234567890.123456789012345678901234567890}',
        reserved VARCHAR(100) DEFAULT '{"$serde_json::private::Number":"1"}',
        huge VARCHAR(30) DEFAULT '1e131071',
        tiny VARCHAR(30) DEFAULT '1e-16383',
        ip6 VARCHAR(100) DEFAULT '2001:0DB8:0000:0000:0000:0000:0000:0001/64',
        ip4 VARCHAR(100) DEFAULT '192.000.002.129/24',
        bytes_value BINARY(5) DEFAULT 'a',
        duration TIME(6) DEFAULT '-838:59:58.123456',
        positive TIME(6) DEFAULT '12:34:56.123456',
        zero_time TIME(6) DEFAULT '00:00:00',
        now0 DATETIME(0) DEFAULT CURRENT_TIMESTAMP(0),
        now6 TIMESTAMP(6) DEFAULT CURRENT_TIMESTAMP(6),
        state ENUM('red','green') DEFAULT 'green'
    ) ENGINE=InnoDB"#).await.unwrap();
    admin
        .query_drop("INSERT INTO source.t15_complete(id) VALUES(1)")
        .await
        .unwrap();
    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target.client.batch_execute("DROP SCHEMA IF EXISTS t15_complete CASCADE; DROP SCHEMA IF EXISTS t15_impostor CASCADE").await.unwrap();
    let plan = pipeline::plan(&config, &creds).await.unwrap();
    let args = Args::try_parse_from(["my2pg", "run", "synthetic.toml", "--quiet"]).unwrap();
    let (_sender, receiver) = watch::channel(false);
    let report =
        crate::test_pipeline::run(&config, &creds, &mut Console::new(&config, &args), receiver)
            .await
            .unwrap();
    assert_eq!(report.exit_code(), 0, "{:?}", report.verification);
    assert_eq!(report.verification.status, VerificationStatus::Complete);
    let saved: RunReport = serde_json::from_slice(
        &std::fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved.verification.status, VerificationStatus::Complete);
    let native=target.client.query_one(
        "INSERT INTO t15_complete.t15_complete(id) VALUES(2) RETURNING payload->>'n',payload->>'z',reserved->>'$serde_json::private::Number',length(huge::text),length(tiny::text),ip6::text,ip4::text,encode(bytes_value,'hex')",&[]
    ).await.unwrap();
    assert_eq!(
        native.get::<_, String>(0),
        "12345678901234567890.123456789012345678901234567890"
    );
    assert_eq!(native.get::<_, String>(1), "2");
    assert_eq!(native.get::<_, String>(2), "1");
    assert_eq!(native.get::<_, i32>(3), 131072);
    assert_eq!(native.get::<_, i32>(4), 16385);
    assert_eq!(native.get::<_, String>(5), "2001:db8::1/64");
    assert_eq!(native.get::<_, String>(6), "192.0.2.129/24");
    assert_eq!(native.get::<_, String>(7), "6100000000");
    target
        .client
        .execute("DELETE FROM t15_complete.t15_complete WHERE id=2", &[])
        .await
        .unwrap();
    let mut source = mysql::connect(
        &config.source,
        creds.source.expose(),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    mysql::start_snapshot(&mut source).await.unwrap();
    // Explicit typed-literal catalog proof includes bytes whose MySQL metadata
    // default is currently truncated at NUL; that source fidelity gate is separate.
    let mut escaped_plan = plan.clone();
    escaped_plan.tables[0]
        .columns
        .iter_mut()
        .find(|column| column.target_name == "bytes_value")
        .unwrap()
        .default_sql = Some(r"E'\\x00095c7fff'".into());
    target.client.batch_execute(r"ALTER TABLE t15_complete.t15_complete ALTER COLUMN bytes_value SET DEFAULT '\x00095c7fff'::bytea").await.unwrap();
    for style in ["postgres", "postgres_verbose", "sql_standard", "iso_8601"] {
        for binary in ["hex", "escape"] {
            target.client.batch_execute(&format!("SET IntervalStyle={style}; SET bytea_output={binary}; SET search_path=t15_complete,pg_catalog")).await.unwrap();
            let result = check(&mut source, &target, &escaped_plan, &report).await;
            assert_eq!(
                result.status,
                VerificationStatus::Complete,
                "{style}/{binary}: {:?}",
                result.differences
            );
        }
    }
    target
        .client
        .batch_execute("SET IntervalStyle=postgres; SET bytea_output=hex")
        .await
        .unwrap();
    let original_binary = plan.tables[0]
        .columns
        .iter()
        .find(|column| column.target_name == "bytes_value")
        .unwrap()
        .default_sql
        .as_deref()
        .unwrap();
    target.client.batch_execute(&format!("ALTER TABLE t15_complete.t15_complete ALTER COLUMN bytes_value SET DEFAULT {original_binary}")).await.unwrap();
    for (name, changed) in [
        (
            "payload",
            r#"'{"z":2,"a":[1,1000,null,true,"😀"],"n":12345678901234567890.12345678901234567890123456789}'"#,
        ),
        ("reserved", r#"'{ "$serde_json::private::Number": "1" }'"#),
        ("ip6", "'2001:db8::1/64'"),
        ("ip4", "'192.0.2.129/24'"),
        ("now0", "CURRENT_TIMESTAMP(0)"),
        ("now6", "CURRENT_TIMESTAMP"),
    ] {
        target
            .client
            .batch_execute(&format!(
                "ALTER TABLE t15_complete.t15_complete ALTER COLUMN {} SET DEFAULT {changed}",
                postgres::quote_ident(name)
            ))
            .await
            .unwrap();
        let result = check(&mut source, &target, &plan, &report).await;
        assert_eq!(
            result.status,
            VerificationStatus::Complete,
            "equivalent {name}: {:?}",
            result.differences
        );
    }
    for (name, changed) in [
        (
            "payload",
            r#"'{"z":2,"a":[1000,1,null,true,"😀"],"n":12345678901234567890.12345678901234567890123456789}'"#,
        ),
        ("reserved", "'1'"),
        ("huge", "'1e131070'"),
        ("tiny", "'1e-16382'"),
        ("ip6", "'2001:db8::2/64'"),
        ("ip4", "'192.0.2.129/32'"),
        ("bytes_value", r"'\x00095c7ffe'::bytea"),
        ("duration", "'-838:59:57.123456'"),
        ("positive", "'12:34:55.123456'"),
        ("zero_time", "'00:00:01'"),
        ("now0", "CURRENT_TIMESTAMP(3)"),
        ("now6", "CURRENT_TIMESTAMP(0)"),
        ("state", "'red'"),
    ] {
        target
            .client
            .batch_execute(&format!(
                "ALTER TABLE t15_complete.t15_complete ALTER COLUMN {} SET DEFAULT {changed}",
                postgres::quote_ident(name)
            ))
            .await
            .unwrap();
        let result = check(&mut source, &target, &plan, &report).await;
        assert_eq!(
            result.status,
            VerificationStatus::Different,
            "changed {name}: {:?}",
            result.differences
        );
        let restore = plan.tables[0]
            .columns
            .iter()
            .find(|column| column.target_name == name)
            .unwrap()
            .default_sql
            .as_deref()
            .unwrap();
        target
            .client
            .batch_execute(&format!(
                "ALTER TABLE t15_complete.t15_complete ALTER COLUMN {} SET DEFAULT {restore}",
                postgres::quote_ident(name)
            ))
            .await
            .unwrap();
    }
    // Different same-label enum type under a shadowing search_path must differ.
    let enum_name:String=target.client.query_one("SELECT t.typname::text FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='t15_complete' AND t.typtype='e'",&[]).await.unwrap().get(0);
    let impostor = postgres::qualified("t15_impostor", &enum_name);
    target.client.batch_execute(&format!("CREATE SCHEMA t15_impostor; CREATE TYPE {impostor} AS ENUM('red','green'); SET search_path=t15_impostor,t15_complete,pg_catalog")).await.unwrap();
    assert_eq!(
        check(&mut source, &target, &plan, &report).await.status,
        VerificationStatus::Complete
    );
    target.client.batch_execute(&format!("ALTER TABLE t15_complete.t15_complete ALTER COLUMN state DROP DEFAULT; ALTER TABLE t15_complete.t15_complete ALTER COLUMN state TYPE {impostor} USING state::text::{impostor}; ALTER TABLE t15_complete.t15_complete ALTER COLUMN state SET DEFAULT 'green'")).await.unwrap();
    let wrong = check(&mut source, &target, &plan, &report).await;
    assert_eq!(
        wrong.status,
        VerificationStatus::Different,
        "{:?}",
        wrong.differences
    );
    let original = &plan.tables[0]
        .columns
        .iter()
        .find(|column| column.target_name == "state")
        .unwrap()
        .target_type;
    target.client.batch_execute(&format!("ALTER TABLE t15_complete.t15_complete ALTER COLUMN state DROP DEFAULT; ALTER TABLE t15_complete.t15_complete ALTER COLUMN state TYPE {original} USING state::text::{original}; ALTER TABLE t15_complete.t15_complete ALTER COLUMN state SET DEFAULT 'green'")).await.unwrap();
    // JSON target storage retains original text/order/duplicates, unlike JSONB.
    let mut text_plan = plan.clone();
    let json = text_plan.tables[0]
        .columns
        .iter_mut()
        .find(|column| column.target_name == "payload")
        .unwrap();
    json.target_type = "json".into();
    target.client.batch_execute(&format!("ALTER TABLE t15_complete.t15_complete ALTER COLUMN payload DROP DEFAULT; ALTER TABLE t15_complete.t15_complete ALTER COLUMN payload TYPE json USING payload::json; ALTER TABLE t15_complete.t15_complete ALTER COLUMN payload SET DEFAULT {}",json.default_sql.as_deref().unwrap())).await.unwrap();
    assert_eq!(
        check(&mut source, &target, &text_plan, &report)
            .await
            .status,
        VerificationStatus::Complete
    );
    target.client.batch_execute(r#"ALTER TABLE t15_complete.t15_complete ALTER COLUMN payload SET DEFAULT '{"z":2,"a":[1,1000,null,true,"😀"],"n":12345678901234567890.12345678901234567890123456789}'"#).await.unwrap();
    assert_eq!(
        check(&mut source, &target, &text_plan, &report)
            .await
            .status,
        VerificationStatus::Different
    );
    source.disconnect().await.unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA t15_complete CASCADE; DROP SCHEMA t15_impostor CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
    admin
        .query_drop("DROP TABLE source.t15_complete")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    std::fs::remove_dir_all(report.artifact_dir).unwrap();
}
