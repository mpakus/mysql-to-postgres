//! Mandatory native NLS lane; no locale absence is counted as success.
use my2pg::{
    config::{RowErrorPolicy, TargetConfig},
    model::{BatchStorage, EncodedBatch, RowLocator, RowPosition, TablePlan},
    pipeline::recovery::{self, RecoveryContext, RejectBudget},
    postgres,
    report::RunArtifacts,
};
use std::{env, fs, path::PathBuf, sync::Arc};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing mandatory {name}"))
}

#[tokio::test]
#[ignore = "requires separately pinned native PostgreSQL NLS fixture and real French locale"]
async fn actual_french_builtin_check_error_keeps_sqlstate_recovery_and_durable_reject() {
    let locale = required("MY2PG_NLS_LOCALE");
    assert!(
        locale.starts_with("fr_FR"),
        "this oracle requires real French"
    );
    let config: TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_POSTGRES_URL", "schema":"legacy", "ca_file":required("MY2PG_TLS_CA")
    }))
    .unwrap();
    let mut conn = postgres::connect(&config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let actual: String = conn
        .client
        .query_one("SELECT set_config('lc_messages',$1,false)", &[&locale])
        .await
        .unwrap()
        .get(0);
    assert_eq!(actual, locale);
    let schema = format!("t12_nls_{}", std::process::id());
    let qualified = postgres::qualified(&schema, "rows");
    conn.client
        .batch_execute(&format!(
            "CREATE SCHEMA {};CREATE TABLE {qualified}(n integer CHECK(n>=0))",
            postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
    let error = conn
        .client
        .batch_execute(&format!("INSERT INTO {qualified} VALUES(-99)"))
        .await
        .unwrap_err();
    let native = error.as_db_error().expect("actual builtin server error");
    assert_eq!(native.code().code(), "23514");
    assert!(
        native.message().contains("contrainte"),
        "native French CHECK message missing: {}",
        native.message()
    );
    assert!(!native.message().contains("violates check constraint"));
    let artifacts =
        RunArtifacts::create(&PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join("t12-nls"))
            .unwrap();
    fs::write(artifacts.directory.join("localized-error.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "locale": actual, "sqlstate": native.code().code(), "message": native.message(), "severity": native.severity()
    })).unwrap()).unwrap();
    let table: TablePlan = serde_json::from_value(serde_json::json!({
        "id":"t_0000000000000022", "source_name":"source", "target_schema":schema,
        "target_name":"rows", "engine":"InnoDB", "is_view":false, "estimated_rows":null,
        "next_auto_increment":null, "primary_key":[], "structure":my2pg::model::SchemaExpectations::default(),
        "columns":[{"source_name":"n", "target_name":"n", "source_type":"int", "target_type":"integer", "kind":"text",
            "nullable":true, "default_sql":null, "identity":false, "generated_expression":null, "copy":true,
            "transform":null, "charset":null, "comment":null, "enum_labels":[], "set_labels":[]}]
    })).unwrap();
    let bytes = b"1\n-1\n2\n";
    let permit = Arc::new(tokio::sync::Semaphore::new(bytes.len()))
        .acquire_many_owned(bytes.len() as u32)
        .await
        .unwrap();
    let batch = EncodedBatch {
        storage: Arc::new(BatchStorage {
            bytes: bytes.as_slice().into(),
            permit,
        }),
        rows: [(0, 2), (2, 5), (5, 7)]
            .into_iter()
            .enumerate()
            .map(|(index, (start, end))| RowPosition {
                start,
                end,
                locator: RowLocator {
                    ordinal: index as u64 + 1,
                    key: None,
                },
            })
            .collect(),
    };
    let budget = RejectBudget::new(2);
    let mut observer = |_| Ok(());
    let outcome = recovery::copy_with_recovery(
        &mut conn,
        &table,
        &batch,
        &mut RecoveryContext {
            policy: RowErrorPolicy::Reject,
            artifacts: &artifacts,
            reject_budget: &budget,
            observer: &mut observer,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (outcome.committed_rows, outcome.rejected_rows, budget.used()),
        (2, 1, 1)
    );
    assert_eq!(
        fs::read(
            artifacts
                .directory
                .join(format!("{}.reject.copy", table.id))
        )
        .unwrap(),
        b"-1\n"
    );
    let metadata = fs::read_to_string(
        artifacts
            .directory
            .join(format!("{}.reject.jsonl", table.id)),
    )
    .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(metadata.trim()).unwrap();
    assert_eq!(metadata["locator"]["ordinal"], 2);
    assert_eq!(metadata["reason"]["code"], "COPY_ROW_REJECTED");
    assert!(
        metadata["reason"]["message"]
            .as_str()
            .unwrap()
            .contains("23514")
    );
    artifacts.flush().unwrap();
    let actual: Vec<i32> = conn
        .client
        .query(&format!("SELECT n FROM {qualified} ORDER BY n"), &[])
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(actual, vec![1, 2]);
    conn.client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
    conn.close().await.unwrap();
}
