//! Strangler's NULL-safe multi-column FULLTEXT replacement through the real CLI.
use my2pg::{config::*, model::MigrationPlan, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::Path};
use tokio::process::Command;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

async fn cli(
    config: &MigrationConfig,
    directory: &Path,
    label: &str,
    operation: &str,
    run: Option<&Path>,
) -> std::process::Output {
    let path = directory.join(format!("{label}.toml"));
    fs::write(&path, toml::to_string(config).unwrap()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_my2pg"));
    command
        .arg(operation)
        .arg(path)
        .args(["--output", "json", "--progress", "never"]);
    if let Some(run) = run {
        command.arg("--run-dir").arg(run);
    }
    let output = command.output().await.unwrap();
    assert!(!output.stdout.is_empty() || !output.stderr.is_empty());
    output
}

fn diagnostic(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[tokio::test]
#[ignore = "requires owned MySQL/PostgreSQL TLS fixtures"]
async fn native_fulltext_mapping_preserves_null_terms_boundaries_indexes_and_verification() {
    let suffix = std::process::id();
    let source_table = format!("T17Fulltext{suffix}");
    let schema = format!("t17_fulltext_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t17-fulltext-{suffix}"));
    fs::create_dir_all(&directory).unwrap();
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {"url_env":"MY2PG_MYSQL_URL", "ca_file":required("MY2PG_TLS_CA"), "consistency":"frozen"},
        "target": {"url_env":"MY2PG_POSTGRES_URL", "ca_file":required("MY2PG_TLS_CA"), "schema":schema},
        "migration": {"identifiers":"downcase", "reset_sequences":false, "batch_rows":2, "batch_bytes":4096,
            "max_row_bytes":1024, "memory_bytes":262144},
        "tables": {"include":[source_table], "rename":[{"source":source_table,"target":"search rows"}]},
        "report": {"directory":directory.join("runs"), "console":"json", "progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 4096)
        .await
        .unwrap();
    let table = mysql::quote_ident(&source_table);
    let body = mysql::quote_ident("Body \" (x)");
    source.query_drop(format!("CREATE TABLE {table}(id INT NOT NULL PRIMARY KEY, Title VARCHAR(64), {body} TEXT, City VARCHAR(64), FULLTEXT KEY title_only(Title), FULLTEXT KEY two_columns(Title,{body}), FULLTEXT KEY three_columns(Title,{body},City)) ENGINE=InnoDB")).await.unwrap();
    source.query_drop(format!("INSERT INTO {table} VALUES (1,'red','lamp','north'),(2,NULL,'lamp',NULL),(3,'lamp',NULL,'north'),(4,NULL,NULL,NULL),(5,'red','lamp','north'),(6,'lam','p',NULL)")).await.unwrap();
    let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let qualified = postgres::qualified(&schema, "search rows");
    assert!(
        !target
            .client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname=$1)",
                &[&schema]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let output = cli(&config, &directory, "requires-decision", "run", None).await;
    assert!(!output.status.success(), "{}", diagnostic(&output));
    assert!(diagnostic(&output).contains("INDEX_UNSUPPORTED"));
    assert!(
        !target
            .client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname=$1)",
                &[&schema]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );

    config.migration.fulltext = FulltextPolicy::SimpleGin;
    let output = cli(&config, &directory, "accepted", "run", None).await;
    assert!(output.status.success(), "{}", diagnostic(&output));
    let run = fs::read_dir(&config.report.directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let plan: MigrationPlan =
        serde_json::from_slice(&fs::read(run.join("plan.json")).unwrap()).unwrap();
    assert_eq!(
        plan.diagnostics
            .iter()
            .filter(|item| item.code == "FULLTEXT_REPLACED")
            .count(),
        3
    );
    assert_eq!(
        target
            .client
            .query_one(&format!("SELECT count(*) FROM {qualified}"), &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        6
    );
    let indexes = target.client.query("SELECT c.oid,c.relname,i.indisvalid,pg_get_expr(i.indexprs,i.indrelid) FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid JOIN pg_class t ON t.oid=i.indrelid JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_am a ON a.oid=c.relam WHERE n.nspname=$1 AND a.amname='gin' ORDER BY c.relname", &[&schema]).await.unwrap();
    assert_eq!(indexes.len(), 3);
    let oids: Vec<u32> = indexes.iter().map(|row| row.get(0)).collect();
    target
        .client
        .batch_execute("SET enable_seqscan=off")
        .await
        .unwrap();
    for index in &indexes {
        let name: String = index.get(1);
        assert!(index.get::<_, bool>(2));
        let expression: String = index.get(3);
        let expected = if name.contains("title_only") { 1 } else { 4 };
        let query = format!(
            "SELECT count(*) FROM {qualified} WHERE {expression} @@ to_tsquery('simple', 'lamp')"
        );
        assert_eq!(
            target
                .client
                .query_one(&query, &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            expected,
            "{name}"
        );
        let explain = target
            .client
            .query(&format!("EXPLAIN {query}"), &[])
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(explain.contains(&name), "{explain}");
    }
    // An independent query checks the column boundary rather than reusing the catalog expression.
    let matching: Vec<i32> = target.client.query(&format!("SELECT id FROM {qualified} WHERE to_tsvector('simple', coalesce(title::text,'') || ' ' || coalesce(\"body \"\" (x)\",'')) @@ to_tsquery('simple','lamp') ORDER BY id"), &[]).await.unwrap().iter().map(|row|row.get(0)).collect();
    assert_eq!(matching, [1, 2, 3, 5]);
    let output = cli(&config, &directory, "verify", "verify", Some(&run)).await;
    assert!(output.status.success(), "{}", diagnostic(&output));

    // Existing-target data-only preserves the GIN identity instead of rebuilding it.
    let mut append = config.clone();
    append.migration.mode = MigrationMode::DataOnly;
    append.target.on_existing = ExistingPolicy::Append;
    append.report.directory = directory.join("append-runs");
    let output = cli(&append, &directory, "append-plan", "plan", None).await;
    assert!(output.status.success(), "{}", diagnostic(&output));
    let bound: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(bound["plan"]["ddl"].as_array().unwrap().is_empty());
    let mut truncate = append.clone();
    truncate.target.on_existing = ExistingPolicy::Truncate;
    truncate.report.directory = directory.join("truncate-runs");
    let output = cli(&truncate, &directory, "truncate-run", "run", None).await;
    assert!(output.status.success(), "{}", diagnostic(&output));
    assert_eq!(
        target
            .client
            .query_one(&format!("SELECT count(*) FROM {qualified}"), &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        6
    );
    let after_oids: Vec<u32> = target.client.query("SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_am a ON a.oid=c.relam WHERE n.nspname=$1 AND a.amname='gin' ORDER BY c.relname", &[&schema]).await.unwrap().iter().map(|row|row.get(0)).collect();
    assert_eq!(after_oids, oids);

    let index = indexes
        .iter()
        .find(|row| row.get::<_, String>(1).contains("two_columns"))
        .unwrap();
    let name: String = index.get(1);
    let original_expression: String = index.get(3);
    let index_name = postgres::qualified(&schema, &name);
    for replacement in [
        original_expression.replace("'simple'", "'english'"),
        original_expression.replace("' '::text", "''::text"),
        "to_tsvector('simple'::regconfig, title::text)".into(),
    ] {
        target
            .client
            .batch_execute(&format!(
                "DROP INDEX {index_name}; CREATE INDEX {} ON {qualified} USING gin ({replacement})",
                postgres::quote_ident(&name)
            ))
            .await
            .unwrap();
        let output = cli(&config, &directory, "drift", "verify", Some(&run)).await;
        assert_eq!(output.status.code(), Some(4), "{}", diagnostic(&output));
        let output = cli(&append, &directory, "refuse-drift", "plan", None).await;
        assert!(!output.status.success(), "{}", diagnostic(&output));
    }
    target
        .client
        .batch_execute(&format!("DROP INDEX {index_name}"))
        .await
        .unwrap();
    let output = cli(&config, &directory, "missing-index", "verify", Some(&run)).await;
    assert_eq!(output.status.code(), Some(4), "{}", diagnostic(&output));
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE {table}"))
        .await
        .unwrap();
    drop(source);
    drop(target);
    fs::remove_dir_all(directory).unwrap();
}
