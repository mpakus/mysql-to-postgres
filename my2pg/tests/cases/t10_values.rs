use bytes::Bytes;
use futures_util::{SinkExt, TryStreamExt};
use my2pg::{config::*, convert, model::*, mysql, plan};
use mysql_async::prelude::Queryable;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}

async fn target() -> (
    tokio_postgres::Client,
    tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
) {
    let cert = native_tls::Certificate::from_pem(&std::fs::read(required("MY2PG_TLS_CA")).unwrap())
        .unwrap();
    let tls = native_tls::TlsConnector::builder()
        .add_root_certificate(cert)
        .build()
        .unwrap();
    let mut config: tokio_postgres::Config = required("MY2PG_POSTGRES_URL").parse().unwrap();
    config.ssl_mode(tokio_postgres::config::SslMode::Require);
    let (client, connection) = config
        .connect(postgres_native_tls::MakeTlsConnector::new(tls))
        .await
        .unwrap();
    (client, tokio::spawn(connection))
}

async fn migrate(
    config: &MigrationConfig,
    source: &mut mysql::SourceConnection,
    pg: &mut tokio_postgres::Client,
) -> MigrationPlan {
    let catalog = mysql::inspect(source).await.unwrap();
    let target = my2pg::postgres::inspect(pg, &config.target.schema)
        .await
        .unwrap();
    let resolved = plan::build(config, &catalog, &target).unwrap();
    for step in resolved
        .ddl
        .iter()
        .filter(|step| step.phase == DdlPhase::Prepare)
    {
        pg.batch_execute(&step.sql).await.unwrap();
    }
    for table in &resolved.tables {
        let transaction = pg.transaction().await.unwrap();
        let columns = table
            .columns
            .iter()
            .filter(|column| column.copy)
            .map(|column| plan::quote_identifier(&column.target_name))
            .collect::<Vec<_>>()
            .join(",");
        let sink = transaction
            .copy_in(&format!(
                "COPY {} ({columns}) FROM STDIN WITH (FORMAT text)",
                plan::qualified_name(&table.target_schema, &table.target_name)
            ))
            .await
            .unwrap();
        tokio::pin!(sink);
        let mut stream = mysql::table_stream(source, &catalog.database, table)
            .await
            .unwrap();
        let mut count = 0;
        while let Some(row) = stream.try_next().await.unwrap() {
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
        drop(stream);
        assert_eq!(sink.as_mut().finish().await.unwrap(), count);
        transaction.commit().await.unwrap();
    }
    for step in resolved
        .ddl
        .iter()
        .filter(|step| step.phase != DdlPhase::Prepare)
    {
        pg.batch_execute(&step.sql).await.unwrap();
    }
    resolved
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4 and PostgreSQL16 TLS fixtures"]
async fn native_numeric_temporal_enum_set_charsets_and_defaults_roundtrip() {
    let mut config: MigrationConfig =
        toml::from_str(include_str!("../contracts/config.toml")).unwrap();
    config.tables.rename.clear();
    config.tables.include = vec![
        "t10_values".into(),
        "t10_choices".into(),
        "t10_transforms".into(),
        "t10_unsigned".into(),
    ];
    config.source.ca_file = Some(required("MY2PG_TLS_CA").into());
    config.target.ca_file = config.source.ca_file.clone();
    config.target.schema = "t10_values_acceptance".into();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    source
        .query_drop(
            "DROP TABLE IF EXISTS source.t10_values,source.t10_choices,source.t10_transforms,source.t10_unsigned",
        )
        .await
        .unwrap();
    source.query_drop("CREATE TABLE source.t10_values(id INT PRIMARY KEY,tiny TINYINT,small SMALLINT,medium MEDIUMINT,int_value INT,big BIGINT,unsigned_big BIGINT UNSIGNED,exact_decimal DECIMAL(65,30),float_value FLOAT DEFAULT 0.1,double_value DOUBLE DEFAULT 1.7976931348623157e308,bits BIT(64) DEFAULT b'101',date_value DATE DEFAULT '2024-02-29',datetime_value DATETIME(6) DEFAULT '2024-02-29 01:02:03.123456',timestamp_value TIMESTAMP(6) DEFAULT '2024-02-29 01:02:03.123456',duration TIME(6) DEFAULT '-838:59:59',year_value YEAR DEFAULT 0,bytes_value BINARY(4) DEFAULT 'a',json_value JSON,latin_value VARCHAR(3) CHARACTER SET latin1 DEFAULT '€ÿ') ENGINE=InnoDB").await.unwrap();
    source.query_drop("CREATE TABLE source.t10_unsigned(id INT PRIMARY KEY,tiny TINYINT UNSIGNED,small SMALLINT UNSIGNED,medium MEDIUMINT UNSIGNED,int_value INT UNSIGNED,integer_truth TINYINT(1) NOT NULL DEFAULT 2) ENGINE=InnoDB").await.unwrap();
    source.query_drop("INSERT INTO source.t10_unsigned VALUES(1,255,65535,16777215,4294967295,-1),(2,0,0,0,0,2)").await.unwrap();
    let decimal = format!("{}.{}", "9".repeat(35), "9".repeat(30));
    source.exec_drop("INSERT INTO source.t10_values VALUES(1,-128,-32768,-8388608,-2147483648,-9223372036854775808,18446744073709551615,?,0.1,0.1,18446744073709551615,'2024-02-29','2024-03-10 02:30:00.123456','2024-02-29 01:02:03.123456','-838:59:59',2155,X'00FF5C',JSON_OBJECT('huge',18446744073709551615,'emoji','😀'),CONVERT(X'8081FF' USING latin1))",(&decimal,)).await.unwrap();
    source.exec_drop("INSERT INTO source.t10_values VALUES(2,127,32767,8388607,2147483647,9223372036854775807,0,?,3.4028234663852886e38,1.7976931348623157e308,b'1','1000-01-01','9999-12-31 23:59:59.999999','2038-01-19 03:14:07','838:59:59',1901,X'',JSON_EXTRACT('null','$'),'')",(&format!("-{}.{}","9".repeat(35),"9".repeat(30)),)).await.unwrap();
    let labels = (0..64)
        .map(|i| format!("'label{i}'"))
        .collect::<Vec<_>>()
        .join(",");
    source.query_drop(format!("CREATE TABLE source.t10_choices(id INT PRIMARY KEY,state ENUM('','a,b','quote''label','slash\\\\label') DEFAULT '', memberships SET({labels}) DEFAULT 'label0,label63') ENGINE=InnoDB")).await.unwrap();
    source.query_drop("INSERT INTO source.t10_choices VALUES(1,'',9223372036854775809),(2,'a,b',0),(3,'quote''label',NULL),(4,'slash\\\\label',1)").await.unwrap();
    source.query_drop("CREATE TABLE source.t10_transforms(id INT PRIMARY KEY,trimmed TEXT,cleaned TEXT,empty_text TEXT,truth TINYINT,bit_truth BIT(1),hex_bytes TEXT,ip INT UNSIGNED,year_number YEAR,tiny_number TINYINT,raw_bytes BLOB) ENGINE=InnoDB").await.unwrap();
    source.query_drop("INSERT INTO source.t10_transforms VALUES(1,'trailing  ',CONCAT('a',CHAR(0),'b'),'',-1,b'1','00FF5c',2130706433,2155,-128,X'00FF5C'),(2,'plain','plain',NULL,0,b'0','',0,0,127,X'')").await.unwrap();
    for (name, target, transform) in [
        ("trimmed", "text", "right-trim"),
        ("cleaned", "text", "remove-null-characters"),
        ("empty_text", "text", "empty-string-to-null"),
        ("truth", "boolean", "tinyint-to-boolean"),
        ("bit_truth", "boolean", "bits-to-boolean"),
        ("hex_bytes", "bytea", "hex-to-bytea"),
        ("ip", "inet", "int-to-ip"),
        ("year_number", "smallint", "year-to-integer"),
        ("tiny_number", "smallint", "tinyint-to-integer"),
        ("raw_bytes", "bytea", "byte-vector-to-bytea"),
    ] {
        config.cast.push(CastRule {
            source_table: Some("t10_transforms".into()),
            source_column: Some(name.into()),
            target_type: Some(target.into()),
            transform: Some(transform.into()),
            ..Default::default()
        });
    }
    let (mut pg, task) = target().await;
    pg.batch_execute("DROP SCHEMA IF EXISTS t10_values_acceptance CASCADE")
        .await
        .unwrap();
    let resolved = migrate(&config, &mut source, &mut pg).await;
    let unsigned=pg.query_one("SELECT tiny,small,medium,int_value,integer_truth FROM t10_values_acceptance.t10_unsigned WHERE id=1",&[]).await.unwrap();
    assert_eq!(unsigned.get::<_, i16>(0), 255);
    assert_eq!(unsigned.get::<_, i32>(1), 65535);
    assert_eq!(unsigned.get::<_, i32>(2), 16777215);
    assert_eq!(unsigned.get::<_, i64>(3), 4294967295);
    assert_eq!(unsigned.get::<_, i16>(4), -1);
    let zero=pg.query_one("SELECT tiny,small,medium,int_value,integer_truth FROM t10_values_acceptance.t10_unsigned WHERE id=2",&[]).await.unwrap();
    assert_eq!(zero.get::<_, i16>(0), 0);
    assert_eq!(zero.get::<_, i32>(1), 0);
    assert_eq!(zero.get::<_, i32>(2), 0);
    assert_eq!(zero.get::<_, i64>(3), 0);
    assert_eq!(zero.get::<_, i16>(4), 2);
    let truth_default = pg
        .query_one(
            "INSERT INTO t10_values_acceptance.t10_unsigned(id) VALUES(3) RETURNING integer_truth",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(truth_default.get::<_, i16>(0), 2);
    let cp1252_bytes = (1..=255).collect::<Vec<u8>>();
    let hex = cp1252_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let native_charset: String = source
        .query_first(format!(
            "SELECT CONVERT(CONVERT(X'{hex}' USING latin1) USING utf8mb4)"
        ))
        .await
        .unwrap()
        .unwrap();
    let mut charset_column = resolved
        .tables
        .iter()
        .find(|table| table.source_name == "t10_values")
        .unwrap()
        .columns
        .iter()
        .find(|column| column.source_name == "latin_value")
        .unwrap()
        .clone();
    charset_column.target_type = "text".into();
    assert_eq!(
        convert::encode_value(&charset_column, &RawValue::Bytes(cp1252_bytes))
            .unwrap()
            .as_deref(),
        Some(native_charset.as_str())
    );
    let first=pg.query_one("SELECT tiny,small,medium,int_value,big,unsigned_big::text,exact_decimal::text,float_value::double precision,double_value,bits::text,date_value::text,datetime_value::text,EXTRACT(EPOCH FROM timestamp_value)::text,EXTRACT(EPOCH FROM duration)::text,year_value,encode(bytes_value,'hex'),json_value->>'huge',latin_value FROM t10_values_acceptance.t10_values WHERE id=1",&[]).await.unwrap();
    assert_eq!(first.get::<_, i16>(0), -128);
    assert_eq!(first.get::<_, i16>(1), -32768);
    assert_eq!(first.get::<_, i32>(2), -8388608);
    assert_eq!(first.get::<_, i32>(3), i32::MIN);
    assert_eq!(first.get::<_, i64>(4), i64::MIN);
    assert_eq!(first.get::<_, String>(5), "18446744073709551615");
    assert_eq!(first.get::<_, String>(6), decimal);
    assert_eq!(first.get::<_, f64>(7), 0.10000000149011612);
    assert_eq!(first.get::<_, f64>(8), 0.1);
    assert_eq!(first.get::<_, String>(9), "1".repeat(64));
    assert_eq!(first.get::<_, String>(10), "2024-02-29");
    assert_eq!(first.get::<_, String>(11), "2024-03-10 02:30:00.123456");
    assert_eq!(first.get::<_, String>(12), "1709168523.123456");
    assert_eq!(first.get::<_, String>(13), "-3020399.000000");
    assert_eq!(first.get::<_, i16>(14), 2155);
    assert_eq!(first.get::<_, String>(15), "00ff5c00");
    assert_eq!(first.get::<_, String>(16), "18446744073709551615");
    assert_eq!(first.get::<_, String>(17), "€\u{81}ÿ");
    let json_semantics=pg.query_one("SELECT value.json_value->>'emoji',other.json_value IS NULL,other.json_value='null'::jsonb FROM t10_values_acceptance.t10_values value,t10_values_acceptance.t10_values other WHERE value.id=1 AND other.id=2",&[]).await.unwrap();
    assert_eq!(json_semantics.get::<_, String>(0), "😀");
    assert!(!json_semantics.get::<_, bool>(1));
    assert!(json_semantics.get::<_, bool>(2));
    let json_column = resolved
        .tables
        .iter()
        .find(|table| table.source_name == "t10_values")
        .unwrap()
        .columns
        .iter()
        .find(|column| column.source_name == "json_value")
        .unwrap();
    for numeric in ["1e131071", "1e-16383"] {
        convert::encode_value(json_column, &RawValue::Bytes(numeric.as_bytes().to_vec())).unwrap();
        let native: String = pg
            .query_one("SELECT ($1::text::jsonb)::text", &[&numeric])
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            native.len(),
            if numeric == "1e131071" { 131072 } else { 16385 }
        );
    }
    for numeric in ["1e131072", "1e-16384"] {
        assert!(
            convert::encode_value(json_column, &RawValue::Bytes(numeric.as_bytes().to_vec()))
                .is_err()
        );
        let rejected = pg
            .query_one("SELECT $1::text::jsonb", &[&numeric])
            .await
            .unwrap_err();
        assert_eq!(rejected.code().unwrap().code(), "22003");
    }
    let second=pg.query_one("SELECT tiny,small,medium,int_value,big,float_value,double_value FROM t10_values_acceptance.t10_values WHERE id=2",&[]).await.unwrap();
    assert_eq!(second.get::<_, i16>(0), 127);
    assert_eq!(second.get::<_, i16>(1), 32767);
    assert_eq!(second.get::<_, i32>(2), 8388607);
    assert_eq!(second.get::<_, i32>(3), i32::MAX);
    assert_eq!(second.get::<_, i64>(4), i64::MAX);
    assert_eq!(second.get::<_, f32>(5), f32::MAX);
    assert_eq!(second.get::<_, f64>(6), f64::MAX);
    let choices = pg
        .query(
            "SELECT state::text,memberships FROM t10_values_acceptance.t10_choices ORDER BY id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(choices[0].get::<_, String>(0), "");
    assert_eq!(choices[0].get::<_, Vec<String>>(1), ["label0", "label63"]);
    assert_eq!(choices[1].get::<_, String>(0), "a,b");
    assert!(choices[1].get::<_, Vec<String>>(1).is_empty());
    assert_eq!(choices[2].get::<_, String>(0), "quote'label");
    assert_eq!(choices[2].get::<_, Option<Vec<String>>>(1), None);
    assert_eq!(choices[3].get::<_, String>(0), "slash\\label");
    let enum_order:Vec<String>=pg.query("SELECT e.enumlabel::text FROM pg_catalog.pg_enum e JOIN pg_catalog.pg_type t ON t.oid=e.enumtypid JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='t10_values_acceptance' ORDER BY e.enumsortorder",&[]).await.unwrap().into_iter().map(|row|row.get(0)).collect();
    assert_eq!(enum_order, ["", "a,b", "quote'label", "slash\\label"]);
    let membership_error=pg.execute("INSERT INTO t10_values_acceptance.t10_choices(id,memberships) VALUES(99,ARRAY['outside'])",&[]).await.unwrap_err();
    assert_eq!(membership_error.code().unwrap().code(), "23514");
    let defaults_choices=pg.query_one("INSERT INTO t10_values_acceptance.t10_choices(id) VALUES(99) RETURNING state::text,memberships",&[]).await.unwrap();
    assert_eq!(defaults_choices.get::<_, String>(0), "");
    assert_eq!(
        defaults_choices.get::<_, Vec<String>>(1),
        ["label0", "label63"]
    );
    let defaults=pg.query_one("INSERT INTO t10_values_acceptance.t10_values(id) VALUES(3) RETURNING bits::text,date_value::text,datetime_value::text,year_value,encode(bytes_value,'hex'),latin_value",&[]).await.unwrap();
    assert_eq!(
        defaults.get::<_, String>(0),
        format!("{}101", "0".repeat(61))
    );
    assert_eq!(defaults.get::<_, String>(1), "2024-02-29");
    assert_eq!(defaults.get::<_, String>(2), "2024-02-29 01:02:03.123456");
    assert_eq!(defaults.get::<_, i16>(3), 0);
    source
        .query_drop("INSERT INTO source.t10_values(id) VALUES(3)")
        .await
        .unwrap();
    let native_default: Option<String> = source
        .query_first("SELECT HEX(bytes_value) FROM source.t10_values WHERE id=3")
        .await
        .unwrap();
    assert_eq!(native_default.as_deref(), Some("61000000"));
    assert_eq!(defaults.get::<_, String>(4), "61000000");
    assert_eq!(defaults.get::<_, String>(5), "€ÿ");
    let float_defaults=pg.query_one("SELECT float_value::double precision,double_value,json_value IS NULL FROM t10_values_acceptance.t10_values WHERE id=3",&[]).await.unwrap();
    assert_eq!(float_defaults.get::<_, f64>(0), 0.10000000149011612);
    assert_eq!(float_defaults.get::<_, f64>(1), f64::MAX);
    assert!(float_defaults.get::<_, bool>(2));
    let transformed=pg.query_one("SELECT trimmed,cleaned,empty_text,truth,bit_truth,encode(hex_bytes,'hex'),host(ip),year_number,tiny_number,encode(raw_bytes,'hex') FROM t10_values_acceptance.t10_transforms WHERE id=1",&[]).await.unwrap();
    assert_eq!(transformed.get::<_, String>(0), "trailing");
    assert_eq!(transformed.get::<_, String>(1), "ab");
    assert_eq!(transformed.get::<_, Option<String>>(2), None);
    assert!(transformed.get::<_, bool>(3));
    assert!(transformed.get::<_, bool>(4));
    assert_eq!(transformed.get::<_, String>(5), "00ff5c");
    assert_eq!(transformed.get::<_, String>(6), "127.0.0.1");
    assert_eq!(transformed.get::<_, i16>(7), 2155);
    assert_eq!(transformed.get::<_, i16>(8), -128);
    assert_eq!(transformed.get::<_, String>(9), "00ff5c");
    assert_eq!(resolved.tables.len(), 4);
    let enum_name:String=pg.query_one("SELECT t.typname::text FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='t10_values_acceptance' AND t.typtype='e'",&[]).await.unwrap().get(0);
    let catalog = mysql::inspect(&mut source).await.unwrap();
    for (schema, definition) in [
        ("t10_domain_conflict", "DOMAIN"),
        ("t10_composite_conflict", "TYPE"),
    ] {
        pg.batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE; CREATE SCHEMA {}; CREATE {definition} {} {};",
            plan::quote_identifier(schema),
            plan::quote_identifier(schema),
            plan::qualified_name(schema, &enum_name),
            if definition == "DOMAIN" {
                "AS text"
            } else {
                "AS (value integer)"
            }
        ))
        .await
        .unwrap();
        let inspected = my2pg::postgres::inspect(&mut pg, schema).await.unwrap();
        assert!(inspected.occupied_types.contains(&enum_name));
        let mut collision = config.clone();
        collision.target.schema = schema.into();
        assert!(
            matches!(plan::build(&collision,&catalog,&inspected),Err(plan::PlanError::Blocked(ref errors)) if errors.iter().any(|error|error.code=="ENUM_NAME_OCCUPIED"))
        );
    }
    source
        .query_drop("DROP TABLE source.t10_values,source.t10_choices,source.t10_transforms,source.t10_unsigned")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    drop(pg);
    task.await.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4 and PostgreSQL16 TLS fixtures"]
async fn native_invalid_enum_zero_dates_and_byte_sequences_are_explicit_errors() {
    let mut config: MigrationConfig =
        toml::from_str(include_str!("../contracts/config.toml")).unwrap();
    config.tables.rename.clear();
    config.tables.include = vec!["t10_rejections".into()];
    config.source.ca_file = Some(required("MY2PG_TLS_CA").into());
    config.target.ca_file = config.source.ca_file.clone();
    config.target.schema = "t10_rejections_acceptance".into();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    source.query_drop("SET SESSION sql_mode=''").await.unwrap();
    source
        .query_drop("DROP TABLE IF EXISTS source.t10_rejections")
        .await
        .unwrap();
    source.query_drop("CREATE TABLE source.t10_rejections(id INT PRIMARY KEY,state ENUM('','valid'),broken_date DATE NOT NULL DEFAULT '0000-00-00',raw_text VARBINARY(8)) ENGINE=InnoDB").await.unwrap();
    source.query_drop("INSERT INTO source.t10_rejections VALUES(1,'outside','0000-00-00',X'FF'),(2,'','2024-02-29',X'6162')").await.unwrap();
    let ordinals: Vec<u64> = source
        .query("SELECT CAST(state AS UNSIGNED) FROM source.t10_rejections ORDER BY id")
        .await
        .unwrap();
    assert_eq!(ordinals, [0, 1]);
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let (mut pg, task) = target().await;
    let target = my2pg::postgres::inspect(&mut pg, &config.target.schema)
        .await
        .unwrap();
    let blocked = plan::build(&config, &catalog, &target).unwrap_err();
    assert!(
        matches!(blocked,plan::PlanError::Blocked(ref errors) if errors.iter().any(|error|error.code=="DEFAULT_UNSUPPORTED"))
    );
    config.cast = vec![
        CastRule {
            source_table: Some("t10_rejections".into()),
            source_column: Some("state".into()),
            target_type: Some("text".into()),
            ..Default::default()
        },
        CastRule {
            source_table: Some("t10_rejections".into()),
            source_column: Some("broken_date".into()),
            drop_default: true,
            ..Default::default()
        },
        CastRule {
            source_table: Some("t10_rejections".into()),
            source_column: Some("raw_text".into()),
            target_type: Some("text".into()),
            charset: Some("utf8mb4".into()),
            ..Default::default()
        },
    ];
    let resolved = plan::build(&config, &catalog, &target).unwrap();
    let table = &resolved.tables[0];
    let mut stream = mysql::table_stream(&mut source, &catalog.database, table)
        .await
        .unwrap();
    let first = mysql::raw_values(stream.try_next().await.unwrap().unwrap(), 1024).unwrap();
    let second = mysql::raw_values(stream.try_next().await.unwrap().unwrap(), 1024).unwrap();
    drop(stream);
    assert!(convert::encode_value(&table.columns[1], &first[1]).is_err());
    assert!(convert::encode_value(&table.columns[2], &first[2]).is_err());
    assert!(convert::encode_value(&table.columns[3], &first[3]).is_err());
    assert_eq!(
        convert::encode_value(&table.columns[1], &second[1])
            .unwrap()
            .as_deref(),
        Some("")
    );
    assert_eq!(
        convert::encode_value(&table.columns[2], &second[2])
            .unwrap()
            .as_deref(),
        Some("2024-02-29")
    );
    config.cast[1].transform = Some("zero-dates-to-null".into());
    assert!(
        matches!(plan::build(&config,&catalog,&target),Err(plan::PlanError::Blocked(ref errors)) if errors.iter().any(|error|error.code=="ZERO_DATE_POLICY"))
    );
    config.cast[1].drop_not_null = true;
    let transformed = plan::build(&config, &catalog, &target).unwrap();
    let date = &transformed.tables[0].columns[2];
    assert_eq!(convert::encode_value(date, &first[2]).unwrap(), None);
    assert!(convert::transformation_applied(date, &first[2]).unwrap());
    let mut empty_enum = transformed.tables[0].columns[1].clone();
    empty_enum.transform = Some("empty-string-to-null".into());
    assert!(convert::encode_value(&empty_enum, &first[1]).is_err());
    assert_eq!(
        convert::encode_value(&empty_enum, &second[1]).unwrap(),
        None
    );
    source
        .query_drop("DROP TABLE source.t10_rejections")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    drop(pg);
    task.await.unwrap().unwrap();
}
