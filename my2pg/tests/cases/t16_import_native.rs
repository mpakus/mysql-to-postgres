//! Importer's runnable semantics, independently observed through native SQL.
use my2pg::{
    config::{self, MigrationConfig, ObjectOverride},
    model::RunReport,
    mysql, postgres,
};
use mysql_async::prelude::Queryable;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required native fixture variable {name} absent"))
}
struct Directory(PathBuf);
impl Directory {
    fn new(label: &str) -> Self {
        let root = PathBuf::from(required("MY2PG_ARTIFACT_DIR")).join(format!(
            "{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("preserved T16 evidence: {}", self.0.display());
        }
    }
}
fn head() -> &'static str {
    "LOAD DATABASE FROM mysql://reader:legacy%2Dsource%2Dsecret@127.0.0.1:1/source INTO postgresql://writer:legacy%2Dtarget%2Dsecret@127.0.0.1:1/target "
}
fn secret_free(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for secret in [
        "legacy-source-secret",
        "legacy-target-secret",
        "legacy%2Dsource%2Dsecret",
        "legacy%2Dtarget%2Dsecret",
        "integration-only",
        "integration-root",
    ] {
        assert!(
            !text.contains(secret),
            "credential disclosure in import/migration output"
        );
    }
}
async fn cli(args: &[&str], overrides: &[(&str, &str)]) -> Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_my2pg"));
    command.args(args).kill_on_drop(true);
    for (key, value) in overrides {
        command.env(key, value);
    }
    let output = tokio::time::timeout(Duration::from_secs(60), command.output())
        .await
        .expect("owned CLI deadline exceeded")
        .unwrap();
    secret_free(&output.stdout);
    secret_free(&output.stderr);
    output
}
fn successful(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
async fn import(
    directory: &Directory,
    label: &str,
    schema: &str,
    clauses: &str,
    append: bool,
) -> MigrationConfig {
    let input = directory.0.join(format!("{label}.load"));
    let output = directory.0.join(format!("{label}.toml"));
    fs::write(&input, format!("{}{};", head(), clauses)).unwrap();
    let mut args = vec![
        "config",
        "import",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--target-schema",
        schema,
        "--consistency",
        "single_snapshot",
        "--source-env",
        "MY2PG_MYSQL_URL",
        "--target-env",
        "MY2PG_POSTGRES_URL",
        "--use-my2pg-defaults",
    ];
    if append {
        args.push("--append-data-only");
    }
    let result = cli(&args, &[]).await;
    successful(&result);
    let compatibility: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(compatibility["default_policy"], "my2pg_reviewed");
    assert!(
        compatibility["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("case-sensitive"))
    );
    let bytes = fs::read(&output).unwrap();
    secret_free(&bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let mut config = config::load(&output).unwrap();
    assert_eq!(config.source.url_env.as_deref(), Some("MY2PG_MYSQL_URL"));
    assert_eq!(config.target.url_env.as_deref(), Some("MY2PG_POSTGRES_URL"));
    // Fixture transport/output paths only; all imported semantic decisions are retained.
    config.source.ca_file = Some(required("MY2PG_TLS_CA").into());
    config.target.ca_file = Some(required("MY2PG_TLS_CA").into());
    config.report.directory = directory.0.join("runs");
    config
}
fn runnable(directory: &Directory, label: &str, config: &MigrationConfig) -> PathBuf {
    let path = directory.0.join(format!("{label}.runnable.toml"));
    fs::write(&path, toml::to_string(config).unwrap()).unwrap();
    path
}
fn acknowledge_policy_collation(config: &mut MigrationConfig) {
    assert!(config.overrides.is_empty());
    config.overrides.push(ObjectOverride {
        object: "source.T16ImportPolicy.value.collation".into(),
        omit: true,
        target_expression: None,
        target_sql: None,
        materialize: None,
    });
}
async fn run(path: &Path) -> Output {
    cli(
        &[
            "run",
            path.to_str().unwrap(),
            "--output",
            "json",
            "--progress",
            "never",
        ],
        &[],
    )
    .await
}
fn outcome(output: &Output) -> RunReport {
    let envelope = String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|value| value["kind"] == "outcome")
        .expect("final outcome absent");
    serde_json::from_value(envelope["report"].clone()).unwrap()
}
async fn source(config: &MigrationConfig) -> mysql::SourceConnection {
    mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_ROOT_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap()
}
async fn target(config: &MigrationConfig) -> postgres::TargetConnection {
    postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap()
}
async fn source_catalog_precondition(config: &MigrationConfig) {
    let mut reader = mysql::connect(
        &config.source,
        &required("MY2PG_MYSQL_URL"),
        config.migration.max_row_bytes,
    )
    .await
    .unwrap();
    if let Err(error) = mysql::inspect(&mut reader).await {
        let evidence = format!("{error:?}");
        secret_free(evidence.as_bytes());
        panic!("native source catalog precondition: {evidence}");
    }
    reader.disconnect().await.unwrap();
}
async fn relation_oid(target: &postgres::TargetConnection, name: &str) -> u32 {
    target
        .client
        .query_one("SELECT $1::text::regclass::oid", &[&name])
        .await
        .unwrap()
        .get(0)
}
async fn protected(target: &postgres::TargetConnection) -> (u32, String) {
    (
        relation_oid(target, "public.users").await,
        target
            .client
            .query_one("SELECT sentinel FROM public.users WHERE id=999", &[])
            .await
            .unwrap()
            .get(0),
    )
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL8.4/PostgreSQL16 TLS harness"]
async fn imported_cli_casts_filters_renames_and_defaults_have_native_fidelity() {
    let directory = Directory::new("t16-import-values");
    let clauses = r#"
 WITH include no drop, no truncate, create tables, create indexes, foreign keys, reset sequences, quote identifiers, workers=1, batch rows=1, max parallel create index=1
 SET MYSQL PARAMETERS time_zone TO '+00:00', wait_timeout TO '60'
 SET POSTGRESQL PARAMETERS timezone TO 'UTC', datestyle TO 'ISO, YMD'
 CAST column T16ImportParent.flag TO boolean USING tinyint-to-boolean,
      column T16ImportParent.hex_value TO bytea DROP DEFAULT USING hex-to-bytea,
      column T16ImportParent.empty_value DROP DEFAULT DROP NOT NULL USING empty-string-to-null,
      column T16ImportParent.dirty TO text DROP TYPEMOD USING remove-null-characters,
      column T16ImportChild.parent_id TO integer,
      type decimal WHEN (and (= precision 18) (= scale 6)) TO numeric(18,6),
      type decimal TO numeric(30,12),
      type int TO numeric(20,0)
 INCLUDING ONLY TABLE NAMES MATCHING ~/^T16Import/
 EXCLUDING TABLE NAMES MATCHING 'T16ImportIgnored'
 ALTER SCHEMA 'source' RENAME TO 't16_import_values'
 ALTER TABLE NAMES MATCHING 'T16ImportParent' RENAME TO 'parent'
 ALTER TABLE NAMES MATCHING 'T16ImportChild' RENAME TO 'child'
 "#;
    let config = import(&directory, "values", "t16_import_values", clauses, false).await;
    let path = runnable(&directory, "values", &config);
    // Offline check must not contact the deliberately unreachable child-only URLs.
    let check = cli(
        &["check", path.to_str().unwrap(), "--output", "json"],
        &[
            (
                "MY2PG_MYSQL_URL",
                "mysql://reader:offline@127.0.0.1:1/source",
            ),
            (
                "MY2PG_POSTGRES_URL",
                "postgresql://writer:offline@127.0.0.1:1/target",
            ),
        ],
    )
    .await;
    successful(&check);
    let mut source = source(&config).await;
    for statement in [
        "DROP TABLE IF EXISTS T16ImportChild",
        "DROP TABLE IF EXISTS T16ImportPolicy",
        "DROP TABLE IF EXISTS T16ImportParent",
        "DROP TABLE IF EXISTS T16ImportIgnored",
        "CREATE TABLE T16ImportParent(id INT NOT NULL AUTO_INCREMENT PRIMARY KEY,plain_id INT NOT NULL DEFAULT 7,implicit_flag TINYINT(1) DEFAULT 2,flag TINYINT(1) DEFAULT 0,unsigned_value BIGINT UNSIGNED NOT NULL,small_decimal DECIMAL(18,6) DEFAULT 1.250000,large_decimal DECIMAL(30,12) DEFAULT 12.500000000000,hex_value VARCHAR(30),empty_value VARCHAR(20) NOT NULL DEFAULT '',dirty VARCHAR(30),wall DATETIME(6),duration TIME(6),state ENUM('alpha','beta') DEFAULT 'alpha',memberships SET('alpha','beta') DEFAULT 'alpha',note VARCHAR(20) COMMENT 'column kept',UNIQUE KEY uq_unsigned(unsigned_value)) ENGINE=InnoDB AUTO_INCREMENT=500 COMMENT='parent kept'",
        "CREATE TABLE T16ImportChild(id INT PRIMARY KEY,parent_id INT NOT NULL,KEY ix_parent(parent_id),CONSTRAINT fk_parent FOREIGN KEY(parent_id) REFERENCES T16ImportParent(id) ON DELETE CASCADE ON UPDATE RESTRICT) ENGINE=InnoDB",
        "CREATE TABLE T16ImportIgnored(id INT) ENGINE=MyISAM",
        "INSERT INTO T16ImportParent(id,unsigned_value,flag,hex_value,empty_value,dirty,wall,duration,state,memberships,note) VALUES(7,18446744073709551615,2,'DEADBEEF','',CONCAT('a',CHAR(0),'b'),'2024-02-29 12:34:56.123456','-838:59:58.123456','beta','alpha,beta','one'),(10,42,0,'00ff','kept','clean','2024-03-01 00:00:00.000001','25:00:00.000001','alpha','beta','two')",
        "INSERT INTO T16ImportChild VALUES(1,7),(2,10)",
    ] {
        source.query_drop(statement).await.unwrap();
    }
    let direct: Option<(Option<String>, Option<String>)> = source
        .query_first(
            "SELECT HEX(DEFAULT(empty_value)),HEX(DEFAULT(state)) FROM T16ImportParent LIMIT 1",
        )
        .await
        .unwrap();
    let outer: Option<(Option<String>,Option<String>)> = source.query_first("SELECT HEX(CONVERT(DEFAULT(s.empty_value) USING utf8mb4)),HEX(CONVERT(DEFAULT(s.state) USING utf8mb4)) FROM T16ImportParent s RIGHT JOIN (SELECT 1 AS anchor) a ON FALSE").await.unwrap();
    let alias: Option<(Option<String>,u8)> = source.query_first("SELECT HEX(CONVERT(DEFAULT(s.empty_value) USING utf8mb4)),DEFAULT(s.empty_value) IS NULL FROM T16ImportParent s LIMIT 1").await.unwrap();
    let server_null: Option<(u8,u8,String)> = source.query_first("SELECT DEFAULT(s.empty_value) IS NULL,CONVERT(DEFAULT(s.empty_value) USING utf8mb4) IS NULL,COALESCE(HEX(CONVERT(DEFAULT(s.empty_value) USING utf8mb4)),'SERVER_SQL_NULL') FROM T16ImportParent s RIGHT JOIN (SELECT 1 AS anchor) a ON FALSE").await.unwrap();
    let metadata:Option<(Option<String>,String)> = source.query_first("SELECT HEX(COLUMN_DEFAULT),IS_NULLABLE FROM information_schema.COLUMNS WHERE TABLE_SCHEMA='source' AND TABLE_NAME='T16ImportParent' AND COLUMN_NAME='empty_value'").await.unwrap();
    type CoalescedDefaults = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let coalesced: Option<CoalescedDefaults> = source.query_first("SELECT CONVERT(DEFAULT(s.empty_value) USING utf8mb4),COALESCE(CONVERT(DEFAULT(s.empty_value) USING utf8mb4),'SERVER_SQL_NULL'),COALESCE(DEFAULT(s.empty_value),'SERVER_SQL_NULL'),HEX(DEFAULT(s.empty_value)) FROM T16ImportParent s RIGHT JOIN (SELECT 1 AS anchor) a ON FALSE").await.unwrap();
    source
        .query_drop("DROP TABLE IF EXISTS T16DefaultProbeEmpty")
        .await
        .unwrap();
    source.query_drop("CREATE TABLE T16DefaultProbeEmpty(value VARCHAR(20) NOT NULL DEFAULT '') ENGINE=InnoDB").await.unwrap();
    let empty_outer:Option<(u8,u8,String)> = source.query_first("SELECT DEFAULT(s.value) IS NULL,CONVERT(DEFAULT(s.value) USING utf8mb4) IS NULL,COALESCE(HEX(CONVERT(DEFAULT(s.value) USING utf8mb4)),'SERVER_SQL_NULL') FROM T16DefaultProbeEmpty s RIGHT JOIN (SELECT 1 AS anchor) a ON FALSE").await.unwrap();
    source
        .query_drop("DROP TABLE T16DefaultProbeEmpty")
        .await
        .unwrap();
    eprintln!(
        "independent native literal-default probes: direct={direct:?}; alias={alias:?}; false_outer={outer:?}; server_null={server_null:?}; metadata={metadata:?}; coalesced={coalesced:?}; empty_outer={empty_outer:?}"
    );
    let target = target(&config).await;
    let sentinel = protected(&target).await;
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t16_import_values CASCADE")
        .await
        .unwrap();
    source_catalog_precondition(&config).await;
    let source_before:String=source.query_first("SELECT CONCAT((SELECT COUNT(*) FROM T16ImportParent),':',(SELECT COUNT(*) FROM T16ImportChild),':',(SELECT COUNT(*) FROM T16ImportIgnored))").await.unwrap().unwrap();
    let conflicting_clauses =
        clauses.replace("      column T16ImportChild.parent_id TO integer,\n", "");
    assert_ne!(conflicting_clauses, clauses);
    let conflicting = import(
        &directory,
        "fk-incompatible",
        "t16_import_values",
        &conflicting_clauses,
        false,
    )
    .await;
    assert!(
        conflicting
            .cast
            .iter()
            .any(|rule| rule.source_type.as_deref() == Some("int")
                && rule.target_type.as_deref() == Some("numeric(20,0)"))
    );
    assert!(
        !conflicting.cast.iter().any(
            |rule| rule.source_table.as_deref() == Some("T16ImportChild")
                && rule.source_column.as_deref() == Some("parent_id")
        )
    );
    assert!(!conflicting.report.directory.exists());
    let refused = run(&runnable(&directory, "fk-incompatible", &conflicting)).await;
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("FK_TYPE_INCOMPATIBLE"));
    assert!(!conflicting.report.directory.exists());
    assert!(
        target
            .client
            .query_one("SELECT to_regnamespace('t16_import_values') IS NULL", &[])
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    assert_eq!(protected(&target).await, sentinel);
    let source_after_refusal:String=source.query_first("SELECT CONCAT((SELECT COUNT(*) FROM T16ImportParent),':',(SELECT COUNT(*) FROM T16ImportChild),':',(SELECT COUNT(*) FROM T16ImportIgnored))").await.unwrap().unwrap();
    assert_eq!(source_after_refusal, source_before);
    let plan = cli(&["plan", path.to_str().unwrap(), "--output", "json"], &[]).await;
    successful(&plan);
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    let tables = plan["plan"]["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 2);
    assert!(
        tables
            .iter()
            .all(|t| t["source_name"] != "T16ImportIgnored")
    );
    let parent = tables
        .iter()
        .find(|t| t["target_name"] == "parent")
        .unwrap();
    let columns = parent["columns"].as_array().unwrap();
    let child = tables
        .iter()
        .find(|table| table["target_name"] == "child")
        .unwrap();
    assert_eq!(
        child["columns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|column| column["source_name"] == "parent_id")
            .unwrap()["target_type"],
        "integer"
    );
    assert_eq!(
        columns.iter().find(|c| c["source_name"] == "id").unwrap()["target_type"],
        "integer"
    );
    assert_eq!(
        columns
            .iter()
            .find(|c| c["source_name"] == "plain_id")
            .unwrap()["target_type"],
        "numeric(20,0)"
    );
    assert!(
        target
            .client
            .query_one("SELECT to_regnamespace('t16_import_values') IS NULL", &[])
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    assert_eq!(protected(&target).await, sentinel);
    let output = run(&path).await;
    successful(&output);
    let report = outcome(&output);
    assert_eq!(report.exit_code(), 0);
    assert_eq!(
        report.tables.iter().map(|t| t.committed_rows).sum::<u64>(),
        4
    );
    let rows=target.client.query("SELECT id::text,plain_id::text,implicit_flag::text,flag,unsigned_value::text,small_decimal::text,large_decimal::text,encode(hex_value,'hex'),empty_value,dirty,to_char(wall,'YYYY-MM-DD HH24:MI:SS.US'),EXTRACT(EPOCH FROM duration)::text,state::text,memberships::text[] FROM t16_import_values.parent ORDER BY t16_import_values.parent.id",&[]).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, String>(0), "7");
    assert_eq!(rows[0].get::<_, String>(1), "7");
    assert_eq!(rows[0].get::<_, String>(2), "2");
    assert!(rows[0].get::<_, bool>(3));
    assert!(!rows[1].get::<_, bool>(3));
    assert_eq!(rows[0].get::<_, String>(4), "18446744073709551615");
    assert_eq!(rows[0].get::<_, String>(5), "1.250000");
    assert_eq!(rows[0].get::<_, String>(6), "12.500000000000");
    assert_eq!(rows[0].get::<_, String>(7), "deadbeef");
    assert_eq!(rows[1].get::<_, String>(7), "00ff");
    assert_eq!(rows[0].get::<_, Option<String>>(8), None);
    assert_eq!(rows[1].get::<_, String>(8), "kept");
    assert_eq!(rows[0].get::<_, String>(9), "ab");
    assert_eq!(rows[0].get::<_, String>(10), "2024-02-29 12:34:56.123456");
    assert_eq!(rows[0].get::<_, String>(11), "-3020398.123456");
    assert_eq!(rows[1].get::<_, String>(11), "90000.000001");
    assert_eq!(rows[0].get::<_, String>(12), "beta");
    assert_eq!(rows[0].get::<_, Vec<String>>(13), ["alpha", "beta"]);
    let types=target.client.query("SELECT attname,format_type(atttypid,atttypmod),attidentity FROM pg_attribute WHERE attrelid='t16_import_values.parent'::regclass AND attnum>0 ORDER BY attnum",&[]).await.unwrap();
    let observed = types
        .iter()
        .map(|r| {
            (
                r.get::<_, String>(0),
                (r.get::<_, String>(1), r.get::<_, i8>(2)),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(observed["id"], ("integer".into(), b'd' as i8));
    assert_eq!(observed["plain_id"].0, "numeric(20,0)");
    assert_eq!(observed["implicit_flag"].0, "smallint");
    assert_eq!(observed["flag"].0, "boolean");
    assert_eq!(observed["small_decimal"].0, "numeric(18,6)");
    assert_eq!(observed["large_decimal"].0, "numeric(30,12)");
    assert_eq!(observed["wall"].0, "timestamp(6) without time zone");
    assert_eq!(observed["duration"].0, "interval(6)");
    let child_key: String = target.client.query_one("SELECT format_type(atttypid,atttypmod) FROM pg_attribute WHERE attrelid='t16_import_values.child'::regclass AND attname='parent_id'", &[]).await.unwrap().get(0);
    assert_eq!(child_key, "integer");
    let comments=target.client.query_one("SELECT obj_description('t16_import_values.parent'::regclass),col_description('t16_import_values.parent'::regclass,(SELECT attnum FROM pg_attribute WHERE attrelid='t16_import_values.parent'::regclass AND attname='note'))",&[]).await.unwrap();
    assert_eq!(comments.get::<_, String>(0), "parent kept");
    assert_eq!(comments.get::<_, String>(1), "column kept");
    assert_eq!(target.client.query_one("SELECT COUNT(*) FROM pg_constraint WHERE conrelid IN ('t16_import_values.parent'::regclass,'t16_import_values.child'::regclass) AND contype='p'",&[]).await.unwrap().get::<_,i64>(0),2);
    assert!(
        target
            .client
            .batch_execute("INSERT INTO t16_import_values.child VALUES(99,99999)")
            .await
            .is_err()
    );
    assert!(
        target
            .client
            .batch_execute("INSERT INTO t16_import_values.parent(id,unsigned_value) VALUES(99,42)")
            .await
            .is_err()
    );
    let defaults=target.client.query_one("INSERT INTO t16_import_values.parent(unsigned_value) VALUES(123) RETURNING id,plain_id::text,implicit_flag,flag,small_decimal::text,large_decimal::text,state::text,memberships::text[]",&[]).await.unwrap();
    assert_eq!(defaults.get::<_, i32>(0), 500);
    assert_eq!(defaults.get::<_, String>(1), "7");
    assert_eq!(defaults.get::<_, i16>(2), 2);
    assert!(!defaults.get::<_, bool>(3));
    assert_eq!(defaults.get::<_, String>(4), "1.250000");
    assert_eq!(defaults.get::<_, String>(5), "12.500000000000");
    assert_eq!(defaults.get::<_, String>(6), "alpha");
    assert_eq!(defaults.get::<_, Vec<String>>(7), ["alpha"]);
    let source_after:String=source.query_first("SELECT CONCAT((SELECT COUNT(*) FROM T16ImportParent),':',(SELECT COUNT(*) FROM T16ImportChild),':',(SELECT COUNT(*) FROM T16ImportIgnored))").await.unwrap().unwrap();
    assert_eq!(source_after, source_before);
    assert_eq!(protected(&target).await, sentinel);
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned disposable MySQL8.4/PostgreSQL16 TLS harness"]
async fn imported_existing_modes_preserve_native_oids_and_refusals_publish_nothing() {
    let directory = Directory::new("t16-import-policy");
    let selection = "INCLUDING ONLY TABLE NAMES MATCHING 'T16ImportPolicy' ALTER TABLE NAMES MATCHING 'T16ImportPolicy' RENAME TO 'items'";
    let mut schema=import(&directory,"schema","t16_import_policy",&format!("WITH schema only, include no drop, create tables, create indexes, workers=1 {selection}"),false).await;
    let mut source = source(&schema).await;
    source
        .query_drop("DROP TABLE IF EXISTS T16ImportPolicy")
        .await
        .unwrap();
    source.query_drop("CREATE TABLE T16ImportPolicy(id INT PRIMARY KEY,value VARCHAR(20) NOT NULL DEFAULT 'kept',UNIQUE KEY uq_value(value)) ENGINE=InnoDB").await.unwrap();
    source
        .query_drop("INSERT INTO T16ImportPolicy VALUES(1,'source')")
        .await
        .unwrap();
    let target = target(&schema).await;
    let sentinel = protected(&target).await;
    target
        .client
        .batch_execute("DROP SCHEMA IF EXISTS t16_import_policy CASCADE")
        .await
        .unwrap();
    source_catalog_precondition(&schema).await;
    assert!(schema.overrides.is_empty());
    let unacknowledged = run(&runnable(&directory, "collation-unacknowledged", &schema)).await;
    assert_eq!(unacknowledged.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unacknowledged.stderr).contains("COLLATION_SEMANTICS"));
    let created: bool = target
        .client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='t16_import_policy')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!created);
    assert_eq!(protected(&target).await, sentinel);
    acknowledge_policy_collation(&mut schema);
    let output = run(&runnable(&directory, "schema", &schema)).await;
    successful(&output);
    assert_eq!(outcome(&output).tables[0].committed_rows, 0);
    let oid = relation_oid(&target, "t16_import_policy.items").await;
    target.client.batch_execute("CREATE TABLE t16_import_policy.audit(id integer);CREATE FUNCTION t16_import_policy.audit_insert() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN INSERT INTO t16_import_policy.audit VALUES(NEW.id);RETURN NEW;END$$;CREATE TRIGGER t16_preserved AFTER INSERT ON t16_import_policy.items FOR EACH ROW EXECUTE FUNCTION t16_import_policy.audit_insert();INSERT INTO t16_import_policy.items VALUES(99,'earlier');DELETE FROM t16_import_policy.audit").await.unwrap();
    let trigger:u32=target.client.query_one("SELECT oid FROM pg_trigger WHERE tgrelid='t16_import_policy.items'::regclass AND tgname='t16_preserved'",&[]).await.unwrap().get(0);
    let mut full = import(
        &directory,
        "existing-error",
        "t16_import_policy",
        &format!("WITH include no drop, no truncate, workers=1 {selection}"),
        false,
    )
    .await;
    acknowledge_policy_collation(&mut full);
    let refused = run(&runnable(&directory, "existing-error", &full)).await;
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("TARGET_EXISTS"));
    assert_eq!(relation_oid(&target, "t16_import_policy.items").await, oid);
    let mut append=import(&directory,"append","t16_import_policy",&format!("WITH data only, include no drop, no truncate, reset no sequences, workers=1 {selection}"),true).await;
    acknowledge_policy_collation(&mut append);
    let output = run(&runnable(&directory, "append", &append)).await;
    successful(&output);
    assert_eq!(outcome(&output).tables[0].committed_rows, 1);
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT array_agg(id ORDER BY id) FROM t16_import_policy.items",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Vec<i32>>(0),
        [1, 99]
    );
    assert_eq!(relation_oid(&target, "t16_import_policy.items").await, oid);
    assert_eq!(target.client.query_one("SELECT oid FROM pg_trigger WHERE tgrelid='t16_import_policy.items'::regclass AND tgname='t16_preserved'",&[]).await.unwrap().get::<_,u32>(0),trigger);
    let mut truncate = import(
        &directory,
        "truncate",
        "t16_import_policy",
        &format!("WITH data only, truncate, reset no sequences, workers=1 {selection}"),
        false,
    )
    .await;
    acknowledge_policy_collation(&mut truncate);
    let output = run(&runnable(&directory, "truncate", &truncate)).await;
    successful(&output);
    assert_eq!(relation_oid(&target, "t16_import_policy.items").await, oid);
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT array_agg(id ORDER BY id) FROM t16_import_policy.items",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Vec<i32>>(0),
        [1]
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT array_agg(id ORDER BY id) FROM t16_import_policy.audit",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Vec<i32>>(0),
        [1, 1]
    );
    assert_eq!(target.client.query_one("SELECT oid FROM pg_trigger WHERE tgrelid='t16_import_policy.items'::regclass AND tgname='t16_preserved'",&[]).await.unwrap().get::<_,u32>(0),trigger);
    let mut recreate = import(
        &directory,
        "recreate",
        "t16_import_policy",
        &format!("WITH include drop, create tables, create indexes, workers=1 {selection}"),
        false,
    )
    .await;
    acknowledge_policy_collation(&mut recreate);
    let output = run(&runnable(&directory, "recreate", &recreate)).await;
    successful(&output);
    assert_ne!(relation_oid(&target, "t16_import_policy.items").await, oid);
    assert_eq!(protected(&target).await, sentinel);
    // Rejected legacy syntax cannot execute SQL or publish a partial configuration.
    for (index, clause) in [
        "BEFORE LOAD DO $$ SELECT pg_sleep(60) $$",
        "CAST type tinyint when (= precision 1) to boolean",
        "WITH multiple readers per thread",
    ]
    .iter()
    .enumerate()
    {
        let input = directory.0.join(format!("rejected-{index}.load"));
        let output = directory.0.join(format!("rejected-{index}.toml"));
        fs::write(&input, format!("{}{clause};", head())).unwrap();
        let rejected = cli(
            &[
                "import-load",
                input.to_str().unwrap(),
                "--output",
                output.to_str().unwrap(),
                "--target-schema",
                "t16_import_policy",
                "--use-my2pg-defaults",
            ],
            &[],
        )
        .await;
        assert_eq!(rejected.status.code(), Some(2));
        assert!(!output.exists());
        assert_eq!(protected(&target).await, sentinel);
    }
    let input = directory.0.join("safe.load");
    fs::write(&input, format!("{}{selection};", head())).unwrap();
    let protected_file = directory.0.join("protected.toml");
    fs::write(&protected_file, b"prior artifact remains\n").unwrap();
    let refused = cli(
        &[
            "config",
            "import",
            input.to_str().unwrap(),
            "--output",
            protected_file.to_str().unwrap(),
            "--target-schema",
            "t16_import_policy",
            "--use-my2pg-defaults",
        ],
        &[],
    )
    .await;
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(
        fs::read(&protected_file).unwrap(),
        b"prior artifact remains\n"
    );
    assert!(directory.0.read_dir().unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp")
    }));
    assert_eq!(protected(&target).await, sentinel);
    assert_eq!(
        source
            .query_first::<u64, _>("SELECT COUNT(*) FROM T16ImportPolicy")
            .await
            .unwrap(),
        Some(1)
    );
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
