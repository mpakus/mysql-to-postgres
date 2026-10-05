//! Deterministically drop the COMMIT acknowledgement after PostgreSQL completed it.

use my2pg::{
    config::{MigrationConfig, TlsMode},
    model::RunStatus,
    mysql, postgres,
};
use mysql_async::prelude::Queryable;
use std::{
    env, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::Command,
    task::JoinHandle,
};
use url::Url;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing mandatory {name}"))
}

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    startup_packet: bool,
) -> io::Result<Option<Vec<u8>>> {
    let mut first = [0_u8; 1];
    if reader.read(&mut first).await? == 0 {
        return Ok(None);
    }
    if startup_packet {
        let mut prefix = [0_u8; 3];
        reader.read_exact(&mut prefix).await?;
        let length = u32::from_be_bytes([first[0], prefix[0], prefix[1], prefix[2]]) as usize;
        if !(8..=MAX_FRAME_BYTES).contains(&length) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid PostgreSQL startup frame length",
            ));
        }
        let mut frame = Vec::with_capacity(length);
        frame.extend_from_slice(&first);
        frame.extend_from_slice(&prefix);
        frame.resize(length, 0);
        reader.read_exact(&mut frame[4..]).await?;
        return Ok(Some(frame));
    }

    let mut length_bytes = [0_u8; 4];
    reader.read_exact(&mut length_bytes).await?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if !(4..=MAX_FRAME_BYTES).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid PostgreSQL message frame length",
        ));
    }
    let mut frame = Vec::with_capacity(length + 1);
    frame.extend_from_slice(&first);
    frame.extend_from_slice(&length_bytes);
    frame.resize(length + 1, 0);
    reader.read_exact(&mut frame[5..]).await?;
    Ok(Some(frame))
}

async fn forward_frontend<R, W>(
    mut reader: R,
    mut writer: W,
    copy_submitted: Arc<AtomicBool>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut startup = true;
    while let Some(frame) = read_frame(&mut reader, startup).await? {
        startup = false;
        if matches!(frame.first(), Some(b'P' | b'Q'))
            && frame.windows(5).any(|window| window == b"COPY ")
        {
            copy_submitted.store(true, Ordering::Release);
        }
        writer.write_all(&frame).await?;
        writer.flush().await?;
    }
    Ok(())
}

async fn forward_backend_until_commit<R, W>(
    mut reader: R,
    mut writer: W,
    copy_submitted: Arc<AtomicBool>,
) -> io::Result<Option<ProxyProof>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut copy_complete = false;
    let mut copy_command = None;
    while let Some(frame) = read_frame(&mut reader, false).await? {
        let command = (frame.first() == Some(&b'C')).then(|| frame.get(5..).unwrap_or_default());
        if copy_submitted.load(Ordering::Acquire)
            && command.is_some_and(|tag| tag.starts_with(b"COPY "))
        {
            copy_complete = true;
            copy_command = command.map(command_text);
        }
        if copy_complete && command.is_some_and(|tag| tag == b"COMMIT\0") {
            // PostgreSQL has emitted CommandComplete only after executing COMMIT. Hide
            // that acknowledgement and ReadyForQuery from the product connection.
            return Ok(Some(ProxyProof {
                copy_command: copy_command.expect("COPY completion was captured"),
                commit_command: command.map(command_text).expect("COMMIT completion exists"),
                commit_ack_forwarded: false,
                ready_for_query_forwarded: false,
            }));
        }
        writer.write_all(&frame).await?;
        writer.flush().await?;
    }
    Ok(None)
}

#[derive(Debug)]
struct ProxyProof {
    copy_command: String,
    commit_command: String,
    commit_ack_forwarded: bool,
    ready_for_query_forwarded: bool,
}

fn command_text(command: &[u8]) -> String {
    String::from_utf8_lossy(command.strip_suffix(b"\0").unwrap_or(command)).into_owned()
}

struct CommitAckDropProxy {
    route: String,
    task: JoinHandle<io::Result<Option<ProxyProof>>>,
}

impl CommitAckDropProxy {
    async fn start(upstream: &str) -> Self {
        let upstream_url = Url::parse(upstream).unwrap();
        let host = upstream_url.host_str().expect("PostgreSQL host");
        let port = upstream_url.port().unwrap_or(5432);
        let upstream: SocketAddr = tokio::net::lookup_host((host, port))
            .await
            .unwrap()
            .next()
            .expect("PostgreSQL endpoint resolves");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let route = route_url(&upstream_url, address.port());
        let task = tokio::spawn(async move {
            let (client, _) = listener.accept().await?;
            let server = TcpStream::connect(upstream).await?;
            let (client_read, client_write) = client.into_split();
            let (server_read, server_write) = server.into_split();
            let copy_submitted = Arc::new(AtomicBool::new(false));
            let frontend = forward_frontend(client_read, server_write, Arc::clone(&copy_submitted));
            let backend = forward_backend_until_commit(server_read, client_write, copy_submitted);
            tokio::select! {
                result = frontend => {
                    result?;
                    Ok(None)
                }
                result = backend => result,
            }
        });
        Self { route, task }
    }

    async fn wait_for_drop(self) -> Result<ProxyProof, String> {
        let proof = tokio::time::timeout(Duration::from_secs(30), self.task)
            .await
            .map_err(|_| "proxy timed out before observing completed COMMIT".to_owned())?
            .map_err(|error| format!("proxy task panicked: {error}"))?
            .map_err(|error| format!("proxy forwarding failed: {error}"))?;
        proof.ok_or_else(|| {
            "proxy connection closed before dropping a completed COMMIT acknowledgement".into()
        })
    }
}

fn route_url(upstream: &Url, port: u16) -> String {
    let mut route = upstream.clone();
    route.set_host(Some("127.0.0.1")).unwrap();
    route.set_port(Some(port)).unwrap();
    route.to_string()
}

#[tokio::test]
#[ignore = "requires owned disposable TLS databases; PostgreSQL target uses an isolated loopback plaintext proxy"]
async fn completed_commit_with_dropped_ack_is_indeterminate_and_never_replayed() {
    assert_eq!(required("MY2PG_INTEGRATION"), "1");
    let artifact_root = PathBuf::from(required("MY2PG_ARTIFACT_DIR"));
    assert_eq!(
        artifact_root.parent().unwrap().canonicalize().unwrap(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/integration")
            .canonicalize()
            .unwrap()
    );
    let source_name = format!(
        "t12_lost_ack_{}_{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let source_table = mysql::quote_ident(&source_name);
    let target_schema = source_name.clone();
    let target_table = postgres::qualified(&target_schema, &source_name);
    let run_dir = artifact_root.join(&source_name);
    std::fs::create_dir_all(&run_dir).unwrap();

    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {
            "url_env": "MY2PG_MYSQL_URL",
            "consistency": "single_snapshot",
            "ca_file": required("MY2PG_TLS_CA")
        },
        "target": {
            "url_env": "MY2PG_T12_PROXY_URL",
            "schema": target_schema,
            "tls_mode": "disable",
            "on_existing": "error"
        },
        "migration": {
            "batch_rows": 2,
            "batch_bytes": 4096,
            "max_row_bytes": 1024,
            "memory_bytes": 262144
        },
        "tables": { "include": [source_name] },
        "report": {
            "directory": run_dir.join("runs"),
            "console": "json",
            "progress": "never"
        }
    }))
    .unwrap();
    config.target.tls_mode = TlsMode::Disable;

    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE {source_table}(id INT,value TEXT) ENGINE=InnoDB CHARSET=utf8mb4"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {source_table} VALUES(1,'one'),(2,'two')"
        ))
        .await
        .unwrap();

    let target_url = required("MY2PG_POSTGRES_URL");
    let direct_target = postgres::connect(&config.target, &target_url)
        .await
        .unwrap();
    direct_target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            postgres::quote_ident(&target_schema)
        ))
        .await
        .unwrap();
    direct_target.close().await.unwrap();

    let proxy = CommitAckDropProxy::start(&target_url).await;
    let config_path = run_dir.join("migration.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("run")
        .arg(&config_path)
        .args(["--output", "json", "--progress", "never"])
        .env("MY2PG_T12_PROXY_URL", &proxy.route)
        .output()
        .await
        .unwrap();
    std::fs::write(run_dir.join("cli.stdout"), &output.stdout).unwrap();
    std::fs::write(run_dir.join("cli.stderr"), &output.stderr).unwrap();
    let proxy_proof = proxy.wait_for_drop().await.unwrap_or_else(|error| {
        panic!(
            "{error}; exit={:?} stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(proxy_proof.copy_command, "COPY 2");
    assert_eq!(proxy_proof.commit_command, "COMMIT");
    assert!(!proxy_proof.commit_ack_forwarded);
    assert!(!proxy_proof.ready_for_query_forwarded);
    std::fs::write(
        run_dir.join("proxy-proof.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "backend_copy_command_complete": proxy_proof.copy_command,
            "backend_commit_command_complete": proxy_proof.commit_command,
            "commit_ack_forwarded_to_cli": proxy_proof.commit_ack_forwarded,
            "ready_for_query_forwarded_to_cli": proxy_proof.ready_for_query_forwarded
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let events: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let report: my2pg::model::RunReport =
        serde_json::from_value(events.last().unwrap()["report"].clone()).unwrap();
    assert_eq!(report.status, RunStatus::Indeterminate);
    assert_eq!(report.tables.len(), 1);
    let table = &report.tables[0];
    assert_eq!(
        (
            table.rows_read,
            table.committed_rows,
            table.rejected_rows,
            table.unresolved_rows,
            table.indeterminate_rows
        ),
        (2, 0, 0, 0, 2)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "COMMIT_INDETERMINATE" })
    );

    let direct_target = postgres::connect(&config.target, &target_url)
        .await
        .unwrap();
    let actual: Vec<(i32, String)> = direct_target
        .client
        .query(
            &format!("SELECT id,value FROM {target_table} ORDER BY id"),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        actual,
        vec![(1, "one".into()), (2, "two".into())],
        "server committed exactly once while its acknowledgement was dropped"
    );
    direct_target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&target_schema)
        ))
        .await
        .unwrap();
    direct_target.close().await.unwrap();
    source
        .query_drop(format!("DROP TABLE {source_table}"))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
}
