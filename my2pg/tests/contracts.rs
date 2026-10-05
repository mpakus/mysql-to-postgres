use my2pg::{config::*, model::*};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

fn report() -> RunReport {
    serde_json::from_str(include_str!("contracts/report.json")).unwrap()
}

fn round_trip<T: DeserializeOwned + Serialize>(fixture: &str) -> T {
    let expected: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let typed: T = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), expected);
    typed
}

#[test]
fn dependency_and_type_observations_distinguish_unknown_from_known_empty() {
    let unknown: TargetCatalog =
        serde_json::from_str(include_str!("contracts/target-catalog.json")).unwrap();
    assert!(unknown.type_parts.is_none());
    assert!(unknown.dependency_graph.is_none());
    let mut known = unknown;
    known.type_parts = Some(Vec::new());
    known.dependency_graph = Some(TargetDependencyGraph {
        database_oid: 16384,
        relation_root_count: 0,
        type_root_count: 0,
        event_trigger_count: 0,
        parts: Vec::new(),
    });
    let value = serde_json::to_value(&known).unwrap();
    assert_eq!(value["type_parts"], serde_json::json!([]));
    let decoded: TargetCatalog = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(decoded.type_parts, Some(Vec::new()));
    assert_eq!(decoded.dependency_graph, known.dependency_graph);
    for field in [
        "database_oid",
        "relation_root_count",
        "type_root_count",
        "event_trigger_count",
        "parts",
    ] {
        let mut incomplete = value.clone();
        incomplete["dependency_graph"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            serde_json::from_value::<TargetCatalog>(incomplete).is_err(),
            "missing {field}"
        );
    }
}

#[test]
fn configuration_credentials_remain_references_and_defaults_are_safe() {
    let config: MigrationConfig = toml::from_str(include_str!("contracts/config.toml")).unwrap();
    assert_eq!(config.source.url_env.as_deref(), Some("MY2PG_MYSQL_URL"));
    assert_eq!(config.source.tls_mode, TlsMode::VerifyFull);
    assert_eq!(config.target.schema, "legacy");
    assert_eq!(config.target.on_existing, ExistingPolicy::Error);
    assert_eq!(config.migration.table_workers, 1);
    assert_eq!(config.migration.on_row_error, RowErrorPolicy::Stop);
    assert_eq!(config.verification.mode, VerificationMode::CountsAndSchema);
    assert_eq!(config.tables.rename[0].source, "CamelCase");
    let encoded = toml::to_string(&config).unwrap();
    let decoded: MigrationConfig = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.tables.rename[0].target, "camel_case");
    assert!(!encoded.contains("mysql://"));
    assert!(!encoded.contains("postgres://"));
}

#[test]
fn catalog_plan_event_and_report_fixtures_preserve_identity_and_metadata() {
    let source: SourceCatalog = round_trip(include_str!("contracts/source-catalog.json"));
    assert_eq!(source.tables[0].name, "CamelCase");
    assert_eq!(source.tables[0].columns[0].name, "ID");
    assert_eq!(source.tables[0].next_auto_increment, Some(4_294_967_295));
    assert_eq!(
        source.tables[0].indexes[0].parts[0].column.as_deref(),
        Some("ID")
    );
    let target: TargetCatalog = round_trip(include_str!("contracts/target-catalog.json"));
    assert!(
        target.tables[0].observed.is_none(),
        "old metadata is uninspected, never known-empty"
    );
    assert_eq!(target.enums["ordered_enum"], ["second", "first"]);
    assert!(target.occupied_types.contains("untouched"));
    let plan: MigrationPlan = round_trip(include_str!("contracts/plan.json"));
    assert_eq!(plan.tables[0].columns[0].source_name, "ID");
    assert_eq!(plan.tables[0].columns[0].target_name, "id");
    assert_eq!(plan.verification, VerificationMode::CountsAndSchema);
    assert_eq!(plan.tables[0].primary_key, ["ID"]);
    assert_eq!(plan.tables[0].structure.sequences[0].max_value, i64::MAX);
    assert_eq!(
        plan.tables[0].structure.sequences[0].next_minimum,
        4_294_967_295
    );
    let event: RunEvent = round_trip(include_str!("contracts/event.json"));
    assert_eq!(event.progress.unwrap().quality, EstimateQuality::Metadata);
    let result: RunReport = round_trip(include_str!("contracts/report.json"));
    assert_eq!(result.exit_code(), 0);
}

#[test]
fn observed_empty_catalog_is_explicit_and_missing_inventory_never_becomes_empty() {
    let observed: TargetTableObserved = round_trip(include_str!("contracts/target-observed.json"));
    assert_eq!(observed.oid, 70001);
    assert_eq!(observed.owner, "fixture_owner");
    assert!(
        observed.columns.is_empty()
            && observed.index_parts.is_empty()
            && observed.constraint_parts.is_empty()
    );
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("contracts/target-observed.json")).unwrap();
    for inventory in [
        "columns",
        "index_parts",
        "constraint_parts",
        "sequence_parts",
        "trigger_parts",
    ] {
        let saved = value.as_object_mut().unwrap().remove(inventory).unwrap();
        assert!(serde_json::from_value::<TargetTableObserved>(value.clone()).is_err());
        value[inventory] = saved;
    }
}

#[test]
fn raw_values_round_trip_unsigned_max_binary_and_invalid_dates_losslessly() {
    let fixture = r#"[
        {"kind":"u_int","value":18446744073709551615},
        {"kind":"bytes","value":[0,1,92,9,255]},
        {"kind":"date","value":{"year":0,"month":0,"day":0,"hour":0,"minute":0,"second":0,"micros":0}},
        {"kind":"time","value":{"negative":true,"days":34,"hour":22,"minute":59,"second":59,"micros":999999}}
    ]"#;
    let values: Vec<RawValue> = round_trip(fixture);
    assert_eq!(values[0], RawValue::UInt(u64::MAX));
    assert_eq!(values[1], RawValue::Bytes(vec![0, 1, 92, 9, 255]));
    assert_eq!(
        values[3],
        RawValue::Time {
            negative: true,
            days: 34,
            hour: 22,
            minute: 59,
            second: 59,
            micros: 999_999,
        }
    );
}

#[test]
fn final_outcome_precedence_is_execution_then_cancel_then_verification_then_rejects() {
    let mut result = report();
    result.tables[0].committed_rows = 1;
    result.tables[0].rejected_rows = 1;
    assert_eq!(result.exit_code(), 3);
    result.verification.status = VerificationStatus::Different;
    assert_eq!(result.exit_code(), 4);
    result.status = RunStatus::Cancelled;
    assert_eq!(result.exit_code(), 130);
    result.tables[0].status = RunStatus::Indeterminate;
    assert_eq!(result.exit_code(), 1);
}

#[test]
fn missing_verification_and_unfinished_or_incomplete_accounting_cannot_succeed() {
    for status in [
        VerificationStatus::NotRun,
        VerificationStatus::Unsupported,
        VerificationStatus::Different,
    ] {
        let mut result = report();
        result.verification.status = status;
        assert_eq!(result.exit_code(), 4);
    }
    for status in [
        RunStatus::Running,
        RunStatus::Failed,
        RunStatus::Indeterminate,
    ] {
        let mut result = report();
        result.tables[0].status = status;
        assert_eq!(result.exit_code(), 1);
    }
    let mut result = report();
    result.tables[0].committed_rows = 1;
    assert_eq!(result.exit_code(), 1);
    result.tables[0].unresolved_rows = 1;
    assert_eq!(result.exit_code(), 1);
    result.status = RunStatus::Cancelled;
    result.tables[0].status = RunStatus::Cancelled;
    assert_eq!(result.exit_code(), 130);
    result.tables[0].indeterminate_rows = 1;
    assert_eq!(result.exit_code(), 1);
}

#[test]
fn overflowing_accounting_and_required_ddl_failure_are_execution_failures() {
    let mut result = report();
    result.tables[0].rows_read = u64::MAX;
    result.tables[0].committed_rows = u64::MAX;
    result.tables[0].rejected_rows = 1;
    assert_eq!(result.tables[0].accounted_rows(), None);
    assert_eq!(result.exit_code(), 1);
    let mut result = report();
    result
        .failed_steps
        .push("legacy.camel_case PRIMARY KEY".into());
    assert_eq!(result.exit_code(), 1);
}

#[test]
fn contradictory_verification_success_or_missing_table_coverage_never_succeeds() {
    let mut result = report();
    result
        .verification
        .differences
        .push("legacy.camel_case count differs".into());
    assert_eq!(result.exit_code(), 4);
    result.verification.differences.clear();
    result.verification.tables_checked = 0;
    assert_eq!(result.exit_code(), 4);
    result.verification.status = VerificationStatus::Error;
    result.status = RunStatus::Cancelled;
    assert_eq!(result.exit_code(), 1);
}

#[test]
fn json_validation_preserves_decimal_precision_beyond_binary_floats() {
    let source = r#"{"n":12345678901234567890123456789012345.123456789012345678901234567890}"#;
    let value: serde_json::Value = serde_json::from_str(source).unwrap();
    assert_eq!(
        value["n"].to_string(),
        "12345678901234567890123456789012345.123456789012345678901234567890"
    );
    assert_eq!(serde_json::to_string(&value).unwrap(), source);
}

#[tokio::test]
async fn recovery_slices_retain_one_buffer_and_its_budget_until_last_drop() {
    let budget = Arc::new(tokio::sync::Semaphore::new(16));
    let permit = budget.clone().acquire_many_owned(8).await.unwrap();
    let batch = EncodedBatch {
        storage: Arc::new(BatchStorage {
            bytes: bytes::Bytes::from_static(b"a\tb\nc\td\n"),
            permit,
        }),
        rows: vec![
            RowPosition {
                start: 0,
                end: 4,
                locator: RowLocator {
                    ordinal: 1,
                    key: None,
                },
            },
            RowPosition {
                start: 4,
                end: 8,
                locator: RowLocator {
                    ordinal: 2,
                    key: None,
                },
            },
        ],
    };
    let slice = EncodedBatch {
        storage: batch.storage.clone(),
        rows: batch.rows[1..].to_vec(),
    };
    assert!(Arc::ptr_eq(&batch.storage, &slice.storage));
    assert_eq!(
        &slice.storage.bytes[slice.rows[0].start..slice.rows[0].end],
        b"c\td\n"
    );
    drop(batch);
    assert_eq!(budget.available_permits(), 8);
    drop(slice);
    assert_eq!(budget.available_permits(), 16);
}

#[test]
fn binary_help_and_version_work_without_database_connections() {
    for flag in ["--help", "--version"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("my2pg"));
    }
}

#[test]
fn private_artifact_worker_dispatches_before_clap_and_rejects_extra_arguments() {
    use std::process::Stdio;

    let marker = my2pg::report::artifact_io::WORKER_ARGUMENT;
    let protocol_eof = std::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg(marker)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(protocol_eof.status.code(), Some(1));
    assert!(protocol_eof.stdout.is_empty());
    let diagnostic = String::from_utf8(protocol_eof.stderr).unwrap();
    assert!(diagnostic.contains("ARTIFACT_WORKER"));
    assert!(!diagnostic.contains("Usage:"));

    let extra_argument = std::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .args([marker, "private-argument-must-not-be-echoed"])
        .output()
        .unwrap();
    assert_eq!(extra_argument.status.code(), Some(2));
    assert!(extra_argument.stdout.is_empty());
    let diagnostic = String::from_utf8(extra_argument.stderr).unwrap();
    assert!(diagnostic.contains("ARTIFACT_WORKER"));
    assert!(!diagnostic.contains("Usage:"));
    assert!(!diagnostic.contains("private-argument-must-not-be-echoed"));
}

#[test]
fn check_succeeds_offline_and_never_serializes_resolved_passwords() {
    // Port 1 has no fixture service. Successful check must not open a connection.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .args(["check", "tests/contracts/config.toml", "--output", "json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env(
            "MY2PG_MYSQL_URL",
            "mysql://user:source-secret@127.0.0.1:1/source",
        )
        .env(
            "MY2PG_POSTGRES_URL",
            "postgresql://user:target-secret@127.0.0.1:1/target",
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let result: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result["operation"], "check");
    assert_eq!(result["status"], "valid");
    assert_eq!(result["config"]["target"]["schema"], "legacy");
    assert!(!text.contains("source-secret"));
    assert!(!text.contains("target-secret"));
    assert!(output.stderr.is_empty());
}

#[test]
fn excluded_source_fails_before_target_credential_lookup_and_redacts_input() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .args(["check", "tests/contracts/config.toml"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env(
            "MY2PG_MYSQL_URL",
            "sqlite://user:source-secret@127.0.0.1:1/source",
        )
        .env_remove("MY2PG_POSTGRES_URL")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("source"));
    assert!(!diagnostic.contains("target.url_env"));
    assert!(!diagnostic.contains("source-secret"));
    assert!(output.stdout.is_empty());
}
