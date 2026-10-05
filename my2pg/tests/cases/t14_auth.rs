use my2pg::{config::*, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{
    env, fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing owned fixture variable {name}"))
}
fn source() -> SourceConfig {
    serde_json::from_value(serde_json::json!({"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")})).unwrap()
}
fn target() -> TargetConfig {
    serde_json::from_value(serde_json::json!({"url_env":"MY2PG_POSTGRES_URL","schema":"t14_auth","ca_file":required("MY2PG_TLS_CA")})).unwrap()
}
fn no_secret(error: impl std::fmt::Display + std::fmt::Debug, secret: &str) {
    let text = format!("{error} {error:?}");
    assert!(!text.contains(secret));
    assert!(!text.contains("mysql://"));
    assert!(!text.contains("postgresql://"));
}
fn message(stream: &mut impl Write, tag: u8, payload: &[u8]) {
    stream.write_all(&[tag]).unwrap();
    stream
        .write_all(&(4 + payload.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(payload).unwrap();
}
fn startup(stream: &mut impl Read) {
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let length = u32::from_be_bytes(length) as usize;
    assert!((8..4096).contains(&length));
    let mut packet = vec![0; length - 4];
    stream.read_exact(&mut packet).unwrap();
    assert_eq!(&packet[..4], &196608u32.to_be_bytes());
}
fn ready(stream: &mut impl Write) {
    message(stream, b'R', &0u32.to_be_bytes());
    message(stream, b'S', b"client_encoding\0UTF8\0");
    message(stream, b'K', &[0, 0, 0, 42, 0, 0, 0, 7]);
    message(stream, b'Z', b"I");
    stream.flush().unwrap();
}

#[tokio::test]
async fn explicit_identity_and_typed_tls_options_are_required_before_connecting() {
    let source: SourceConfig = serde_json::from_value(
        serde_json::json!({"url_env":"SOURCE","consistency":"frozen","tls_mode":"disable"}),
    )
    .unwrap();
    let target: TargetConfig = serde_json::from_value(
        serde_json::json!({"url_env":"TARGET","schema":"auth","tls_mode":"disable"}),
    )
    .unwrap();
    for uri in [
        "mysql://127.0.0.1:1/source",
        "mysql://user:secret@127.0.0.1:1/source?verify_ca=false",
    ] {
        let error = mysql::connect(&source, uri, 65536).await.unwrap_err();
        assert!(matches!(error, mysql::SourceError::Configuration(_)));
        no_secret(error, "secret");
    }
    for uri in [
        "postgresql://127.0.0.1:1/target",
        "postgresql://user:secret@127.0.0.1:1/target?sslmode=disable",
        "postgresql://user:secret@%2Ftmp/target",
        "postgresql://user:secret@127.0.0.1,localhost/target",
    ] {
        let error = postgres::connect(&target, uri).await.unwrap_err();
        assert_eq!(error.kind, postgres::FailureKind::Configuration);
        no_secret(error, "secret");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ipv6_passfile_uses_the_driver_normalized_host() {
    use std::os::unix::fs::OpenOptionsExt;
    let listener = TcpListener::bind("[::1]:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("IPv6 listener failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        startup(&mut stream);
        message(&mut stream, b'R', &3u32.to_be_bytes());
        stream.flush().unwrap();
        let mut header = [0; 5];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], b'p');
        let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
        assert!((5..256).contains(&length));
        let mut password = vec![0; length - 4];
        stream.read_exact(&mut password).unwrap();
        // No server readiness is sent: close immediately after observing the
        // production client's correctly selected passfile authentication.
        password == b"t14-ipv6-passfile-secret\0"
    });
    let path = env::temp_dir().join(format!(
        "my2pg-t14-ipv6-{}-{}.pgpass",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    writeln!(
        file,
        r"\:\:1:{port}:target:ipv6test:t14-ipv6-passfile-secret"
    )
    .unwrap();
    drop(file);
    let config: TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env":"TARGET","schema":"auth","tls_mode":"disable","passfile":path
    }))
    .unwrap();
    let result = postgres::connect(
        &config,
        &format!("postgresql://ipv6test@[::1]:{port}/target"),
    )
    .await;
    fs::remove_file(path).unwrap();
    let observed = tokio::task::spawn_blocking(move || peer.join().unwrap())
        .await
        .unwrap();
    assert!(observed, "the real client used the IPv6 matching entry");
    let error = result.unwrap_err();
    assert_eq!(error.kind, postgres::FailureKind::Operational);
    no_secret(error, "t14-ipv6-passfile-secret");
}

#[tokio::test]
async fn invalid_source_sessions_fail_before_a_stalled_endpoint_is_contacted() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "mysql://user:t14-session-secret@127.0.0.1:{}/source",
        listener.local_addr().unwrap().port()
    );
    let base: SourceConfig = serde_json::from_value(
        serde_json::json!({"url_env":"SOURCE","consistency":"frozen","tls_mode":"disable"}),
    )
    .unwrap();
    let oversized = "x".repeat(1025);
    for (name, value) in [
        ("net_read_timeout", "t14-session-secret"),
        ("wait_timeout", "-1"),
        ("group_concat_max_len", "18446744073709551616"),
        ("time_zone", "+01:00"),
        ("unknown_session_key", "t14-session-secret"),
        ("sql_mode", ""),
        ("sql_mode", "invalid\0value"),
        ("sql_mode", oversized.as_str()),
    ] {
        let mut source = base.clone();
        source.session.insert(name.into(), value.into());
        let result = tokio::time::timeout(
            Duration::from_millis(250),
            mysql::connect(&source, &url, 65536),
        )
        .await
        .expect("invalid known configuration must not wait for a network greeting");
        let error = result.unwrap_err();
        assert!(matches!(error, mysql::SourceError::Configuration(_)));
        no_secret(error, "t14-session-secret");
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "invalid configuration must open no connection"
        );
    }
}

#[tokio::test]
async fn whole_initialization_deadlines_cover_silent_handshake_and_stalled_session() {
    let mysql_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mysql_port = mysql_listener.local_addr().unwrap().port();
    let source_accepted = Arc::new(AtomicBool::new(false));
    let observer = source_accepted.clone();
    let mysql_peer = std::thread::spawn(move || {
        let (mut stream, _) = mysql_listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        observer.store(true, Ordering::Release);
        let mut bytes = [0; 64];
        // MySQL is waiting for this peer's greeting. EOF proves timeout drops
        // the accepted socket rather than leaving an initialization task.
        while stream.read(&mut bytes).unwrap() != 0 {}
    });
    let pg_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let pg_port = pg_listener.local_addr().unwrap().port();
    let session_requested = Arc::new(AtomicBool::new(false));
    let observer = session_requested.clone();
    let pg_peer = std::thread::spawn(move || {
        let (mut stream, _) = pg_listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        startup(&mut stream);
        ready(&mut stream);
        let mut bytes = [0; 2048];
        loop {
            match stream.read(&mut bytes) {
                Ok(0) => break,
                Ok(_) => observer.store(true, Ordering::Release),
                Err(error) => panic!("session peer did not close: {error}"),
            }
        }
    });
    let source: SourceConfig = serde_json::from_value(
        serde_json::json!({"url_env":"SOURCE","consistency":"frozen","tls_mode":"disable"}),
    )
    .unwrap();
    let target: TargetConfig = serde_json::from_value(
        serde_json::json!({"url_env":"TARGET","schema":"auth","tls_mode":"disable"}),
    )
    .unwrap();
    let started = Instant::now();
    let mysql_url = format!("mysql://deadline:t14-deadline-secret@127.0.0.1:{mysql_port}/source");
    let postgres_url =
        format!("postgresql://deadline:t14-deadline-secret@127.0.0.1:{pg_port}/target");
    let (source_result, target_result) = tokio::join!(
        mysql::connect(&source, &mysql_url, 65536),
        postgres::connect(&target, &postgres_url)
    );
    assert!(matches!(
        source_result,
        Err(mysql::SourceError::InitializationDeadline)
    ));
    let error = target_result.unwrap_err();
    assert_eq!(error.kind, postgres::FailureKind::Operational);
    assert!(error.message.contains("initialization"));
    no_secret(error, "t14-deadline-secret");
    assert!(started.elapsed() >= Duration::from_secs(9));
    assert!(started.elapsed() < Duration::from_secs(15));
    assert!(source_accepted.load(Ordering::Acquire));
    assert!(
        session_requested.load(Ordering::Acquire),
        "PG auth completed and a real session query was sent"
    );
    tokio::task::spawn_blocking(move || {
        mysql_peer.join().unwrap();
        pg_peer.join().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixtures and dedicated PostgreSQL mTLS role"]
async fn production_postgres_mutual_tls_rejects_invalid_client_identities() {
    let config = target();
    let mut uri = url::Url::parse(&required("MY2PG_POSTGRES_URL")).unwrap();
    uri.set_username("my2pg client").unwrap();
    let error = postgres::connect(&config, uri.as_str()).await.unwrap_err();
    no_secret(error, "integration-only");
    let mut mutual = config.clone();
    mutual.client_cert = Some(required("MY2PG_TLS_CLIENT").into());
    mutual.client_key = Some(required("MY2PG_TLS_CLIENT_KEY").into());
    let connection = postgres::connect(&mutual, uri.as_str())
        .await
        .expect("production PEM client identity must satisfy cert+password auth");
    let row = connection
        .client
        .query_one(
            "SELECT current_user,ssl,client_dn FROM pg_stat_ssl WHERE pid=pg_backend_pid()",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "my2pg client");
    assert!(row.get::<_, bool>(1));
    assert!(
        row.get::<_, Option<String>>(2)
            .unwrap()
            .contains("my2pg client")
    );
    connection.close().await.unwrap();
    for name in ["UNTRUSTED_CLIENT", "EXPIRED_CLIENT", "WRONG_CLIENT"] {
        let mut invalid = mutual.clone();
        invalid.client_cert = Some(required(&format!("MY2PG_TLS_{name}")).into());
        invalid.client_key = Some(required(&format!("MY2PG_TLS_{name}_KEY")).into());
        no_secret(
            postgres::connect(&invalid, uri.as_str()).await.unwrap_err(),
            "integration-only",
        );
    }
    uri.set_password(Some("t14-wrong-password-sentinel"))
        .unwrap();
    no_secret(
        postgres::connect(&mutual, uri.as_str()).await.unwrap_err(),
        "t14-wrong-password-sentinel",
    );
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4 certificate/password fixture"]
async fn production_mysql_mutual_tls_rejects_invalid_certificate_chains() {
    let source = source();
    let mut uri = url::Url::parse(&required("MY2PG_MYSQL_URL")).unwrap();
    uri.set_username("tls_client").unwrap();
    no_secret(
        mysql::connect(&source, uri.as_str(), 65536)
            .await
            .unwrap_err(),
        "integration-only",
    );
    let mut mutual = source.clone();
    mutual.client_cert = Some(required("MY2PG_TLS_CLIENT").into());
    mutual.client_key = Some(required("MY2PG_TLS_CLIENT_KEY").into());
    let mut conn = mysql::connect(&mutual, uri.as_str(), 65536).await.unwrap();
    assert!(conn.opts().ssl_opts().unwrap().disable_built_in_roots());
    let user: String = conn
        .query_first("SELECT CURRENT_USER()")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user, "tls_client@%");
    conn.disconnect().await.unwrap();
    for name in ["UNTRUSTED_CLIENT", "EXPIRED_CLIENT"] {
        let mut invalid = mutual.clone();
        invalid.client_cert = Some(required(&format!("MY2PG_TLS_{name}")).into());
        invalid.client_key = Some(required(&format!("MY2PG_TLS_{name}_KEY")).into());
        no_secret(
            mysql::connect(&invalid, uri.as_str(), 65536)
                .await
                .unwrap_err(),
            "integration-only",
        );
    }
    // MySQL's REQUIRE X509 policy intentionally does not compare certificate
    // CN with the password-authenticated account name (unlike PG verify-full).
    let mut other = mutual.clone();
    other.client_cert = Some(required("MY2PG_TLS_WRONG_CLIENT").into());
    other.client_key = Some(required("MY2PG_TLS_WRONG_CLIENT_KEY").into());
    mysql::connect(&other, uri.as_str(), 65536)
        .await
        .unwrap()
        .disconnect()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires owned synthetic MySQL/PostgreSQL TLS databases"]
async fn percent_encoded_passwords_private_passfiles_and_secret_redaction() {
    let source = source();
    let target = target();
    let mut admin = mysql::connect(&source, &required("MY2PG_MYSQL_ROOT_URL"), 65536)
        .await
        .unwrap();
    let postgres = postgres::connect(&target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let password = "t14:p\\a/ss%@'Ω";
    admin
        .query_drop("DROP USER IF EXISTS 't14_percent'@'%'")
        .await
        .unwrap();
    admin
        .query_drop("SET SESSION sql_mode='NO_BACKSLASH_ESCAPES'")
        .await
        .unwrap();
    admin
        .query_drop(format!(
            "CREATE USER 't14_percent'@'%' IDENTIFIED BY '{}'",
            password.replace('\'', "''")
        ))
        .await
        .unwrap();
    admin
        .query_drop("GRANT SELECT ON source.* TO 't14_percent'@'%'")
        .await
        .unwrap();
    postgres
        .client
        .batch_execute("DROP ROLE IF EXISTS t14_percent")
        .await
        .unwrap();
    postgres
        .client
        .batch_execute(&format!(
            "CREATE ROLE t14_percent LOGIN PASSWORD '{}'",
            password.replace('\'', "''")
        ))
        .await
        .unwrap();
    let mut mysql_uri = url::Url::parse(&required("MY2PG_MYSQL_URL")).unwrap();
    mysql_uri.set_username("t14_percent").unwrap();
    mysql_uri.set_password(Some(password)).unwrap();
    let mut conn = mysql::connect(&source, mysql_uri.as_str(), 65536)
        .await
        .unwrap();
    assert_eq!(
        conn.query_first::<String, _>("SELECT CURRENT_USER()")
            .await
            .unwrap()
            .unwrap(),
        "t14_percent@%"
    );
    conn.disconnect().await.unwrap();
    let mut pg_uri = url::Url::parse(&required("MY2PG_POSTGRES_URL")).unwrap();
    pg_uri.set_username("t14_percent").unwrap();
    pg_uri.set_password(Some(password)).unwrap();
    let conn = postgres::connect(&target, pg_uri.as_str()).await.unwrap();
    assert_eq!(
        conn.client
            .query_one("SELECT current_user", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "t14_percent"
    );
    conn.close().await.unwrap();
    let directory = PathBuf::from(required("MY2PG_ARTIFACT_DIR"));
    let path = directory.join("t14.pgpass");
    let escaped = password.replace('\\', "\\\\").replace(':', "\\:");
    // A matching wildcard first entry must beat the later exact entry.
    fs::write(
        &path,
        format!(
            "*:{}:target:t14_percent:{escaped}\n{}:{}:target:t14_percent:wrong\n",
            pg_uri.port().unwrap(),
            pg_uri.host_str().unwrap(),
            pg_uri.port().unwrap()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut pass = target.clone();
    pass.passfile = Some(path.clone());
    pg_uri.set_password(None).unwrap();
    let conn = postgres::connect(&pass, pg_uri.as_str()).await.unwrap();
    conn.close().await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        no_secret(
            postgres::connect(&pass, pg_uri.as_str()).await.unwrap_err(),
            password,
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::write(&path, "malformed:t14:password").unwrap();
    no_secret(
        postgres::connect(&pass, pg_uri.as_str()).await.unwrap_err(),
        password,
    );
    fs::remove_file(&path).unwrap();
    no_secret(
        postgres::connect(&pass, pg_uri.as_str()).await.unwrap_err(),
        password,
    );
    mysql_uri
        .set_password(Some("t14-auth-failure-sentinel"))
        .unwrap();
    no_secret(
        mysql::connect(&source, mysql_uri.as_str(), 65536)
            .await
            .unwrap_err(),
        "t14-auth-failure-sentinel",
    );
    pg_uri
        .set_password(Some("t14-auth-failure-sentinel"))
        .unwrap();
    no_secret(
        postgres::connect(&target, pg_uri.as_str())
            .await
            .unwrap_err(),
        "t14-auth-failure-sentinel",
    );
    admin
        .query_drop("DROP USER 't14_percent'@'%'")
        .await
        .unwrap();
    postgres
        .client
        .batch_execute("DROP ROLE t14_percent")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    postgres.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned synthetic MySQL/PostgreSQL TLS databases"]
async fn sessions_apply_on_every_connection_and_bad_values_fail_without_leaking() {
    let mut source = source();
    let mut target = target();
    source
        .session
        .insert("net_read_timeout".into(), "17".into());
    source
        .session
        .insert("net_write_timeout".into(), "19".into());
    target
        .session
        .insert("application_name".into(), "t14-session".into());
    target
        .session
        .insert("statement_timeout".into(), "12000".into());
    for _ in 0..2 {
        let mut conn = mysql::connect(&source, &required("MY2PG_MYSQL_URL"), 65536)
            .await
            .unwrap();
        let settings: (String, u64, u64) = conn
            .query_first(
                "SELECT @@session.time_zone,@@session.net_read_timeout,@@session.net_write_timeout",
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(settings, ("+00:00".into(), 17, 19));
        conn.disconnect().await.unwrap();
        let conn = postgres::connect(&target, &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap();
        let row=conn.client.query_one("SELECT current_setting('application_name'),current_setting('TimeZone'),current_setting('DateStyle'),current_setting('standard_conforming_strings'),current_setting('statement_timeout')",&[]).await.unwrap();
        assert_eq!(row.get::<_, String>(0), "t14-session");
        assert_eq!(row.get::<_, String>(1), "UTC");
        assert_eq!(row.get::<_, String>(2), "ISO, YMD");
        assert_eq!(row.get::<_, String>(3), "on");
        assert_eq!(row.get::<_, String>(4), "12s");
        conn.close().await.unwrap();
    }
    source
        .session
        .insert("sql_mode".into(), "t14-session-secret-sentinel".into());
    no_secret(
        mysql::connect(&source, &required("MY2PG_MYSQL_URL"), 65536)
            .await
            .unwrap_err(),
        "t14-session-secret-sentinel",
    );
    source.session.remove("sql_mode");
    source.session.insert("net_read_timeout".into(), "0".into());
    let error = mysql::connect(&source, &required("MY2PG_MYSQL_URL"), 65536)
        .await
        .unwrap_err();
    assert!(
        matches!(error, mysql::SourceError::Configuration(_)),
        "server-clamped source session values must fail initialization"
    );
    target.session.insert(
        "statement_timeout".into(),
        "t14-session-secret-sentinel".into(),
    );
    no_secret(
        postgres::connect(&target, &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap_err(),
        "t14-session-secret-sentinel",
    );
}

#[derive(Clone, Copy)]
enum Prelude {
    Mysql,
    Postgres,
}
fn tls_peer(mode: Prelude, certificate: &str) -> (u16, std::thread::JoinHandle<(bool, bool)>) {
    let cert = PathBuf::from(required(certificate));
    let key = cert.with_file_name(format!(
        "{}-key.pem",
        cert.file_stem().unwrap().to_str().unwrap()
    ));
    let identity =
        native_tls::Identity::from_pkcs8(&fs::read(cert).unwrap(), &fs::read(key).unwrap())
            .unwrap();
    let acceptor = native_tls::TlsAcceptor::new(identity).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        match mode {
            Prelude::Postgres => {
                let mut request = [0; 8];
                stream.read_exact(&mut request).unwrap();
                assert_eq!(request, [0, 0, 0, 8, 4, 210, 22, 47]);
                stream.write_all(b"S").unwrap();
            }
            Prelude::Mysql => {
                let mut greeting = vec![10];
                greeting.extend_from_slice(b"8.4.11\0");
                greeting.extend_from_slice(&7u32.to_le_bytes());
                greeting.extend_from_slice(b"abcdefgh\0");
                greeting.extend_from_slice(&0x8a09u16.to_le_bytes());
                greeting.push(45);
                greeting.extend_from_slice(&2u16.to_le_bytes());
                greeting.extend_from_slice(&8u16.to_le_bytes());
                greeting.push(21);
                greeting.extend_from_slice(&[0; 10]);
                greeting.extend_from_slice(b"ijklmnopqrst\0caching_sha2_password\0");
                stream.write_all(&[greeting.len() as u8, 0, 0, 0]).unwrap();
                stream.write_all(&greeting).unwrap();
                let mut header = [0; 4];
                stream.read_exact(&mut header).unwrap();
                let length = usize::from(header[0])
                    | (usize::from(header[1]) << 8)
                    | (usize::from(header[2]) << 16);
                assert_eq!(length, 32);
                let mut ssl = [0; 32];
                stream.read_exact(&mut ssl).unwrap();
                assert_ne!(u32::from_le_bytes(ssl[..4].try_into().unwrap()) & 0x800, 0);
            }
        }
        let tls = acceptor.accept(stream);
        (true, tls.is_ok())
    });
    (port, thread)
}

#[tokio::test]
#[ignore = "requires owned certificate fixtures; production connectors use actual scoped TLS peers"]
async fn production_hostname_and_expired_server_validation_are_tls_failures() {
    for certificate in ["MY2PG_TLS_WRONG_HOST", "MY2PG_TLS_EXPIRED_SERVER"] {
        let (port, peer) = tls_peer(Prelude::Postgres, certificate);
        no_secret(
            postgres::connect(
                &target(),
                &format!("postgresql://test:t14-tls-secret@127.0.0.1:{port}/target"),
            )
            .await
            .unwrap_err(),
            "t14-tls-secret",
        );
        let (ssl_requested, tls_accepted) = peer.join().unwrap();
        assert!(ssl_requested);
        assert!(
            !tls_accepted,
            "production client must reject invalid server certificate before SQL/auth"
        );
        let (port, peer) = tls_peer(Prelude::Mysql, certificate);
        no_secret(
            mysql::connect(
                &source(),
                &format!("mysql://test:t14-tls-secret@127.0.0.1:{port}/source"),
                65536,
            )
            .await
            .unwrap_err(),
            "t14-tls-secret",
        );
        let (ssl_requested, tls_accepted) = peer.join().unwrap();
        assert!(ssl_requested);
        assert!(
            !tls_accepted,
            "production source must reject invalid server certificate before password authentication"
        );
    }
}
