use futures_util::TryStreamExt;
use my2pg::{
    config::{Consistency, SourceConfig},
    model::{ColumnPlan, RawValue, TablePlan, ValueKind},
    mysql,
};
use mysql_async::{Conn, Opts, prelude::Queryable};
use std::{collections::BTreeMap, env};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}
fn config() -> SourceConfig {
    serde_json::from_value(serde_json::json!({"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")})).unwrap()
}
fn column(source: &str, kind: ValueKind, charset: Option<&str>) -> ColumnPlan {
    ColumnPlan {
        source_name: source.into(),
        target_name: "renamed destination".into(),
        source_type: "".into(),
        target_type: "".into(),
        kind,
        nullable: true,
        default_sql: None,
        identity: false,
        generated_expression: None,
        copy: true,
        transform: None,
        charset: charset.map(Into::into),
        comment: None,
        enum_labels: Vec::new(),
        set_labels: Vec::new(),
    }
}
fn table(name: &str, columns: Vec<ColumnPlan>) -> TablePlan {
    TablePlan {
        id: "t05".into(),
        source_name: name.into(),
        target_schema: "legacy".into(),
        target_name: "different target name".into(),
        engine: "InnoDB".into(),
        is_view: false,
        estimated_rows: None,
        next_auto_increment: None,
        columns,
        primary_key: Vec::new(),
        structure: Default::default(),
    }
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture"]
async fn catalog_preserves_order_and_original_metadata() {
    let mut admin = Conn::new(Opts::from_url(&required("MY2PG_MYSQL_ROOT_URL")).unwrap())
        .await
        .unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS `t05 child`; DROP TABLE IF EXISTS `t05 parent`")
        .await
        .unwrap();
    admin.query_drop("CREATE TABLE `t05 parent` (`b` INT NOT NULL, `a` INT NOT NULL, `literal` VARCHAR(8) DEFAULT '' COMMENT 'empty default', PRIMARY KEY (`b`,`a`)) ENGINE=InnoDB COMMENT='parent comment'").await.unwrap();
    admin.query_drop("CREATE TABLE `t05 child` (`id` BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY, `parent a` INT, `parent b` INT, `hidden` INT INVISIBLE DEFAULT 7, `double` INT GENERATED ALWAYS AS (`checked` * 2) STORED, `checked` INT, CONSTRAINT `t05 fk` FOREIGN KEY (`parent b`,`parent a`) REFERENCES `t05 parent` (`b`,`a`) ON UPDATE CASCADE ON DELETE SET NULL, KEY `t05 ix` (`parent a` DESC, `parent b`), CONSTRAINT `t05 ck` CHECK (`checked` >= 0)) ENGINE=InnoDB AUTO_INCREMENT=42 COMMENT='child comment'").await.unwrap();
    let mut conn = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 1024 * 1024)
        .await
        .unwrap();
    conn.query_drop("SET SESSION auto_increment_increment=5; SET SESSION auto_increment_offset=3")
        .await
        .unwrap();
    let catalog = mysql::inspect(&mut conn).await.unwrap();
    assert_eq!(catalog.auto_increment_increment, 5);
    assert_eq!(catalog.auto_increment_offset, 3);
    assert!(
        catalog.server_version.starts_with("8.0.") || catalog.server_version.starts_with("8.4."),
        "catalog fixture requires reviewed Oracle MySQL 8.0 or 8.4"
    );
    let parent = catalog
        .tables
        .iter()
        .find(|t| t.name == "t05 parent")
        .unwrap();
    assert_eq!(parent.comment, "parent comment");
    assert_eq!(
        parent
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["b", "a", "literal"]
    );
    assert_eq!(parent.columns[2].default.as_deref(), Some(""));
    assert_eq!(parent.columns[2].comment, "empty default");
    assert_eq!(
        parent
            .indexes
            .iter()
            .find(|i| i.primary)
            .unwrap()
            .parts
            .iter()
            .map(|p| p.column.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
    let child = catalog
        .tables
        .iter()
        .find(|t| t.name == "t05 child")
        .unwrap();
    assert_eq!(child.next_auto_increment, Some(42));
    assert_eq!(child.columns[0].column_type, "bigint unsigned");
    assert!(child.columns[0].extra.contains("auto_increment"));
    assert!(child.columns[3].extra.contains("INVISIBLE"));
    assert!(
        child.columns[4]
            .generation_expression
            .as_deref()
            .unwrap()
            .contains("checked")
    );
    let fk = &child.foreign_keys[0];
    assert_eq!(fk.columns, ["parent b", "parent a"]);
    assert_eq!(fk.referenced_columns, ["b", "a"]);
    assert_eq!(fk.on_update, "CASCADE");
    assert_eq!(fk.on_delete, "SET NULL");
    assert_eq!(child.checks[0].name, "t05 ck");
    assert!(child.checks[0].enforced);
    assert!(
        child
            .indexes
            .iter()
            .find(|i| i.name == "t05 ix")
            .unwrap()
            .parts[0]
            .descending
    );
    mysql::start_snapshot(&mut conn).await.unwrap();
    let one: Option<u8> = conn.query_first("SELECT 1").await.unwrap();
    assert_eq!(one, Some(1));
    assert!(
        conn.query_drop("INSERT INTO `t05 parent` (`b`,`a`) VALUES (99,99)")
            .await
            .is_err(),
        "snapshot must be read only"
    );
    conn.query_drop("ROLLBACK").await.unwrap();
    conn.disconnect().await.unwrap();
    admin
        .query_drop("DROP TABLE `t05 child`; DROP TABLE `t05 parent`")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture"]
async fn projection_preserves_raw_charset_set_mask_and_snapshot() {
    let mut writer = Conn::new(Opts::from_url(&required("MY2PG_MYSQL_ROOT_URL")).unwrap())
        .await
        .unwrap();
    writer
        .query_drop("DROP TABLE IF EXISTS `t05 raw` ")
        .await
        .unwrap();
    assert!(
        writer
            .query_drop("CREATE TABLE `t05 raw` (`membership` SET('a,b'))")
            .await
            .is_err(),
        "Oracle MySQL forbids comma-containing SET labels"
    );
    let labels = (0..64)
        .map(|i| format!("'label{i}'"))
        .collect::<Vec<_>>()
        .join(",");
    writer.query_drop(format!("CREATE TABLE `t05 raw` (`id` INT PRIMARY KEY, `odd``text` VARCHAR(8) CHARACTER SET latin1, `membership` SET({labels}), `blob` BLOB) ENGINE=InnoDB")).await.unwrap();
    writer
        .query_drop(
            "INSERT INTO `t05 raw` VALUES (1,_latin1 0xE9,9223372036854775809,REPEAT('x',4096))",
        )
        .await
        .unwrap();
    let mut conn = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 16384)
        .await
        .unwrap();
    mysql::start_snapshot(&mut conn).await.unwrap();
    let plan = table(
        "t05 raw",
        vec![
            column("id", ValueKind::SignedInteger, None),
            column("odd`text", ValueKind::Text, Some("latin1")),
            column("membership", ValueKind::Set, None),
            column("blob", ValueKind::Binary, None),
        ],
    );
    let read = async |conn: &mut Conn| {
        let mut stream = mysql::table_stream(conn, "source", &plan).await.unwrap();
        let row = stream.try_next().await.unwrap().unwrap();
        assert!(stream.try_next().await.unwrap().is_none());
        row
    };
    let row = read(&mut conn).await;
    assert!(
        mysql::raw_values(row.clone(), 1024).is_err(),
        "oversized row is explicit"
    );
    let raw = mysql::raw_values(row, 16384).unwrap();
    assert_eq!(raw[1], RawValue::Bytes(vec![0xE9]));
    assert_eq!(raw[2], RawValue::UInt((1u64 << 63) | 1));
    writer
        .query_drop("UPDATE `t05 raw` SET `odd``text`=_latin1 0x41 WHERE id=1")
        .await
        .unwrap();
    let repeated = mysql::raw_values(read(&mut conn).await, 16384).unwrap();
    assert_eq!(
        repeated[1],
        RawValue::Bytes(vec![0xE9]),
        "one transaction retains its snapshot"
    );
    conn.query_drop("ROLLBACK").await.unwrap();
    assert_eq!(
        mysql::raw_values(read(&mut conn).await, 16384).unwrap()[1],
        RawValue::Bytes(b"A".to_vec())
    );
    mysql::cancel(conn).await.unwrap();
    writer.query_drop("DROP TABLE `t05 raw`").await.unwrap();
    writer.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires TLS-enabled isolated MySQL fixture"]
async fn production_mysql_tls_mutual_auth_and_policy() {
    let source = config();
    let url = required("MY2PG_MYSQL_URL");
    let conn = mysql::connect(&source, &url, 65536).await.unwrap();
    mysql::cancel(conn).await.unwrap();
    let mut bad = source.clone();
    bad.ca_file = Some(required("MY2PG_TLS_BAD_CA").into());
    assert!(mysql::connect(&bad, &url, 65536).await.is_err());
    let mut bad_url = url::Url::parse(&url).unwrap();
    bad_url.set_query(Some("verify_ca=false"));
    assert!(
        mysql::connect(&source, bad_url.as_str(), 65536)
            .await
            .is_err()
    );
    let mut client_url = url::Url::parse(&url).unwrap();
    client_url.set_username("tls_client").unwrap();
    assert!(
        mysql::connect(&source, client_url.as_str(), 65536)
            .await
            .is_err(),
        "X509 account needs a certificate"
    );
    let mut mutual = source.clone();
    mutual.client_cert = Some(required("MY2PG_TLS_CLIENT").into());
    mutual.client_key = Some(required("MY2PG_TLS_CLIENT_KEY").into());
    let mut conn = mysql::connect(&mutual, client_url.as_str(), 65536)
        .await
        .expect("PEM client certificate authenticates");
    let cipher: (String, String) = conn
        .query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
        .await
        .unwrap()
        .unwrap();
    assert!(!cipher.1.is_empty());
    conn.disconnect().await.unwrap();
    let mut admin = Conn::new(Opts::from_url(&required("MY2PG_MYSQL_ROOT_URL")).unwrap())
        .await
        .unwrap();
    let (version, auth): (String, Option<String>) = admin
        .query_first("SELECT VERSION(), (SELECT plugin FROM mysql.user WHERE user='my2pg')")
        .await
        .unwrap()
        .unwrap();
    let expected = if version.starts_with("5.7.") {
        "mysql_native_password"
    } else if version.starts_with("8.0.") || version.starts_with("8.4.") {
        "caching_sha2_password"
    } else {
        panic!("unreviewed Oracle MySQL version: {version}");
    };
    assert_eq!(auth.as_deref(), Some(expected));
    admin.disconnect().await.unwrap();
}

#[test]
fn nontransactional_snapshot_rejected() {
    let mut catalog = my2pg::model::SourceCatalog {
        database: "source".into(),
        server_version: "8.4.9".into(),
        server_comment: "MySQL Community Server - GPL".into(),
        auto_increment_increment: 1,
        auto_increment_offset: 1,
        tables: Vec::new(),
        unsupported_objects: Vec::new(),
    };
    catalog.tables.push(my2pg::model::SourceTable {
        name: "myisam".into(),
        engine: "MyISAM".into(),
        is_view: false,
        charset: None,
        collation: None,
        comment: "".into(),
        estimated_rows: None,
        next_auto_increment: None,
        columns: Vec::new(),
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
    });
    assert!(mysql::enforce_consistency(&catalog, Consistency::SingleSnapshot).is_err());
    assert!(mysql::enforce_consistency(&catalog, Consistency::Frozen).is_ok());
    let config = SourceConfig {
        url_env: Some("unused".into()),
        url_file: None,
        consistency: Consistency::Frozen,
        tls_mode: my2pg::config::TlsMode::Disable,
        ca_file: None,
        client_cert: None,
        client_key: None,
        session: BTreeMap::new(),
    };
    assert_eq!(config.consistency, Consistency::Frozen);
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture"]
async fn abandoned_reader_guard_closes_without_draining() {
    let mut conn = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 16384)
        .await
        .unwrap();
    let id = conn.id();
    let mut stream=conn.query_stream::<mysql_async::Row,_>("WITH RECURSIVE nums AS (SELECT 1 AS n UNION ALL SELECT n+1 FROM nums WHERE n<1000) SELECT SLEEP(0.01),REPEAT('x',8192) FROM nums").await.unwrap();
    assert!(stream.try_next().await.unwrap().is_some());
    drop(stream);
    drop(conn);
    let mut probe = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 16384)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let row: Option<u32> = probe
                .exec_first(
                    "SELECT ID FROM information_schema.PROCESSLIST WHERE ID=?",
                    (id,),
                )
                .await
                .unwrap();
            if row.is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("guard Drop must close, not drain a ten-second result");
    probe.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture"]
async fn catalog_query_failure_never_returns_empty_success() {
    let mut conn = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 65536)
        .await
        .unwrap();
    let mut admin = Conn::new(Opts::from_url(&required("MY2PG_MYSQL_ROOT_URL")).unwrap())
        .await
        .unwrap();
    admin
        .query_drop(format!("KILL CONNECTION {}", conn.id()))
        .await
        .unwrap();
    assert!(
        mysql::inspect(&mut conn).await.is_err(),
        "lost source metadata connection must fail"
    );
    drop(conn);
    admin.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture"]
async fn reader_drop_during_panic_and_outside_runtime_closes_promptly() {
    for panic_path in [true, false] {
        let mut conn = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 16384)
            .await
            .unwrap();
        let id = conn.id();
        let mut stream=conn.query_stream::<mysql_async::Row,_>("WITH RECURSIVE nums AS (SELECT 1 AS n UNION ALL SELECT n+1 FROM nums WHERE n<1000) SELECT SLEEP(0.01),REPEAT('x',8192) FROM nums").await.unwrap();
        assert!(stream.try_next().await.unwrap().is_some());
        drop(stream);
        if panic_path {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    let _reader = conn;
                    panic!("intentional caught source-reader panic");
                }))
                .is_err()
            );
        } else {
            std::thread::spawn(move || drop(conn))
                .join()
                .expect("reader drop outside Tokio must not panic");
        }
        let mut probe = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 16384)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let active: Option<u32> = probe
                    .exec_first(
                        "SELECT ID FROM information_schema.PROCESSLIST WHERE ID=?",
                        (id,),
                    )
                    .await
                    .unwrap();
                if active.is_none() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("panic/outside-runtime drop must close without draining");
        probe.disconnect().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture"]
async fn empty_database_and_denied_source_are_distinct() {
    let root_url = required("MY2PG_MYSQL_ROOT_URL");
    let mut admin = Conn::new(Opts::from_url(&root_url).unwrap()).await.unwrap();
    admin.query_drop("DROP DATABASE IF EXISTS t05_empty; DROP DATABASE IF EXISTS t05_denied; CREATE DATABASE t05_empty; CREATE DATABASE t05_denied").await.unwrap();
    let mut empty_url = url::Url::parse(&root_url).unwrap();
    empty_url.set_path("/t05_empty");
    let mut empty = mysql::connect(&config(), empty_url.as_str(), 65536)
        .await
        .unwrap();
    let catalog = mysql::inspect(&mut empty).await.unwrap();
    assert_eq!(catalog.database, "t05_empty");
    assert!(catalog.tables.is_empty());
    empty.disconnect().await.unwrap();
    let mut denied_url = url::Url::parse(&required("MY2PG_MYSQL_URL")).unwrap();
    denied_url.set_path("/t05_denied");
    let error = mysql::connect(&config(), denied_url.as_str(), 65536)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        mysql::SourceError::Driver {
            code: Some(1044),
            ..
        }
    ));
    empty_url.set_path("/t05_nonexistent_database");
    let error = mysql::connect(&config(), empty_url.as_str(), 65536)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        mysql::SourceError::Driver {
            code: Some(1049),
            ..
        }
    ));
    admin
        .query_drop("DROP DATABASE t05_empty; DROP DATABASE t05_denied")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture"]
async fn real_packet_over_limit_fails_before_conversion() {
    let mut conn = mysql::connect(&config(), &required("MY2PG_MYSQL_URL"), 1024)
        .await
        .unwrap();
    let mut stream = conn
        .exec_stream::<mysql_async::Row, _, _>("SELECT REPEAT('x',100000)", ())
        .await
        .unwrap();
    assert!(
        stream.try_next().await.is_err(),
        "a 100KB packet must exceed the configured 1KB row plus64KB driver allowance"
    );
    drop(stream);
    drop(conn);
}
