use bytes::Bytes;
use futures_util::{SinkExt, TryStreamExt};
use my2pg::{config::*, convert, model::*, mysql, plan};
use std::env;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4 and PostgreSQL16 TLS fixtures"]
async fn pure_plan_and_text_encoder_execute_real_relational_fixture() {
    let mut config: MigrationConfig =
        toml::from_str(include_str!("../contracts/config.toml")).unwrap();
    config.tables.rename.clear();
    config.tables.include = [
        "users",
        "orders",
        "empty_table",
        "keyless",
        "CamelCase",
        "numeric_edges",
        "all_bytes",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    config.source.ca_file = Some(required("MY2PG_TLS_CA").into());
    config.target.schema = "t06_acceptance".into();
    config.target.ca_file = config.source.ca_file.clone();
    config.overrides.push(ObjectOverride {
        object: "source.orders.label.collation".into(),
        omit: true,
        target_expression: None,
        target_sql: None,
        materialize: None,
    });
    let mut source_connection = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    let source = mysql::inspect(&mut source_connection).await.unwrap();
    let cert = native_tls::Certificate::from_pem(&std::fs::read(required("MY2PG_TLS_CA")).unwrap())
        .unwrap();
    let tls = native_tls::TlsConnector::builder()
        .add_root_certificate(cert)
        .build()
        .unwrap();
    let mut pg_config: tokio_postgres::Config = required("MY2PG_POSTGRES_URL").parse().unwrap();
    pg_config.ssl_mode(tokio_postgres::config::SslMode::Require);
    let (mut postgres, connection) = pg_config
        .connect(postgres_native_tls::MakeTlsConnector::new(tls))
        .await
        .unwrap();
    let task = tokio::spawn(connection);
    let target = my2pg::postgres::inspect(&mut postgres, &config.target.schema)
        .await
        .unwrap();
    let resolved = plan::build(&config, &source, &target).unwrap();
    assert_eq!(resolved.tables.len(), 7);
    for step in resolved
        .ddl
        .iter()
        .filter(|step| step.phase == DdlPhase::Prepare)
    {
        postgres.batch_execute(&step.sql).await.unwrap();
    }
    for table in &resolved.tables {
        let transaction = postgres.transaction().await.unwrap();
        let columns = table
            .columns
            .iter()
            .filter(|column| column.copy)
            .map(|column| plan::quote_identifier(&column.target_name))
            .collect::<Vec<_>>()
            .join(",");
        let copy = format!(
            "COPY {} ({columns}) FROM STDIN WITH (FORMAT text)",
            plan::qualified_name(&table.target_schema, &table.target_name)
        );
        let sink = transaction.copy_in(&copy).await.unwrap();
        tokio::pin!(sink);
        let mut rows = mysql::table_stream(&mut source_connection, &source.database, table)
            .await
            .unwrap();
        let mut count = 0;
        while let Some(row) = rows.try_next().await.unwrap() {
            let values = mysql::raw_values(row, config.migration.max_row_bytes).unwrap();
            sink.as_mut()
                .send(Bytes::from(
                    convert::encode_row_limited(table, &values, config.migration.max_row_bytes)
                        .unwrap(),
                ))
                .await
                .unwrap();
            count += 1;
        }
        drop(rows);
        assert_eq!(sink.as_mut().finish().await.unwrap(), count);
        transaction.commit().await.unwrap();
    }
    for step in resolved
        .ddl
        .iter()
        .filter(|step| step.phase != DdlPhase::Prepare)
    {
        postgres.batch_execute(&step.sql).await.unwrap();
    }
    let users = postgres
        .query(
            "SELECT id,name,note,payload,amount::text FROM t06_acceptance.users ORDER BY id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(users.len(), 3);
    assert_eq!(users[0].get::<_, Option<String>>(2), None);
    assert_eq!(users[0].get::<_, Vec<u8>>(3), vec![0, 1, 92, 9, 255]);
    assert_eq!(
        users[0].get::<_, String>(4),
        "1234567890123456789012.12345678"
    );
    assert_eq!(users[1].get::<_, String>(1), "emoji 😀");
    assert_eq!(users[1].get::<_, String>(2), "");
    assert!(users[1].get::<_, Vec<u8>>(3).is_empty());
    assert_eq!(
        users[2].get::<_, String>(2),
        "literal \\N\ttab\nline\rreturn"
    );
    let bytes: Vec<u8> = postgres
        .query_one("SELECT value FROM t06_acceptance.all_bytes", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(bytes, (0..=255).collect::<Vec<u8>>());
    let edge = postgres.query_one("SELECT unsigned_big::text, unsigned_int, bits::text, EXTRACT(EPOCH FROM duration)::text, value_json->>'large' FROM t06_acceptance.numeric_edges WHERE id=1", &[]).await.unwrap();
    assert_eq!(edge.get::<_, String>(0), "18446744073709551615");
    assert_eq!(edge.get::<_, i64>(1), 4_294_967_295);
    assert_eq!(edge.get::<_, String>(2), "00000101");
    assert_eq!(edge.get::<_, String>(3), "-3020398.999999");
    assert_eq!(edge.get::<_, String>(4), "18446744073709551615");
    let generated: i32 = postgres.query_one("INSERT INTO t06_acceptance.users(name,amount,created_at) VALUES ('identity',1,'2024-01-01') RETURNING id", &[]).await.unwrap().get(0);
    assert_eq!(generated, 42);
    let fk_error = postgres
        .execute(
            "INSERT INTO t06_acceptance.orders VALUES (123456,1,'invalid')",
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(fk_error.code().unwrap().code(), "23503");
    let unique_error = postgres
        .execute(
            "INSERT INTO t06_acceptance.orders VALUES (1,99,'first')",
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(unique_error.code().unwrap().code(), "23505");
    let primary_error = postgres
        .execute(
            "INSERT INTO t06_acceptance.orders VALUES (1,2,'different')",
            &[],
        )
        .await
        .unwrap_err();
    assert_eq!(primary_error.code().unwrap().code(), "23505");
    let empty = postgres
        .query_one(
            "INSERT INTO t06_acceptance.empty_table DEFAULT VALUES RETURNING id,value",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(empty.get::<_, i32>(0), 1);
    assert_eq!(empty.get::<_, String>(1), "");
    let duplicates: i64 = postgres
        .query_one(
            "SELECT COUNT(*) FROM t06_acceptance.keyless WHERE value='same' AND qty=2",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(duplicates, 2);
    let sentinel: String = postgres
        .query_one("SELECT sentinel FROM public.users WHERE id=999", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sentinel, "must-survive");
    source_connection.disconnect().await.unwrap();
    drop(postgres);
    task.await.unwrap().unwrap();
}
