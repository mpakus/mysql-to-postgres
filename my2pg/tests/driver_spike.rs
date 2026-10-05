//! Real driver proof. Run through tests/run-integration.sh; missing endpoints fail.
#[path = "cases/t05_mysql.rs"]
mod t05_mysql;
use bytes::Bytes;
use futures_util::{SinkExt, TryStreamExt};
use mysql_async::{Conn, Opts, OptsBuilder, Row, SslOpts, Value, prelude::*};
use std::{env, path::PathBuf, time::Duration};
use tokio_postgres::{NoTls, config::SslMode};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required integration environment missing: {name}"))
}

fn mysql_opts(tls: bool, ca: Option<&str>, wrong_host: bool) -> OptsBuilder {
    let opts = Opts::from_url(&required("MY2PG_MYSQL_URL")).expect("valid fixture MySQL URL");
    let ssl = tls.then(|| {
        let mut ssl = SslOpts::default().with_disable_built_in_roots(true);
        if let Some(ca) = ca {
            ssl = ssl.with_root_certs(vec![PathBuf::from(ca).into()]);
        }
        if wrong_host {
            ssl = ssl.with_danger_tls_hostname_override(Some("invalid.my2pg.test"));
        }
        ssl
    });
    OptsBuilder::from_opts(opts)
        .prefer_socket(false)
        .ssl_opts(ssl)
}

#[tokio::test]
#[ignore = "requires disposable Oracle MySQL and PostgreSQL endpoints"]
async fn mysql_raw_values_and_copy_transactions() {
    let mut mysql = Conn::new(mysql_opts(false, None, false)).await.unwrap();
    let identity: (String, String) = mysql
        .query_first("SELECT VERSION(), @@version_comment")
        .await
        .unwrap()
        .unwrap();
    assert!(
        identity.0.starts_with("8.0.") || identity.0.starts_with("8.4."),
        "spike requires reviewed Oracle MySQL 8.0 or 8.4"
    );
    assert!(identity.1.contains("MySQL"), "must use Oracle MySQL");
    mysql
        .query_drop("SET SESSION sql_mode = ''; SET SESSION time_zone = '+00:00'")
        .await
        .unwrap();
    mysql.query_drop("CREATE TEMPORARY TABLE driver_values (u BIGINT UNSIGNED, d DECIMAL(65,30), z DATE, t TIME(6), b LONGBLOB, dt DATETIME(6), ts TIMESTAMP(6), n INT NULL)").await.unwrap();
    let decimal = format!("{}.{}", "9".repeat(35), "1234567890".repeat(3));
    let binary: Vec<u8> = (0..=255).collect();
    mysql.exec_drop(
        "INSERT INTO driver_values VALUES (?, ?, '0000-00-00', '-838:59:58.123456', ?, '2024-03-10 02:30:00.123456', '2024-01-01 00:00:00.654321', NULL)",
        (u64::MAX, decimal.clone(), binary.clone()),
    ).await.unwrap();

    // Text protocol preserves bytes, including the invalid DATE, before policy.
    let text: Row = mysql
        .query_first("SELECT u,d,z,t,b FROM driver_values")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        text.as_ref(0),
        Some(&Value::Bytes(u64::MAX.to_string().into_bytes()))
    );
    assert_eq!(
        text.as_ref(1),
        Some(&Value::Bytes(decimal.as_bytes().to_vec()))
    );
    assert_eq!(text.as_ref(2), Some(&Value::Bytes(b"0000-00-00".to_vec())));
    assert_eq!(
        text.as_ref(3),
        Some(&Value::Bytes(b"-838:59:58.123456".to_vec()))
    );
    assert_eq!(text.as_ref(4), Some(&Value::Bytes(binary.clone())));

    // Production uses prepared/binary protocol; no chrono/float conversion.
    {
        let mut stream = mysql
            .exec_stream::<Row, _, _>("SELECT u,d,z,t,b,dt,ts,n FROM driver_values", ())
            .await
            .unwrap();
        let raw = stream.try_next().await.unwrap().unwrap();
        assert_eq!(raw.as_ref(0), Some(&Value::UInt(u64::MAX)));
        assert_eq!(
            raw.as_ref(1),
            Some(&Value::Bytes(decimal.as_bytes().to_vec()))
        );
        assert_eq!(raw.as_ref(2), Some(&Value::Date(0, 0, 0, 0, 0, 0, 0)));
        assert_eq!(
            raw.as_ref(3),
            Some(&Value::Time(true, 34, 22, 59, 58, 123456))
        );
        assert_eq!(raw.as_ref(4), Some(&Value::Bytes(binary.clone())));
        assert_eq!(
            raw.as_ref(5),
            Some(&Value::Date(2024, 3, 10, 2, 30, 0, 123456))
        );
        assert_eq!(
            raw.as_ref(6),
            Some(&Value::Date(2024, 1, 1, 0, 0, 0, 654321))
        );
        assert_eq!(raw.as_ref(7), Some(&Value::NULL));
        assert!(stream.try_next().await.unwrap().is_none());
    }

    let (mut pg, connection) = tokio_postgres::connect(&required("MY2PG_POSTGRES_URL"), NoTls)
        .await
        .unwrap();
    let task = tokio::spawn(connection);
    pg.batch_execute(
        "CREATE TEMP TABLE driver_values (u numeric(20,0), d numeric(65,30), b bytea)",
    )
    .await
    .unwrap();
    let hex: String = binary.iter().map(|byte| format!("{byte:02x}")).collect();
    let payload = Bytes::from(format!("{}\t{}\t\\\\x{}\n", u64::MAX, decimal, hex));
    {
        let tx = pg.transaction().await.unwrap();
        let sink = tx.copy_in("COPY driver_values FROM STDIN").await.unwrap();
        tokio::pin!(sink);
        sink.send(payload.clone()).await.unwrap();
        assert_eq!(sink.finish().await.unwrap(), 1);
        tx.rollback().await.unwrap();
    }
    assert_eq!(
        pg.query_one("SELECT count(*) FROM driver_values", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    {
        let tx = pg.transaction().await.unwrap();
        {
            let sink = tx.copy_in("COPY driver_values FROM STDIN").await.unwrap();
            tokio::pin!(sink);
            sink.send(payload.clone()).await.unwrap();
            // Unfinished COPY must not survive a rollback.
        }
        tx.rollback().await.unwrap();
    }
    assert_eq!(
        pg.query_one("SELECT count(*) FROM driver_values", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    {
        let tx = pg.transaction().await.unwrap();
        let sink = tx.copy_in("COPY driver_values FROM STDIN").await.unwrap();
        tokio::pin!(sink);
        sink.send(payload).await.unwrap();
        assert_eq!(sink.finish().await.unwrap(), 1);
        tx.commit().await.unwrap();
    }
    let row = pg
        .query_one("SELECT u::text,d::text,b FROM driver_values", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), u64::MAX.to_string());
    assert_eq!(row.get::<_, String>(1), decimal);
    assert_eq!(row.get::<_, Vec<u8>>(2), binary);
    {
        let tx = pg.transaction().await.unwrap();
        let sink = tx.copy_in("COPY driver_values FROM STDIN").await.unwrap();
        tokio::pin!(sink);
        sink.send(Bytes::from_static(b"invalid\t0\t\\N\n"))
            .await
            .unwrap();
        assert_eq!(
            sink.finish().await.unwrap_err().code(),
            Some(&tokio_postgres::error::SqlState::INVALID_TEXT_REPRESENTATION)
        );
        tx.rollback().await.unwrap();
    }
    assert_eq!(
        pg.query_one("SELECT count(*) FROM driver_values", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    mysql.disconnect().await.unwrap();
    drop(pg);
    task.await.unwrap().unwrap();
}

#[tokio::test]
#[ignore = "requires disposable TLS-enabled MySQL and PostgreSQL endpoints"]
async fn verified_tls_rejects_wrong_ca_and_hostname() {
    let ca = required("MY2PG_TLS_CA");
    let bad_ca = required("MY2PG_TLS_BAD_CA");
    let mut mysql = Conn::new(mysql_opts(true, Some(&ca), false))
        .await
        .expect("verified TLS must connect");
    let cipher: (String, String) = mysql
        .query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
        .await
        .unwrap()
        .unwrap();
    assert!(!cipher.1.is_empty());
    mysql.disconnect().await.unwrap();
    assert!(
        Conn::new(mysql_opts(true, Some(&bad_ca), false))
            .await
            .is_err(),
        "untrusted CA must fail"
    );
    assert!(
        Conn::new(mysql_opts(true, Some(&ca), true)).await.is_err(),
        "wrong hostname must fail"
    );

    let build = |ca_path: &str| {
        let mut tls = native_tls::TlsConnector::builder();
        tls.disable_built_in_roots(true);
        tls.add_root_certificate(
            native_tls::Certificate::from_pem(&std::fs::read(ca_path).unwrap()).unwrap(),
        );
        postgres_native_tls::MakeTlsConnector::new(tls.build().unwrap())
    };
    let mut cfg: tokio_postgres::Config = required("MY2PG_POSTGRES_URL").parse().unwrap();
    cfg.ssl_mode(SslMode::Require);
    let (pg, connection) = cfg
        .connect(build(&ca))
        .await
        .expect("verified PG TLS must connect");
    let task = tokio::spawn(connection);
    assert!(
        pg.query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()",
            &[]
        )
        .await
        .unwrap()
        .get::<_, bool>(0)
    );
    drop(pg);
    task.await.unwrap().unwrap();
    assert!(
        cfg.connect(build(&bad_ca)).await.is_err(),
        "untrusted PG CA must fail"
    );
    let mut wrong_url = url::Url::parse(&required("MY2PG_POSTGRES_URL")).unwrap();
    wrong_url.set_host(Some("invalid.my2pg.test")).unwrap();
    let mut wrong: tokio_postgres::Config = wrong_url.as_str().parse().unwrap();
    wrong
        .hostaddr("127.0.0.1".parse::<std::net::IpAddr>().unwrap())
        .ssl_mode(SslMode::Require);
    assert!(
        wrong.connect(build(&ca)).await.is_err(),
        "wrong PG hostname must fail"
    );
}

#[tokio::test]
#[ignore = "requires disposable Oracle MySQL endpoint"]
async fn mysql_owned_stream_backpressure_and_cancellation() {
    let config:my2pg::config::SourceConfig=serde_json::from_value(serde_json::json!({"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","tls_mode":"disable"})).unwrap();
    let mut conn = my2pg::mysql::connect(&config, &required("MY2PG_MYSQL_URL"), 16384)
        .await
        .unwrap();
    let connection_id = conn.id();
    // One hundred thousand 8KiB rows would exceed 780MiB if collected.
    let query = "WITH digits AS (SELECT 0 AS n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) SELECT SLEEP(0.01),REPEAT('x',8192) FROM digits a,digits b,digits c,digits d,digits e";
    let mut stream = conn.query_stream::<Row, _>(query).await.unwrap();
    assert_eq!(
        stream.try_next().await.unwrap().unwrap().as_ref(1),
        Some(&Value::Bytes(vec![b'x'; 8192]))
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Backpressure leaves the other 99,999 rows unread. Bare Conn::drop schedules
    // cleanup that drains a pending result. The adapter cancels an owning
    // disconnect future at its first pending I/O, closing the socket.
    drop(stream);
    tokio::time::timeout(Duration::from_secs(1), my2pg::mysql::cancel(conn))
        .await
        .expect("cancellation must not drain an approximately 1000-second result")
        .unwrap();
    let mut probe = Conn::new(mysql_opts(false, None, false)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let active: Option<u64> = probe
                .exec_first(
                    "SELECT ID FROM information_schema.PROCESSLIST WHERE ID=?",
                    (connection_id,),
                )
                .await
                .unwrap();
            if active.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cancelled owned stream must close without draining");
    assert_eq!(
        probe.query_first::<u8, _>("SELECT 1").await.unwrap(),
        Some(1)
    );
    probe.disconnect().await.unwrap();
}
