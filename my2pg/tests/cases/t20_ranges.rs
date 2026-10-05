//! Native exact-multiset acceptance for opt-in MySQL integer-key readers.
use clap::Parser;
use my2pg::{cli::Args, config::*, mysql, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{
    env,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL/PostgreSQL TLS fixtures"]
async fn integer_ranges_preserve_exact_signed_unsigned_and_fallback_multisets() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let suffix = format!(
        "{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let schema = format!("t20_ranges_{suffix}");
    let app = format!("{schema}_run");
    let names = [
        "signed",
        "unsigned",
        "empty",
        "single",
        "fallback",
        "text_pk",
        "composite_pk",
    ]
    .map(|name| format!("{schema}_{name}"));
    let config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {"url_env":"MY2PG_MYSQL_URL", "consistency":"frozen", "ca_file":required("MY2PG_TLS_CA")},
        "target": {"url_env":"MY2PG_POSTGRES_URL", "schema":schema, "ca_file":required("MY2PG_TLS_CA"), "session":{"application_name":app}},
        "migration": {"table_workers":1,"readers_per_table":3,"max_key_span":u64::MAX,"index_workers":1,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"queue_batches":1,"memory_bytes":4194304,"reset_sequences":false},
        "tables": {"include":names},
        "report": {"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&schema),"progress":"never"}
    })).unwrap();
    let mut config = config;
    let text_collation_override = format!("source.{}.id.collation", names[5]);
    config.overrides.push(ObjectOverride {
        object: text_collation_override,
        omit: true,
        target_expression: None,
        target_sql: None,
        materialize: None,
    });
    let creds = resolve_credentials(&config).unwrap();
    let mut source = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        1024 * 1024,
    )
    .await
    .unwrap();
    for name in &names {
        source
            .query_drop(format!(
                "DROP TABLE IF EXISTS source.{}",
                mysql::quote_ident(name)
            ))
            .await
            .unwrap();
    }
    source
        .query_drop(format!(
            "CREATE TABLE source.{}(id BIGINT NOT NULL PRIMARY KEY) ENGINE=InnoDB",
            mysql::quote_ident(&names[0])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO source.{} VALUES ({}),(-9),(-3),(4),(11),({})",
            mysql::quote_ident(&names[0]),
            i64::MIN,
            i64::MAX
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE source.{}(id BIGINT UNSIGNED NOT NULL PRIMARY KEY) ENGINE=InnoDB",
            mysql::quote_ident(&names[1])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO source.{} VALUES (0),(9223372036854775808),(18446744073709551615)",
            mysql::quote_ident(&names[1])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE source.{}(id INT NOT NULL PRIMARY KEY) ENGINE=InnoDB",
            mysql::quote_ident(&names[2])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE source.{}(id INT NOT NULL PRIMARY KEY) ENGINE=InnoDB",
            mysql::quote_ident(&names[3])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO source.{} VALUES (7)",
            mysql::quote_ident(&names[3])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE source.{}(id INT NOT NULL, value VARCHAR(8) NOT NULL) ENGINE=InnoDB",
            mysql::quote_ident(&names[4])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO source.{} VALUES (1,'a'),(1,'b'),(3,'c')",
            mysql::quote_ident(&names[4])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE source.{}(id VARCHAR(32) NOT NULL PRIMARY KEY) ENGINE=InnoDB",
            mysql::quote_ident(&names[5])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO source.{} VALUES ('alpha'),('zeta')",
            mysql::quote_ident(&names[5])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE source.{}(first_key INT NOT NULL, second_key INT NOT NULL, value VARCHAR(8) NOT NULL, PRIMARY KEY(first_key,second_key)) ENGINE=InnoDB",
            mysql::quote_ident(&names[6])
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO source.{} VALUES (1,11,'dup'),(2,22,'dup'),(4,44,'tail')",
            mysql::quote_ident(&names[6])
        ))
        .await
        .unwrap();

    let target = postgres::connect(&config.target, creds.target.expose())
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
    let (_, cancel) = watch::channel(false);
    let mut console = Console::new(
        &config,
        &Args::try_parse_from(["my2pg", "run", "fixture.toml", "--quiet"]).unwrap(),
    );
    let report = crate::test_pipeline::run(&config, &creds, &mut console, cancel)
        .await
        .unwrap();
    assert_eq!(report.exit_code(), 0, "{report:?}");
    let text_collation_object = format!("source.{}.id", names[5]);
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "COLLATION_SEMANTICS"
                && diagnostic.object.as_deref() == Some(text_collation_object.as_str())
                && diagnostic.severity == my2pg::model::Severity::Warning
        }),
        "exact collation omission should be acknowledged in the report: {report:?}"
    );
    for name in [&names[2], &names[3], &names[4], &names[5], &names[6]] {
        let table_id = &report
            .tables
            .iter()
            .find(|table| &table.source_name == name)
            .unwrap()
            .id;
        assert!(
            report.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "RANGE_FALLBACK" && diagnostic.object.as_ref() == Some(table_id)
            }),
            "fallback should be explicit for {name}: {report:?}"
        );
    }

    for name in [&names[0], &names[1], &names[2], &names[3], &names[5]] {
        let mut source_rows: Vec<String> = source
            .query(format!(
                "SELECT CAST(id AS CHAR) FROM source.{} ORDER BY id",
                mysql::quote_ident(name)
            ))
            .await
            .unwrap();
        let mut target_rows: Vec<String> = target
            .client
            .query(
                &format!(
                    "SELECT id::text FROM {}.{} ORDER BY id",
                    postgres::quote_ident(&schema),
                    postgres::quote_ident(name)
                ),
                &[],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.get(0))
            .collect();
        // PostgreSQL may map MySQL BIGINT UNSIGNED to a wider numeric type;
        // database ORDER BY can then differ from lexical comparison of the
        // lossless text representations. Compare canonical text multisets.
        source_rows.sort();
        target_rows.sort();
        assert_eq!(
            target_rows, source_rows,
            "exact source/target multiset for {name}"
        );
    }
    let mut source_fallback: Vec<(String, String)> = source
        .query(format!(
            "SELECT CAST(id AS CHAR),value FROM source.{}",
            mysql::quote_ident(&names[4])
        ))
        .await
        .unwrap();
    let mut target_fallback: Vec<(String, String)> = target
        .client
        .query(
            &format!(
                "SELECT id::text,value FROM {}.{}",
                postgres::quote_ident(&schema),
                postgres::quote_ident(&names[4])
            ),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    source_fallback.sort();
    target_fallback.sort();
    assert_eq!(target_fallback, source_fallback, "no-key fallback multiset");
    let mut source_composite: Vec<(String, String, String)> = source
        .query(format!(
            "SELECT CAST(first_key AS CHAR),CAST(second_key AS CHAR),value FROM source.{}",
            mysql::quote_ident(&names[6])
        ))
        .await
        .unwrap();
    let mut target_composite: Vec<(String, String, String)> = target
        .client
        .query(
            &format!(
                "SELECT first_key::text,second_key::text,value FROM {}.{}",
                postgres::quote_ident(&schema),
                postgres::quote_ident(&names[6])
            ),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    source_composite.sort();
    target_composite.sort();
    assert_eq!(
        target_composite, source_composite,
        "composite-key fallback multiset"
    );
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
    drop(target);
    for name in &names {
        source
            .query_drop(format!("DROP TABLE source.{}", mysql::quote_ident(name)))
            .await
            .unwrap();
    }
}
