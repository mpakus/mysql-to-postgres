//! Explicit database lane: ignored in ordinary unit runs, mandatory in run-integration.sh.
#[path = "cases/t06_plan.rs"]
mod t06_plan;
#[path = "cases/t07_postgres.rs"]
mod t07_postgres;
#[path = "cases/t08_pipeline.rs"]
mod t08_pipeline;
#[path = "cases/t09_cli.rs"]
mod t09_cli;
#[path = "cases/t10_binary_default.rs"]
mod t10_binary_default;
#[path = "cases/t10_enum_metadata.rs"]
mod t10_enum_metadata;
#[path = "cases/t10_mysql_projection.rs"]
mod t10_mysql_projection;
#[path = "cases/t10_set_verification.rs"]
mod t10_set_verification;
#[path = "cases/t10_values.rs"]
mod t10_values;
#[path = "cases/t11_catalog.rs"]
mod t11_catalog;
#[path = "cases/t11_cross_schema_run.rs"]
mod t11_cross_schema_run;
#[path = "cases/t11_dependency_graph.rs"]
mod t11_dependency_graph;
#[path = "cases/t11_existing_comments.rs"]
mod t11_existing_comments;
#[path = "cases/t11_existing_policies.rs"]
mod t11_existing_policies;
#[path = "cases/t11_graph_invariants.rs"]
mod t11_graph_invariants;
#[path = "cases/t11_identity_exhaustion.rs"]
mod t11_identity_exhaustion;
#[path = "cases/t11_multischema.rs"]
mod t11_multischema;
#[path = "cases/t11_observed_decoder.rs"]
mod t11_observed_decoder;
#[path = "cases/t11_schema_only_recreate.rs"]
mod t11_schema_only_recreate;
#[path = "cases/t11_sequence_catalog.rs"]
mod t11_sequence_catalog;
#[path = "cases/t11_structures.rs"]
mod t11_structures;
#[path = "cases/t11_truncate_sequence_reset.rs"]
mod t11_truncate_sequence_reset;
#[path = "cases/t12_durable_faults.rs"]
mod t12_durable_faults;
#[path = "cases/t12_fuse_eio.rs"]
mod t12_fuse_eio;
#[path = "cases/t12_lost_commit_ack.rs"]
mod t12_lost_commit_ack;
#[path = "cases/t12_recovery.rs"]
mod t12_recovery;
#[path = "cases/t12_required_ddl.rs"]
mod t12_required_ddl;
#[path = "cases/t12_runner.rs"]
mod t12_runner;
#[path = "cases/t12_uncertain_storage.rs"]
mod t12_uncertain_storage;
#[path = "cases/t13_concurrency.rs"]
mod t13_concurrency;
#[path = "cases/t13_external_mdl.rs"]
mod t13_external_mdl;
#[path = "cases/t13_shutdown_phases.rs"]
mod t13_shutdown_phases;
#[path = "cases/t13_snapshot_consistency.rs"]
mod t13_snapshot_consistency;
#[path = "cases/t13_snapshot_mdl.rs"]
mod t13_snapshot_mdl;
#[path = "cases/t14_auth.rs"]
mod t14_auth;
#[path = "cases/t15_console.rs"]
mod t15_console;
#[path = "cases/t15_default_complete.rs"]
mod t15_default_complete;
#[path = "cases/t15_defaults.rs"]
mod t15_defaults;
#[cfg(feature = "artifact-worker-tests")]
#[path = "cases/t15_durable_run.rs"]
mod t15_durable_run;
#[path = "cases/t15_enum_catalog.rs"]
mod t15_enum_catalog;
#[path = "cases/t15_verify.rs"]
mod t15_verify;
#[path = "support/test_pipeline.rs"]
mod test_pipeline;
use mysql_async::{Conn, Opts, OptsBuilder, SslOpts, prelude::Queryable};
use std::{env, path::PathBuf};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| {
        panic!("required integration variable {name} is absent; start tests/run-integration.sh")
    })
}

#[tokio::test]
#[ignore = "requires the owned disposable MySQL/PostgreSQL harness"]
async fn deterministic_fixture_is_accessible_over_verified_tls() {
    assert_eq!(required("MY2PG_INTEGRATION"), "1");
    let ca = PathBuf::from(required("MY2PG_TLS_CA"));
    let opts = OptsBuilder::from_opts(Opts::from_url(&required("MY2PG_MYSQL_URL")).unwrap())
        .ssl_opts(SslOpts::default().with_root_certs(vec![ca.into()]));
    let mut mysql = Conn::new(opts).await.unwrap();
    let amount: String = mysql
        .query_first("SELECT CAST(amount AS CHAR) FROM source.users WHERE id=1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(amount, "1234567890123456789012.12345678");
    let unsigned: u64 = mysql
        .query_first("SELECT unsigned_big FROM source.numeric_edges WHERE id=1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unsigned, u64::MAX);
    let bytes: Vec<u8> = mysql
        .query_first("SELECT value FROM source.all_bytes WHERE id=1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, (0..=255).collect::<Vec<u8>>());
    let counts: (u64, u64, u64) = mysql.query_first("SELECT (SELECT COUNT(*) FROM source.users), (SELECT COUNT(*) FROM source.orders), (SELECT COUNT(*) FROM source.empty_table)").await.unwrap().unwrap();
    assert_eq!(counts, (3, 2, 0));
    let cipher: (String, String) = mysql
        .query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !cipher.1.is_empty(),
        "MySQL test connection must negotiate TLS"
    );
    mysql.disconnect().await.unwrap();

    let cert = native_tls::Certificate::from_pem(&std::fs::read(required("MY2PG_TLS_CA")).unwrap())
        .unwrap();
    let tls = native_tls::TlsConnector::builder()
        .add_root_certificate(cert)
        .build()
        .unwrap();
    let mut config: tokio_postgres::Config = required("MY2PG_POSTGRES_URL").parse().unwrap();
    config.ssl_mode(tokio_postgres::config::SslMode::Require);
    let (postgres, connection) = config
        .connect(postgres_native_tls::MakeTlsConnector::new(tls))
        .await
        .unwrap();
    let task = tokio::spawn(connection);
    let sentinel: String = postgres
        .query_one("SELECT sentinel FROM public.users WHERE id=999", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(sentinel, "must-survive");
    let ssl: bool = postgres
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(ssl, "PostgreSQL test connection must negotiate TLS");
    drop(postgres);
    task.await.unwrap().unwrap();
}
#[cfg(feature = "native-import-tests")]
#[path = "cases/t10_empty_string_default.rs"]
mod t10_empty_string_default;
#[path = "cases/t10_label_recovery.rs"]
mod t10_label_recovery;
#[path = "cases/t11_constraint_triggers.rs"]
mod t11_constraint_triggers;
#[path = "cases/t11_deferred_keys.rs"]
mod t11_deferred_keys;
#[path = "cases/t11_identity_edges.rs"]
mod t11_identity_edges;
#[path = "cases/t11_identity_types.rs"]
mod t11_identity_types;
#[path = "cases/t11_monotonic_sequences.rs"]
mod t11_monotonic_sequences;
#[cfg(all(unix, feature = "artifact-worker-tests"))]
#[path = "cases/t15_artifact_io.rs"]
mod t15_artifact_io;
#[path = "cases/t18_canonical.rs"]
mod t18_canonical;
#[path = "cases/t18_content.rs"]
mod t18_content;
#[path = "cases/t18_pipeline.rs"]
mod t18_pipeline;
#[path = "cases/t19_views_hooks.rs"]
mod t19_views_hooks;

#[cfg(feature = "native-import-tests")]
#[path = "cases/t16_import_native.rs"]
mod t16_import_native;
#[path = "cases/t17_check_override.rs"]
mod t17_check_override;
#[path = "cases/t17_generated_policy.rs"]
mod t17_generated_policy;
#[path = "cases/t17_index_expression.rs"]
mod t17_index_expression;
#[path = "cases/t17_on_update_override.rs"]
mod t17_on_update_override;
#[path = "cases/t17_spatial.rs"]
mod t17_spatial;
#[path = "cases/t20_ranges.rs"]
mod t20_ranges;
