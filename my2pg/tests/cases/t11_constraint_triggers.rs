//! Existing constraint triggers are preserved by positive catalog linkage, never translated.
use clap::Parser;
use my2pg::{cli::Args, config::*, model::*, mysql, plan, postgres, report::Console};
use mysql_async::prelude::Queryable;
use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::watch;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}
struct Case {
    config: MigrationConfig,
    source: mysql::SourceConnection,
    target: postgres::TargetConnection,
    name: String,
    directory: PathBuf,
}
impl Case {
    async fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "t11ct_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let directory = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(&name);
        fs::create_dir_all(&directory).unwrap();
        let config: MigrationConfig=serde_json::from_value(serde_json::json!({
            "version":1,"source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
            "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":name,"on_existing":"append"},
            "migration":{"mode":"data_only","reset_sequences":false,"batch_rows":1,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
            "tables":{"include":[name],"rename":[{"source":name,"target":"items"}]},
            "report":{"directory":directory.join("runs"),"progress":"never"}
        })).unwrap();
        let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
            .await
            .unwrap();
        source
            .query_drop(format!(
                "CREATE TABLE {}(id INT NOT NULL PRIMARY KEY,value INT NOT NULL) ENGINE=InnoDB",
                mysql::quote_ident(&name)
            ))
            .await
            .unwrap();
        source
            .query_drop(format!(
                "INSERT INTO {} VALUES(1,11),(2,22),(3,33)",
                mysql::quote_ident(&name)
            ))
            .await
            .unwrap();
        let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap();
        let schema = postgres::quote_ident(&name);
        target.client.batch_execute(&format!("CREATE SCHEMA {schema};CREATE TABLE {schema}.items(id integer NOT NULL PRIMARY KEY,value integer NOT NULL);CREATE TABLE {schema}.audit(id integer,label text);CREATE SEQUENCE {schema}.attempts")).await.unwrap();
        Self {
            config,
            source,
            target,
            name,
            directory,
        }
    }
    fn schema(&self) -> String {
        postgres::quote_ident(&self.name)
    }
    async fn triggers(&self, failure: Option<&str>) {
        let s = self.schema();
        let failure=failure.map(|state|format!("IF NEW.id=2 THEN RAISE EXCEPTION 'owned constraint failure' USING ERRCODE='{state}'; END IF;")).unwrap_or_default();
        self.target.client.batch_execute(&format!("CREATE FUNCTION {s}.record() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN INSERT INTO {s}.audit VALUES(NEW.id,TG_NAME);RETURN NEW;END$$;CREATE TRIGGER regular AFTER INSERT ON {s}.items FOR EACH ROW EXECUTE FUNCTION {s}.record();CREATE CONSTRAINT TRIGGER immediate_constraint AFTER INSERT ON {s}.items NOT DEFERRABLE FOR EACH ROW EXECUTE FUNCTION {s}.record();CREATE FUNCTION {s}.deferred_record() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN PERFORM nextval('{s}.attempts');{failure} INSERT INTO {s}.audit VALUES(NEW.id,TG_NAME);RETURN NEW;END$$;CREATE CONSTRAINT TRIGGER deferred_constraint AFTER INSERT ON {s}.items DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION {s}.deferred_record()")).await.unwrap();
    }
    async fn catalogs(&mut self) -> (SourceCatalog, TargetCatalog) {
        (
            mysql::inspect(&mut self.source).await.unwrap(),
            postgres::inspect(&mut self.target.client, &self.name)
                .await
                .unwrap(),
        )
    }
    async fn count(&self, table: &str) -> i64 {
        self.target
            .client
            .query_one(
                &format!("SELECT count(*) FROM {}.{}", self.schema(), table),
                &[],
            )
            .await
            .unwrap()
            .get(0)
    }
    async fn execute(&self) -> RunReport {
        let creds = resolve_credentials(&self.config).unwrap();
        let args = Args::try_parse_from(["my2pg", "run", "fixture.toml", "--quiet"]).unwrap();
        let mut console = Console::new(&self.config, &args);
        let (_, receiver) = watch::channel(false);
        crate::test_pipeline::run(&self.config, &creds, &mut console, receiver)
            .await
            .unwrap()
    }
    async fn cleanup(mut self) {
        self.target
            .client
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema()))
            .await
            .unwrap();
        self.source
            .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&self.name)))
            .await
            .unwrap();
        self.source.disconnect().await.unwrap();
        self.target.close().await.unwrap();
    }
}
fn observed(catalog: &mut TargetCatalog) -> &mut TargetTableObserved {
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
    let plan::PlanError::Blocked(diagnostics) = plan::build(config, source, target).unwrap_err()
    else {
        panic!("expected typed preflight rejection")
    };
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "TARGET_CONSTRAINT_UNSUPPORTED"),
        "{diagnostics:?}"
    );
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn native_linked_constraint_trigger_preservation_firing_and_metadata_fail_closed() {
    let mut case = Case::new("preserve").await;
    case.triggers(None).await;
    let (source, before) = case.catalogs().await;
    let prepared = plan::build(&case.config, &source, &before).unwrap();
    assert!(prepared.ddl.is_empty());
    assert!(
        prepared
            .diagnostics
            .iter()
            .any(|d| d.code == "TARGET_TRIGGER_PRESERVED")
    );
    let facts = before
        .tables
        .iter()
        .find(|t| t.name == "items")
        .unwrap()
        .observed
        .as_ref()
        .unwrap();
    let headers: Vec<_> = facts
        .constraint_parts
        .iter()
        .filter(|p| p.kind == "t")
        .collect();
    assert_eq!(headers.len(), 2);
    assert!(headers.iter().any(|p| p.deferrable && p.initially_deferred));
    assert!(
        headers
            .iter()
            .any(|p| !p.deferrable && !p.initially_deferred)
    );
    for header in &headers {
        assert!(
            facts
                .trigger_parts
                .iter()
                .any(|t| t.constraint_oid == Some(header.constraint_oid)
                    && t.internal == Some(false))
        );
    }
    let oid = headers[0].constraint_oid;
    let mut same = before.clone();
    let header = observed(&mut same)
        .constraint_parts
        .iter()
        .find(|p| p.constraint_oid == oid)
        .unwrap()
        .clone();
    observed(&mut same).constraint_parts.push(header);
    let trigger = observed(&mut same)
        .trigger_parts
        .iter()
        .find(|p| p.constraint_oid == Some(oid))
        .unwrap()
        .clone();
    observed(&mut same).trigger_parts.push(trigger);
    plan::build(&case.config, &source, &same)
        .expect("consistent repeated observation must preserve");
    for fault in 0..12 {
        let mut bad = before.clone();
        let facts = observed(&mut bad);
        match fault {
            0 => facts
                .trigger_parts
                .retain(|p| p.constraint_oid != Some(oid)),
            1 => facts.constraint_parts.retain(|p| p.constraint_oid != oid),
            2 => {
                facts
                    .constraint_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .validated = false
            }
            3 => {
                facts
                    .trigger_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == Some(oid))
                    .unwrap()
                    .enabled = "?".into()
            }
            4 => {
                facts
                    .trigger_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == Some(oid))
                    .unwrap()
                    .table_oid = Some(0)
            }
            5 => {
                facts
                    .trigger_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == Some(oid))
                    .unwrap()
                    .constraint_name = Some("other".into())
            }
            6 => {
                facts
                    .trigger_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == Some(oid))
                    .unwrap()
                    .internal = Some(true)
            }
            7 => {
                facts
                    .trigger_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == Some(oid))
                    .unwrap()
                    .event_flags = Some(3)
            }
            8 => {
                let mut p = facts
                    .constraint_parts
                    .iter()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .clone();
                p.deferrable = !p.deferrable;
                facts.constraint_parts.push(p);
            }
            9 => {
                let mut p = facts
                    .trigger_parts
                    .iter()
                    .find(|p| p.constraint_oid == Some(oid))
                    .unwrap()
                    .clone();
                p.trigger_oid += 100000;
                facts.trigger_parts.push(p);
            }
            10 => {
                let mut p = facts
                    .trigger_parts
                    .iter()
                    .find(|p| p.constraint_oid == Some(oid))
                    .unwrap()
                    .clone();
                p.function_config = Some(vec!["search_path=public".into()]);
                facts.trigger_parts.push(p);
            }
            11 => {
                facts
                    .constraint_parts
                    .iter_mut()
                    .find(|p| p.constraint_oid == oid)
                    .unwrap()
                    .kind = "?".into()
            }
            _ => unreachable!(),
        }
        blocked(&case.config, &source, &bad);
    }
    assert_eq!(case.count("items").await, 0);
    let path = case.directory.join("migration.toml");
    fs::write(&path, toml::to_string(&case.config).unwrap()).unwrap();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(path)
            .args(["--output", "json", "--progress", "never"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(case.count("items").await, 3);
    assert_eq!(case.count("audit").await, 9);
    let labels: Vec<(String, i64)> = case
        .target
        .client
        .query(
            &format!(
                "SELECT label,count(*) FROM {}.audit GROUP BY label ORDER BY label",
                case.schema()
            ),
            &[],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    assert_eq!(
        labels,
        vec![
            ("deferred_constraint".into(), 3),
            ("immediate_constraint".into(), 3),
            ("regular".into(), 3)
        ]
    );
    let (_, after) = case.catalogs().await;
    assert_eq!(
        facts.constraint_parts,
        after
            .tables
            .iter()
            .find(|t| t.name == "items")
            .unwrap()
            .observed
            .as_ref()
            .unwrap()
            .constraint_parts
    );
    assert_eq!(
        facts.trigger_parts,
        after
            .tables
            .iter()
            .find(|t| t.name == "items")
            .unwrap()
            .observed
            .as_ref()
            .unwrap()
            .trigger_parts
    );
    // NOT VALID CHECK stays blocked; a positively linked deferred UNIQUE is preserved.
    case.target
        .client
        .batch_execute(&format!(
            "ALTER TABLE {}.items ADD CONSTRAINT not_valid_check CHECK(value>0) NOT VALID",
            case.schema()
        ))
        .await
        .unwrap();
    let (_, invalid) = case.catalogs().await;
    blocked(&case.config, &source, &invalid);
    case.target.client.batch_execute(&format!("ALTER TABLE {}.items DROP CONSTRAINT not_valid_check;ALTER TABLE {}.items ADD CONSTRAINT deferred_unique UNIQUE(value) DEFERRABLE",case.schema(),case.schema())).await.unwrap();
    let (_, deferred) = case.catalogs().await;
    let preserved = plan::build(&case.config, &source, &deferred).unwrap();
    assert!(preserved.ddl.is_empty());
    let selected = deferred
        .tables
        .iter()
        .find(|t| t.name == "items")
        .unwrap()
        .observed
        .as_ref()
        .unwrap();
    let key = selected
        .constraint_parts
        .iter()
        .find(|p| p.name == "deferred_unique")
        .unwrap();
    assert!(key.constraint_oid > 0 && key.kind == "u" && key.deferrable);
    assert!(
        selected
            .index_parts
            .iter()
            .any(|p| Some(p.index_oid) == key.supporting_index_oid
                && p.constraint_name.as_deref() == Some("deferred_unique")
                && !p.immediate)
    );
    case.cleanup().await;
}

#[tokio::test]
#[ignore = "requires owned native MySQL84/PG16 verified TLS fixture"]
async fn native_deferred_constraint_error_retains_ack_and_custom_retry_is_not_replayed() {
    for state in ["23514", "40001"] {
        let case = Case::new(state).await;
        case.triggers(Some(state)).await;
        let report = case.execute().await;
        assert_eq!(report.status, RunStatus::Failed, "{report:?}");
        assert_eq!(report.tables[0].committed_rows, 1, "{report:?}");
        assert_eq!(report.tables[0].indeterminate_rows, 0, "{report:?}");
        assert!(
            report.diagnostics.iter().any(|d| d.message.contains(state)),
            "native safe SQLSTATE must remain in the final failure diagnostic: {report:?}"
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|d| !d.message.contains("owned constraint failure")),
            "raw PostgreSQL exception text must not enter the report"
        );
        assert_eq!(case.count("items").await, 1);
        assert_eq!(case.count("audit").await, 3);
        let attempts: i64 = case
            .target
            .client
            .query_one(
                &format!("SELECT last_value FROM {}.attempts", case.schema()),
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(attempts, 2, "custom trigger must never cause replay");
        let persisted: RunReport = serde_json::from_slice(
            &fs::read(PathBuf::from(&report.artifact_dir).join("report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted.tables[0].committed_rows, 1);
        // Independently prove the native deferred COMMIT error state, without matching localized text.
        case.target
            .client
            .batch_execute(&format!(
                "BEGIN;INSERT INTO {}.items VALUES(2,22)",
                case.schema()
            ))
            .await
            .unwrap();
        let error = case
            .target
            .client
            .batch_execute("COMMIT")
            .await
            .unwrap_err();
        assert_eq!(error.code().map(|c| c.code()), Some(state));
        case.target.client.batch_execute("ROLLBACK").await.unwrap();
        assert_eq!(case.count("items").await, 1);
        let mut reject = case.config.clone();
        reject.migration.on_row_error = RowErrorPolicy::Reject;
        reject.migration.max_rejected_rows = 10;
        let creds = resolve_credentials(&reject).unwrap();
        let args = Args::try_parse_from(["my2pg", "run", "fixture.toml", "--quiet"]).unwrap();
        let (_, receiver) = watch::channel(false);
        assert!(
            crate::test_pipeline::run(&reject, &creds, &mut Console::new(&reject, &args), receiver)
                .await
                .is_err(),
            "reject mode must retain actual-trigger safety gate before COPY"
        );
        assert_eq!(case.count("items").await, 1);
        let after: i64 = case
            .target
            .client
            .query_one(
                &format!("SELECT last_value FROM {}.attempts", case.schema()),
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(after, 3);
        case.cleanup().await;
    }
}
