use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, plan, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::PathBuf};
use tokio::sync::watch;
fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}
fn facts(catalog: &mut TargetCatalog) -> &mut TargetTableObserved {
    catalog
        .tables
        .iter_mut()
        .find(|t| t.name == "items")
        .unwrap()
        .observed
        .as_mut()
        .unwrap()
}
fn blocked(config: &MigrationConfig, source: &SourceCatalog, target: &TargetCatalog) {
    let plan::PlanError::Blocked(d) = plan::build(config, source, target).unwrap_err() else {
        panic!("expected metadata rejection")
    };
    assert!(
        d.iter().any(|d| matches!(
            d.code.as_str(),
            "TARGET_INDEX_UNSUPPORTED" | "TARGET_CONSTRAINT_UNSUPPORTED"
        )),
        "{d:?}"
    );
}
async fn run(config: &MigrationConfig) -> RunReport {
    let creds = resolve_credentials(config).unwrap();
    let args = Args::try_parse_from(["my2pg", "run", "fixture.toml", "--quiet"]).unwrap();
    let (_, receiver) = watch::channel(false);
    crate::test_pipeline::run(config, &creds, &mut Console::new(config, &args), receiver)
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn native_deferred_unique_extra_preserves_oid_aborts_duplicate_and_rejects_damaged_key_links()
{
    let name = format!("t11_deferred_{}", std::process::id());
    let schema = postgres::quote_ident(&name);
    let config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,"source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":name,"on_existing":"append"},
        "migration":{"mode":"data_only","reset_sequences":false,"batch_rows":1,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":[name],"rename":[{"source":name,"target":"items"}]},
        "report":{"directory":PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&name),"progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    source.query_drop(format!("CREATE TABLE {}(id INT NOT NULL PRIMARY KEY,value INT NOT NULL,grp INT NOT NULL) ENGINE=InnoDB",mysql::quote_ident(&name))).await.unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {} VALUES(1,11,1),(2,22,1)",
            mysql::quote_ident(&name)
        ))
        .await
        .unwrap();
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute(&format!("CREATE SCHEMA {schema};CREATE TABLE {schema}.items(id integer NOT NULL PRIMARY KEY,value integer NOT NULL,grp integer NOT NULL,CONSTRAINT deferred_extra UNIQUE(value,grp) DEFERRABLE INITIALLY DEFERRED,CONSTRAINT initially_immediate_extra UNIQUE(grp,id) DEFERRABLE INITIALLY IMMEDIATE)")).await.unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let before = postgres::inspect(&mut target.client, &name).await.unwrap();
    let prepared = plan::build(&config, &catalog, &before).unwrap();
    assert!(prepared.ddl.is_empty());
    assert!(
        !prepared.tables[0]
            .structure
            .indexes
            .iter()
            .any(|i| i.name == "deferred_extra" || i.name == "initially_immediate_extra"),
        "deferred extras cannot bind immediate source keys"
    );
    let mut cloned = before.clone();
    let original = facts(&mut cloned)
        .constraint_parts
        .iter()
        .find(|p| p.name == "deferred_extra")
        .unwrap()
        .clone();
    let oid = original.constraint_oid;
    let index_oid = original.supporting_index_oid.unwrap();
    facts(&mut cloned).constraint_parts.push(original);
    plan::build(&config, &catalog, &cloned)
        .expect("identical repeated key observation remains consistent");
    for fault in 0..16 {
        let mut bad = before.clone();
        let f = facts(&mut bad);
        match fault {
            0 => f.constraint_parts.retain(|p| p.constraint_oid != oid),
            1 => f.index_parts.retain(|p| p.index_oid != index_oid),
            2 => {
                f.constraint_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .supporting_index_oid = Some(0)
            }
            3 => {
                f.constraint_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .supporting_index_schema = Some("other".into())
            }
            4 => {
                f.constraint_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .column_name = Some("id".into())
            }
            5 => {
                f.index_parts
                    .iter_mut()
                    .find(|p| p.index_oid == index_oid)
                    .unwrap()
                    .ordinal = 99
            }
            6 => {
                f.index_parts
                    .iter_mut()
                    .find(|p| p.index_oid == index_oid)
                    .unwrap()
                    .valid = false
            }
            7 => {
                f.index_parts
                    .iter_mut()
                    .find(|p| p.index_oid == index_oid)
                    .unwrap()
                    .default_operator_class = Some(false)
            }
            8 => {
                let mut p = f
                    .constraint_parts
                    .iter()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .clone();
                p.initially_deferred = false;
                f.constraint_parts.push(p)
            }
            9 => {
                f.index_parts
                    .iter_mut()
                    .find(|p| p.index_oid == index_oid)
                    .unwrap()
                    .constraint_initially_deferred = Some(false)
            }
            10 => {
                f.constraint_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .validated = false
            }
            11 => {
                f.index_parts
                    .iter_mut()
                    .find(|p| p.index_oid == index_oid)
                    .unwrap()
                    .constraint_name = Some("unknown".into())
            }
            12 => {
                f.index_parts
                    .iter_mut()
                    .find(|p| p.index_oid == index_oid)
                    .unwrap()
                    .is_unique = false
            }
            13 => {
                for p in f
                    .index_parts
                    .iter_mut()
                    .filter(|p| p.index_oid == index_oid)
                {
                    p.table_name = "other".into();
                }
            }
            14 => {
                for p in f
                    .index_parts
                    .iter_mut()
                    .filter(|p| p.index_oid == index_oid)
                {
                    p.table_schema = "other".into();
                }
            }
            15 => {
                for p in f
                    .index_parts
                    .iter_mut()
                    .filter(|p| p.index_oid == index_oid)
                {
                    p.index_schema = "other".into();
                    p.table_schema = "other".into();
                }
                for p in f
                    .constraint_parts
                    .iter_mut()
                    .filter(|p| p.constraint_oid == oid)
                {
                    p.supporting_index_schema = Some("other".into());
                }
            }
            _ => unreachable!(),
        }
        blocked(&config, &catalog, &bad);
    }
    let report = run(&config).await;
    assert_eq!(report.status, RunStatus::Complete, "{report:?}");
    assert_eq!(report.tables[0].committed_rows, 2);
    let mut after = postgres::inspect(&mut target.client, &name).await.unwrap();
    let mut original = before.clone();
    assert_eq!(
        facts(&mut original).constraint_parts,
        facts(&mut after).constraint_parts
    );
    assert_eq!(
        facts(&mut original).index_parts,
        facts(&mut after).index_parts
    );
    // Duplicate is legal during COPY but fails this deferred UNIQUE at real COMMIT.
    source
        .query_drop(format!(
            "INSERT INTO {} VALUES(3,22,1)",
            mysql::quote_ident(&name)
        ))
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!("TRUNCATE {schema}.items"))
        .await
        .unwrap();
    let report = run(&config).await;
    assert_eq!(report.status, RunStatus::Failed, "{report:?}");
    assert_eq!(
        (
            report.tables[0].committed_rows,
            report.tables[0].indeterminate_rows
        ),
        (2, 0)
    );
    assert_eq!(
        (
            report.tables[0].rejected_rows,
            report.tables[0].unresolved_rows
        ),
        (0, 1)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("CommitAttempt") && d.message.contains("23505")),
        "{report:?}"
    );
    let ids: Vec<i32> = target
        .client
        .query(&format!("SELECT id FROM {schema}.items ORDER BY id"), &[])
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(ids, [1, 2]);
    let persisted: RunReport = serde_json::from_slice(
        &fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted.tables[0].committed_rows, 2);
    // A deferred target PRIMARY KEY must not satisfy this required immediate source key.
    target.client.batch_execute(&format!("ALTER TABLE {schema}.items DROP CONSTRAINT items_pkey;ALTER TABLE {schema}.items ADD CONSTRAINT deferred_pk PRIMARY KEY(id) DEFERRABLE INITIALLY DEFERRED")).await.unwrap();
    let current = postgres::inspect(&mut target.client, &name).await.unwrap();
    let plan::PlanError::Blocked(d) = plan::build(&config, &catalog, &current).unwrap_err() else {
        panic!("expected immediate key mismatch")
    };
    assert!(
        d.iter().any(|d| d.code == "TARGET_INDEX_INCOMPATIBLE"),
        "{d:?}"
    );
    assert!(
        !d.iter().any(|d| d.code == "TARGET_CONSTRAINT_UNSUPPORTED"),
        "valid deferredPK link is preserved metadata, while required-key policy fails: {d:?}"
    );
    // An identically shaped deferred UNIQUE also cannot satisfy a required source UNIQUE.
    source
        .query_drop(format!(
            "DELETE FROM {} WHERE id=3",
            mysql::quote_ident(&name)
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "ALTER TABLE {} ADD UNIQUE KEY required_value(value,grp)",
            mysql::quote_ident(&name)
        ))
        .await
        .unwrap();
    let required_catalog = mysql::inspect(&mut source).await.unwrap();
    let plan::PlanError::Blocked(d) = plan::build(&config, &required_catalog, &before).unwrap_err()
    else {
        panic!("expected immediate unique mismatch")
    };
    assert!(d.iter().any(|d| d.code == "TARGET_INDEX_INCOMPATIBLE"));
    // Builtin deferred keys are not user-trigger safety exceptions; unchanged T12 tests cover recovery.
    target
        .client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    source
        .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&name)))
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
