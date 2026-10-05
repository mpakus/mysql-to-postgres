use my2pg::{
    config::{ExistingPolicy, MigrationConfig, MigrationMode},
    mysql, plan, postgres,
};
use mysql_async::prelude::Queryable;
use std::{env, fs, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}
fn codes(result: Result<my2pg::model::MigrationPlan, plan::PlanError>) -> Vec<String> {
    let plan::PlanError::Blocked(diagnostics) = result.unwrap_err() else {
        panic!("expected positive planning rejection")
    };
    diagnostics.into_iter().map(|d| d.code).collect()
}
async fn run(config: &MigrationConfig, path: &std::path::Path) -> std::process::Output {
    fs::write(path, toml::to_string(config).unwrap()).unwrap();
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_my2pg"))
            .arg("run")
            .arg(path)
            .args(["--output", "json", "--progress", "never"])
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4/PostgreSQL16 TLS fixtures"]
async fn native_existing_append_truncate_recreate_bind_actual_structures_and_preserve_sentinels() {
    let directory = env::temp_dir().join(format!("my2pg-t11-existing-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,"source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":"t11_existing","on_existing":"append"},
        "migration":{"mode":"data_only","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":["T11ExistingParent","T11ExistingChild"],"rename":[{"source":"T11ExistingParent","target":"parent"},{"source":"T11ExistingChild","target":"child"}]},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    for sql in [
        "DROP TABLE IF EXISTS T11ExistingChild",
        "DROP TABLE IF EXISTS T11ExistingParent",
        "CREATE TABLE T11ExistingParent(id INT NOT NULL PRIMARY KEY, value INT NOT NULL DEFAULT 7, UNIQUE KEY source_unique(value))",
        "CREATE TABLE T11ExistingChild(id INT NOT NULL PRIMARY KEY,parent_id INT, KEY source_ordinary(parent_id), CONSTRAINT source_fk FOREIGN KEY(parent_id) REFERENCES T11ExistingParent(id) ON DELETE CASCADE ON UPDATE RESTRICT)",
        "INSERT INTO T11ExistingParent VALUES(1,11),(2,22)",
        "INSERT INTO T11ExistingChild VALUES(1,1)",
    ] {
        source.query_drop(sql).await.unwrap();
    }
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute("CREATE SCHEMA t11_existing;CREATE SCHEMA t11_external;CREATE TABLE t11_external.sentinel(value text);INSERT INTO t11_external.sentinel VALUES('keep');CREATE TABLE t11_existing.audit(id integer);CREATE TABLE t11_existing.parent(id integer NOT NULL,value integer NOT NULL DEFAULT 7, CONSTRAINT real_parent_pk PRIMARY KEY(id),CONSTRAINT extra_positive CHECK(id>0));CREATE UNIQUE INDEX actual_unique ON t11_existing.parent(value);CREATE TABLE t11_existing.child(id integer NOT NULL,parent_id integer, CONSTRAINT real_child_pk PRIMARY KEY(id),CONSTRAINT real_fk FOREIGN KEY(parent_id) REFERENCES t11_existing.parent(id) ON DELETE CASCADE ON UPDATE RESTRICT);CREATE INDEX actual_ordinary ON t11_existing.child(parent_id);CREATE FUNCTION t11_existing.audit_insert() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN INSERT INTO t11_existing.audit VALUES(NEW.id);RETURN NEW;END$$;CREATE TRIGGER preserved_trigger AFTER INSERT ON t11_existing.parent FOR EACH ROW EXECUTE FUNCTION t11_existing.audit_insert();INSERT INTO t11_existing.parent VALUES(99,99)").await.unwrap();
    // Parent1 is already available for the child regardless of worker scheduling.
    target
        .client
        .batch_execute(
            "INSERT INTO t11_existing.parent VALUES(1,11);DELETE FROM t11_existing.audit",
        )
        .await
        .unwrap();
    source.query_drop("DELETE FROM T11ExistingChild;DELETE FROM T11ExistingParent WHERE id=1;INSERT INTO T11ExistingChild VALUES(2,NULL)").await.unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let before = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    let prepared = plan::build(&config, &catalog, &before).unwrap();
    assert!(prepared.ddl.is_empty());
    assert_eq!(
        prepared.tables[0]
            .structure
            .indexes
            .iter()
            .find(|i| i.primary)
            .unwrap()
            .name,
        "real_child_pk"
    );
    assert_eq!(
        prepared
            .tables
            .iter()
            .find(|t| t.target_name == "parent")
            .unwrap()
            .structure
            .indexes
            .iter()
            .find(|i| !i.primary)
            .unwrap()
            .name,
        "actual_unique"
    );
    assert_eq!(
        prepared
            .tables
            .iter()
            .find(|t| t.target_name == "child")
            .unwrap()
            .structure
            .foreign_keys[0]
            .name,
        "real_fk"
    );
    let trigger_oid: u32 = target
        .client
        .query_one(
            "SELECT oid FROM pg_trigger WHERE tgname='preserved_trigger'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let output = run(&config, &directory.join("append.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = target
        .client
        .query("SELECT id FROM t11_existing.parent ORDER BY id", &[])
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.get::<_, i32>(0)).collect::<Vec<_>>(),
        [1, 2, 99]
    );
    assert_eq!(
        target
            .client
            .query_one("SELECT count(*) FROM t11_existing.audit WHERE id=2", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT oid FROM pg_trigger WHERE tgname='preserved_trigger'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, u32>(0),
        trigger_oid
    );
    for sql in [
        "INSERT INTO t11_existing.parent VALUES(2,222)",
        "INSERT INTO t11_existing.parent VALUES(3,22)",
        "INSERT INTO t11_existing.parent VALUES(-1,-1)",
        "INSERT INTO t11_existing.child VALUES(3,12345)",
    ] {
        assert!(
            target.client.batch_execute(sql).await.is_err(),
            "preserved constraint: {sql}"
        );
    }

    config.target.on_existing = ExistingPolicy::Truncate;
    config.tables.include = vec!["T11ExistingParent".into()];
    config
        .tables
        .rename
        .retain(|r| r.source == "T11ExistingParent");
    assert!(
        codes(plan::build(&config, &catalog, &before))
            .contains(&"TARGET_DEPENDENCY_OUTSIDE_SELECTION".into())
    );
    config.tables.include.push("T11ExistingChild".into());
    config.tables.rename.push(my2pg::config::TableRename {
        source: "T11ExistingChild".into(),
        target: "child".into(),
        schema: None,
    });
    source
        .query_drop("DELETE FROM T11ExistingChild")
        .await
        .unwrap();
    config.report.directory = directory.join("truncate-runs");
    let output = run(&config, &directory.join("truncate.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT array_agg(id ORDER BY id) FROM t11_existing.parent",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Vec<i32>>(0),
        [2]
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT oid FROM pg_trigger WHERE tgname='preserved_trigger'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, u32>(0),
        trigger_oid
    );

    // Native invalid state: no system catalog mutation, and every failure preserves rows.
    target
        .client
        .batch_execute(
            "ALTER TABLE t11_existing.parent ADD CONSTRAINT invalid_extra CHECK(value>0) NOT VALID",
        )
        .await
        .unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        codes(plan::build(
            &config,
            &mysql::inspect(&mut source).await.unwrap(),
            &current
        ))
        .contains(&"TARGET_CONSTRAINT_UNSUPPORTED".into())
    );
    target.client.batch_execute("ALTER TABLE t11_existing.parent DROP CONSTRAINT invalid_extra;DROP TRIGGER preserved_trigger ON t11_existing.parent;ALTER TABLE t11_existing.parent DROP CONSTRAINT extra_positive").await.unwrap();
    config.migration.mode = MigrationMode::Full;
    config.target.on_existing = ExistingPolicy::Recreate;
    config.report.directory = directory.join("recreate-runs");
    target
        .client
        .batch_execute(
            "CREATE VIEW t11_external.external_view AS SELECT * FROM t11_existing.parent",
        )
        .await
        .unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        codes(plan::build(
            &config,
            &mysql::inspect(&mut source).await.unwrap(),
            &current
        ))
        .contains(&"TARGET_DEPENDENCY_OUTSIDE_SELECTION".into())
    );
    target
        .client
        .batch_execute("DROP VIEW t11_external.external_view")
        .await
        .unwrap();
    let output = run(&config, &directory.join("recreate.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        target
            .client
            .query_one("SELECT value FROM t11_external.sentinel", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "keep"
    );
    target
        .client
        .batch_execute("DROP SCHEMA t11_existing CASCADE;DROP SCHEMA t11_external CASCADE")
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE T11ExistingChild")
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE T11ExistingParent")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4/PostgreSQL16 TLS fixtures"]
async fn native_existing_enum_reuses_exact_identity_order_and_preserves_outside_users() {
    let directory = env::temp_dir().join(format!("my2pg-t11-enum-reuse-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,"source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":"t11_reuse","on_existing":"append"},
        "migration":{"mode":"data_only","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":["T11EnumReuse"],"rename":[{"source":"T11EnumReuse","target":"enum_table"}]},
        "report":{"directory":directory.join("append-runs"),"console":"json","progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE IF EXISTS T11EnumReuse")
        .await
        .unwrap();
    source.query_drop("CREATE TABLE T11EnumReuse(id INT PRIMARY KEY,status ENUM('','a,b','quote''','a\\\\b') DEFAULT 'a,b',other ENUM('','a,b','quote''','a\\\\b') DEFAULT 'a,b') CHARACTER SET utf8mb3").await.unwrap();
    source
        .query_drop("INSERT INTO T11EnumReuse(id) VALUES(1)")
        .await
        .unwrap();
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute(r#"DROP SCHEMA IF EXISTS t11_reuse_other CASCADE;DROP SCHEMA IF EXISTS t11_reuse CASCADE;CREATE SCHEMA t11_reuse;CREATE SCHEMA t11_reuse_other;
        CREATE TYPE t11_reuse."Status" AS ENUM ('','a,b','quote''','a\b');
        CREATE TYPE t11_reuse_other."Status" AS ENUM ('other');
        CREATE TABLE t11_reuse.enum_table(id integer PRIMARY KEY,status t11_reuse."Status" DEFAULT 'a,b',other t11_reuse."Status" DEFAULT 'a,b');
        CREATE TABLE t11_reuse_other.outside_user(status t11_reuse."Status");INSERT INTO t11_reuse_other.outside_user VALUES('quote''');
        INSERT INTO t11_reuse.enum_table VALUES(99,'','a\b')"#).await.unwrap();
    let type_oid: u32 = target
        .client
        .query_one("SELECT 't11_reuse.\"Status\"'::regtype::oid", &[])
        .await
        .unwrap()
        .get(0);
    let source_catalog = mysql::inspect(&mut source).await.unwrap();
    let observed = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    let planned = plan::build(&config, &source_catalog, &observed).unwrap();
    assert_eq!(
        planned.tables[0].columns[1].target_type,
        "\"t11_reuse\".\"Status\""
    );
    assert_eq!(
        planned.tables[0].columns[2].target_type,
        "\"t11_reuse\".\"Status\""
    );
    assert!(planned.ddl.is_empty());
    target.client.batch_execute(r#"CREATE TYPE t11_reuse."WrongStatus" AS ENUM ('a,b','','quote''','a\b');
        ALTER TABLE t11_reuse.enum_table ALTER COLUMN status DROP DEFAULT;
        ALTER TABLE t11_reuse.enum_table ALTER COLUMN status TYPE t11_reuse."WrongStatus" USING status::text::t11_reuse."WrongStatus";
        ALTER TABLE t11_reuse.enum_table ALTER COLUMN status SET DEFAULT 'a,b'"#).await.unwrap();
    let wrong_order = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        codes(plan::build(&config, &source_catalog, &wrong_order))
            .contains(&"TARGET_COLUMN_INCOMPATIBLE".into())
    );
    target.client.batch_execute(r#"ALTER TABLE t11_reuse.enum_table ALTER COLUMN status DROP DEFAULT;
        ALTER TABLE t11_reuse.enum_table ALTER COLUMN status TYPE t11_reuse."Status" USING status::text::t11_reuse."Status";
        ALTER TABLE t11_reuse.enum_table ALTER COLUMN status SET DEFAULT 'a,b';DROP TYPE t11_reuse."WrongStatus""#).await.unwrap();
    target.client.batch_execute(r#"CREATE ROLE t11_reuse_reader;GRANT USAGE ON SCHEMA t11_reuse TO t11_reuse_reader;GRANT SELECT,INSERT ON t11_reuse.enum_table TO t11_reuse_reader;REVOKE USAGE ON TYPE t11_reuse."Status" FROM PUBLIC;SET ROLE t11_reuse_reader"#).await.unwrap();
    let denied_native = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        codes(plan::build(&config, &source_catalog, &denied_native))
            .contains(&"TARGET_COLUMN_INCOMPATIBLE".into())
    );
    target.client.batch_execute(r#"RESET ROLE;GRANT USAGE ON TYPE t11_reuse."Status" TO PUBLIC;DROP OWNED BY t11_reuse_reader;DROP ROLE t11_reuse_reader"#).await.unwrap();
    let mut creates = config.clone();
    creates.migration.mode = MigrationMode::SchemaOnly;
    creates.target.on_existing = ExistingPolicy::Error;
    creates.tables.rename[0].target = "new_enum_table".into();
    let fresh = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    let generated = plan::build(&creates, &source_catalog, &fresh)
        .unwrap()
        .tables[0]
        .columns[1]
        .target_type
        .clone();
    for definition in ["AS text", "AS (value integer)", "AS ENUM ()"] {
        target
            .client
            .batch_execute(&format!(
                "CREATE {} {generated} {definition}",
                if definition == "AS text" {
                    "DOMAIN"
                } else {
                    "TYPE"
                }
            ))
            .await
            .unwrap();
        let collision = postgres::inspect(&mut target.client, &config.target.schema)
            .await
            .unwrap();
        assert!(
            codes(plan::build(&creates, &source_catalog, &collision))
                .contains(&"ENUM_NAME_OCCUPIED".into()),
            "{definition}"
        );
        target
            .client
            .batch_execute(&format!(
                "DROP {} {generated}",
                if definition == "AS text" {
                    "DOMAIN"
                } else {
                    "TYPE"
                }
            ))
            .await
            .unwrap();
    }
    let mut mismatched = observed.clone();
    for p in mismatched
        .type_parts
        .as_mut()
        .unwrap()
        .iter_mut()
        .filter(|p| p.type_oid == Some(type_oid))
    {
        p.enum_sort_order = p.enum_sort_order.map(|order| -order);
    }
    assert!(
        codes(plan::build(&config, &source_catalog, &mismatched))
            .contains(&"TARGET_COLUMN_INCOMPATIBLE".into())
    );
    let mut missing_graph = observed.clone();
    missing_graph.dependency_graph = None;
    assert!(
        codes(plan::build(&config, &source_catalog, &missing_graph))
            .contains(&"TARGET_COLUMN_INCOMPATIBLE".into())
    );
    let mut denied = observed.clone();
    for p in denied
        .type_parts
        .as_mut()
        .unwrap()
        .iter_mut()
        .filter(|p| p.type_oid == Some(type_oid))
    {
        p.can_use = Some(false);
    }
    assert!(
        codes(plan::build(&config, &source_catalog, &denied))
            .contains(&"TARGET_COLUMN_INCOMPATIBLE".into())
    );
    let output = run(&config, &directory.join("append.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT status::text,other::text FROM t11_reuse.enum_table WHERE id=1",
                &[]
            )
            .await
            .unwrap()
            .get::<_, String>(0),
        "a,b"
    );
    config.target.on_existing = ExistingPolicy::Recreate;
    config.migration.mode = MigrationMode::Full;
    config.report.directory = directory.join("recreate-runs");
    let output = run(&config, &directory.join("recreate.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        target
            .client
            .query_one("SELECT 't11_reuse.\"Status\"'::regtype::oid", &[])
            .await
            .unwrap()
            .get::<_, u32>(0),
        type_oid
    );
    // A second recreate must reuse generated indexes owned by selected old tables.
    let output = run(&config, &directory.join("recreate-again.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        target
            .client
            .query_one("SELECT status::text FROM t11_reuse_other.outside_user", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "quote'"
    );
    assert_eq!(
        target
            .client
            .query_one("SELECT count(*) FROM t11_reuse.enum_table", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    target
        .client
        .batch_execute("INSERT INTO t11_reuse.enum_table(id) VALUES(2)")
        .await
        .unwrap();
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT status::text,other::text FROM t11_reuse.enum_table WHERE id=2",
                &[]
            )
            .await
            .unwrap()
            .get::<_, String>(1),
        "a,b"
    );
    target
        .client
        .batch_execute("DROP SCHEMA t11_reuse_other CASCADE;DROP SCHEMA t11_reuse CASCADE")
        .await
        .unwrap();
    source.query_drop("DROP TABLE T11EnumReuse").await.unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4/PostgreSQL16 TLS fixtures"]
async fn native_composite_order_extra_states_and_unknown_graph_fail_before_mutation() {
    let directory = env::temp_dir().join(format!("my2pg-t11-composite-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,"source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":"t11_composite","on_existing":"append"},
        "migration":{"mode":"data_only","identifiers":"snake_case","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":["T11Composite"],"rename":[{"source":"T11Composite","target":"mapped"}]},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE IF EXISTS T11Composite")
        .await
        .unwrap();
    source.query_drop("CREATE TABLE T11Composite(FirstKey INT NOT NULL,SecondKey INT NOT NULL,UniqueFirst INT NOT NULL,UniqueSecond INT NOT NULL,Label VARCHAR(20) DEFAULT '',PRIMARY KEY(FirstKey,SecondKey),UNIQUE KEY source_uq(UniqueSecond,UniqueFirst),KEY source_ix(UniqueFirst DESC,UniqueSecond)) CHARACTER SET utf8mb3 COMMENT='composite source'").await.unwrap();
    source
        .query_drop("INSERT INTO T11Composite VALUES(1,11,7,8,'dup')")
        .await
        .unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute("CREATE SCHEMA t11_composite;CREATE TABLE t11_composite.mapped(first_key integer NOT NULL,second_key integer NOT NULL,unique_first integer NOT NULL,unique_second integer NOT NULL,label varchar(20) DEFAULT '',CONSTRAINT actual_composite_pk PRIMARY KEY(first_key,second_key),CONSTRAINT actual_composite_uq UNIQUE(unique_second,unique_first));CREATE INDEX actual_composite_ix ON t11_composite.mapped(unique_first DESC,unique_second);COMMENT ON TABLE t11_composite.mapped IS 'composite source';INSERT INTO t11_composite.mapped VALUES(99,199,97,98,'dup')").await.unwrap();
    let observed = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    let prepared = plan::build(&config, &catalog, &observed).unwrap();
    assert_eq!(prepared.tables[0].primary_key, ["FirstKey", "SecondKey"]);
    assert_eq!(
        prepared.tables[0]
            .structure
            .indexes
            .iter()
            .find(|i| i.primary)
            .unwrap()
            .columns,
        ["first_key", "second_key"]
    );
    assert_eq!(
        prepared.tables[0]
            .structure
            .indexes
            .iter()
            .find(|i| i.unique && !i.primary)
            .unwrap()
            .name,
        "actual_composite_uq"
    );
    let output = run(&config, &directory.join("append.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let unchanged = target
        .client
        .query_one(
            "SELECT array_agg(first_key ORDER BY first_key) FROM t11_composite.mapped",
            &[],
        )
        .await
        .unwrap()
        .get::<_, Vec<i32>>(0);
    assert_eq!(unchanged, [1, 99]);
    target.client.batch_execute("ALTER TABLE t11_composite.mapped DROP CONSTRAINT actual_composite_pk;ALTER TABLE t11_composite.mapped ADD CONSTRAINT reversed_pk PRIMARY KEY(second_key,first_key)").await.unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        codes(plan::build(&config, &catalog, &current))
            .contains(&"TARGET_INDEX_INCOMPATIBLE".into())
    );
    target.client.batch_execute("ALTER TABLE t11_composite.mapped DROP CONSTRAINT reversed_pk;ALTER TABLE t11_composite.mapped ADD CONSTRAINT actual_composite_pk PRIMARY KEY(first_key,second_key)").await.unwrap();
    for create in [
        "CREATE INDEX unsupported_extra ON t11_composite.mapped(unique_first) INCLUDE(label)",
        "CREATE INDEX unsupported_extra ON t11_composite.mapped((lower(label)))",
        "CREATE INDEX unsupported_extra ON t11_composite.mapped(unique_first) WHERE unique_first>0",
        "CREATE INDEX unsupported_extra ON t11_composite.mapped(label varchar_pattern_ops)",
        "CREATE UNIQUE INDEX unsupported_extra ON t11_composite.mapped(unique_first) NULLS NOT DISTINCT",
    ] {
        target.client.batch_execute(create).await.unwrap();
        let current = postgres::inspect(&mut target.client, &config.target.schema)
            .await
            .unwrap();
        assert!(
            codes(plan::build(&config, &catalog, &current))
                .contains(&"TARGET_INDEX_UNSUPPORTED".into()),
            "{create}"
        );
        target
            .client
            .batch_execute("DROP INDEX t11_composite.unsupported_extra")
            .await
            .unwrap();
    }
    assert!(
        target
            .client
            .batch_execute(
                "CREATE UNIQUE INDEX CONCURRENTLY failed_extra ON t11_composite.mapped(label)"
            )
            .await
            .is_err()
    );
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        current
            .tables
            .iter()
            .find(|t| t.name == "mapped")
            .unwrap()
            .observed
            .as_ref()
            .unwrap()
            .index_parts
            .iter()
            .any(|p| p.index_name == "failed_extra" && !p.valid)
    );
    assert!(
        codes(plan::build(&config, &catalog, &current))
            .contains(&"TARGET_INDEX_UNSUPPORTED".into())
    );
    target.client.batch_execute("DROP INDEX t11_composite.failed_extra;ALTER TABLE t11_composite.mapped ADD CONSTRAINT deferred_extra UNIQUE(first_key) DEFERRABLE INITIALLY DEFERRED").await.unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    let preserved = plan::build(&config, &catalog, &current).unwrap();
    assert!(
        preserved.ddl.is_empty(),
        "existing deferred extra key must remain owned by PostgreSQL"
    );
    target.client.batch_execute("ALTER TABLE t11_composite.mapped DROP CONSTRAINT deferred_extra;ALTER TABLE t11_composite.mapped ADD CONSTRAINT pending_extra CHECK(unique_first>0) NOT VALID").await.unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        codes(plan::build(&config, &catalog, &current))
            .contains(&"TARGET_CONSTRAINT_UNSUPPORTED".into())
    );
    target
        .client
        .batch_execute("ALTER TABLE t11_composite.mapped VALIDATE CONSTRAINT pending_extra")
        .await
        .unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    plan::build(&config, &catalog, &current).unwrap();
    config.target.on_existing = ExistingPolicy::Recreate;
    config.migration.mode = MigrationMode::Full;
    assert!(
        plan::build(&config, &catalog, &current)
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code == "TARGET_RECREATE_REMOVE_CONSTRAINT"
                && d.object.as_deref() == Some("pending_extra"))
    );
    config.target.on_existing = ExistingPolicy::Append;
    config.migration.mode = MigrationMode::DataOnly;
    target.client.batch_execute("ALTER TABLE t11_composite.mapped DROP CONSTRAINT pending_extra;CREATE STATISTICS t11_composite.unknown_stats ON first_key,second_key FROM t11_composite.mapped").await.unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    assert!(
        codes(plan::build(&config, &catalog, &current))
            .contains(&"TARGET_OWNED_OBJECT_UNSUPPORTED".into())
    );
    target
        .client
        .batch_execute("DROP STATISTICS t11_composite.unknown_stats")
        .await
        .unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    let mut unknown = current.clone();
    let graph = unknown.dependency_graph.as_mut().unwrap();
    let edge = graph
        .parts
        .iter_mut()
        .find_map(|p| {
            p.edge.as_mut().filter(|e| {
                e.dependency.referenced_catalog.as_deref() == Some("pg_catalog.pg_class")
                    && e.dependency.referenced_oid
                        == Some(current.tables[0].observed.as_ref().unwrap().oid)
            })
        })
        .unwrap();
    edge.dependency.dependency_kind = Some("?".into());
    assert!(
        codes(plan::build(&config, &catalog, &unknown))
            .contains(&"TARGET_DEPENDENCY_UNSUPPORTED".into())
    );
    let mut malformed = current.clone();
    malformed.dependency_graph.as_mut().unwrap().parts[0].node_class_oid = 0;
    assert!(
        codes(plan::build(&config, &catalog, &malformed))
            .contains(&"TARGET_DEPENDENCY_MALFORMED".into())
    );
    let mut header = current.clone();
    header
        .dependency_graph
        .as_mut()
        .unwrap()
        .relation_root_count += 1;
    assert!(
        codes(plan::build(&config, &catalog, &header))
            .contains(&"TARGET_DEPENDENCY_MALFORMED".into())
    );
    target.client.batch_execute("CREATE FUNCTION t11_composite.on_truncate() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NULL;END$$;CREATE TRIGGER truncate_extra AFTER TRUNCATE ON t11_composite.mapped FOR EACH STATEMENT EXECUTE FUNCTION t11_composite.on_truncate()").await.unwrap();
    let current = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    config.target.on_existing = ExistingPolicy::Truncate;
    assert!(
        codes(plan::build(&config, &catalog, &current)).contains(&"TARGET_TRUNCATE_TRIGGER".into())
    );
    config.target.on_existing = ExistingPolicy::Recreate;
    config.migration.mode = MigrationMode::Full;
    assert!(
        plan::build(&config, &catalog, &current)
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code == "TARGET_RECREATE_REMOVE_TRIGGER")
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT array_agg(first_key ORDER BY first_key) FROM t11_composite.mapped",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Vec<i32>>(0),
        unchanged
    );
    target.client.batch_execute("CREATE SCHEMA t11_composite_other;CREATE TABLE t11_composite_other.sentinel(value text);INSERT INTO t11_composite_other.sentinel VALUES('outside remains');ALTER TABLE t11_composite.mapped ADD CONSTRAINT owned_extra_check CHECK(first_key>0);CREATE INDEX owned_extra_index ON t11_composite.mapped(label)").await.unwrap();
    let sentinel_oid: u32 = target
        .client
        .query_one("SELECT 't11_composite_other.sentinel'::regclass::oid", &[])
        .await
        .unwrap()
        .get(0);
    let old_trigger_oid: u32 = target
        .client
        .query_one(
            "SELECT oid FROM pg_trigger WHERE tgname='truncate_extra'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let old_table_oid: u32 = target
        .client
        .query_one("SELECT 't11_composite.mapped'::regclass::oid", &[])
        .await
        .unwrap()
        .get(0);
    let removal = postgres::inspect(&mut target.client, &config.target.schema)
        .await
        .unwrap();
    let prepared = plan::build(&config, &catalog, &removal).unwrap();
    assert!(
        prepared
            .diagnostics
            .iter()
            .any(|d| d.code == "TARGET_RECREATE_REMOVE_INDEX"
                && d.object.as_deref() == Some("owned_extra_index"))
    );
    assert!(
        prepared
            .diagnostics
            .iter()
            .any(|d| d.code == "TARGET_RECREATE_REMOVE_CONSTRAINT"
                && d.object.as_deref() == Some("owned_extra_check"))
    );
    let mut unknown_removal = removal.clone();
    for p in &mut unknown_removal
        .tables
        .iter_mut()
        .find(|t| t.name == "mapped")
        .unwrap()
        .observed
        .as_mut()
        .unwrap()
        .constraint_parts
    {
        if p.name == "owned_extra_check" {
            p.kind = "?".into();
        }
    }
    assert!(
        codes(plan::build(&config, &catalog, &unknown_removal))
            .contains(&"TARGET_DROP_CONSTRAINT_UNSUPPORTED".into())
    );
    let output = run(&config, &directory.join("owned-removal.toml")).await;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(
        target
            .client
            .query_one("SELECT 't11_composite.mapped'::regclass::oid", &[])
            .await
            .unwrap()
            .get::<_, u32>(0),
        old_table_oid
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT count(*) FROM pg_trigger WHERE oid=$1",
                &[&old_trigger_oid]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        target
            .client
            .query_one(
                "SELECT count(*) FROM pg_constraint WHERE conname='owned_extra_check'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert!(
        target
            .client
            .query_one(
                "SELECT to_regclass('t11_composite.owned_extra_index')::oid",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Option<u32>>(0)
            .is_none()
    );
    assert_eq!(target.client.query_one("SELECT 't11_composite_other.sentinel'::regclass::oid,value FROM t11_composite_other.sentinel",&[]).await.unwrap().get::<_,u32>(0),sentinel_oid);
    assert_eq!(
        target
            .client
            .query_one("SELECT value FROM t11_composite_other.sentinel", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "outside remains"
    );
    assert!(
        target
            .client
            .query_one(
                "SELECT to_regprocedure('t11_composite.on_truncate()')::oid",
                &[]
            )
            .await
            .unwrap()
            .get::<_, Option<u32>>(0)
            .is_some(),
        "trigger function was outside table-owned drop closure"
    );
    target
        .client
        .batch_execute("DROP SCHEMA t11_composite_other CASCADE")
        .await
        .unwrap();
    target
        .client
        .batch_execute("DROP SCHEMA t11_composite CASCADE")
        .await
        .unwrap();
    source.query_drop("DROP TABLE T11Composite").await.unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4/PostgreSQL16 TLS fixtures"]
async fn native_mapped_schema_facts_scope_collisions_and_privileges() {
    let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,"source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":"t11_cross_a","on_existing":"append"},
        "migration":{"mode":"data_only","reset_sequences":false,"batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":["T11CrossA","T11CrossB"],"rename":[{"source":"T11CrossA","target":"same"},{"source":"T11CrossB","target":"secondary_only","schema":"t11_cross_b"}]}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    for sql in [
        "DROP TABLE IF EXISTS T11CrossA",
        "DROP TABLE IF EXISTS T11CrossB",
        "CREATE TABLE T11CrossA(id INT PRIMARY KEY)",
        "CREATE TABLE T11CrossB(id INT PRIMARY KEY)",
        "INSERT INTO T11CrossA VALUES(1)",
        "INSERT INTO T11CrossB VALUES(2)",
    ] {
        source.query_drop(sql).await.unwrap();
    }
    let catalog = mysql::inspect(&mut source).await.unwrap();
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute("CREATE SCHEMA t11_cross_a;CREATE SCHEMA t11_cross_b;CREATE TABLE t11_cross_a.same(id integer NOT NULL,CONSTRAINT shared_pk PRIMARY KEY(id));CREATE VIEW t11_cross_a.secondary_only AS SELECT 42 AS id;CREATE TABLE t11_cross_b.secondary_only(id integer NOT NULL,CONSTRAINT shared_pk PRIMARY KEY(id));INSERT INTO t11_cross_a.same VALUES(99);INSERT INTO t11_cross_b.secondary_only VALUES(199);CREATE VIEW t11_cross_b.occupied AS SELECT 1 AS id").await.unwrap();
    let additional = vec!["t11_cross_b".into()];
    let observed =
        postgres::inspect_schemas(&mut target.client, &config.target.schema, &additional)
            .await
            .unwrap();
    let prepared = plan::build(&config, &catalog, &observed).unwrap();
    assert_eq!(prepared.tables.len(), 2);
    assert!(prepared.ddl.is_empty());
    assert!(
        prepared
            .tables
            .iter()
            .all(|t| t.structure.indexes[0].name == "shared_pk")
    );
    assert!(
        prepared
            .tables
            .iter()
            .any(|t| t.target_schema == "t11_cross_b" && t.target_name == "secondary_only")
    );
    // A primary-schema view of the same name never claims the mapped schema's table.
    let mut missing = observed.clone();
    missing
        .namespaces
        .as_mut()
        .unwrap()
        .retain(|n| n.schema == "t11_cross_a");
    assert!(
        codes(plan::build(&config, &catalog, &missing))
            .contains(&"TARGET_SCHEMA_UNINSPECTED".into())
    );
    config.tables.rename[1].target = "occupied".into();
    assert!(
        codes(plan::build(&config, &catalog, &observed))
            .contains(&"TARGET_RELATION_COLLISION".into())
    );
    config.tables.rename[1].target = "secondary_only".into();
    target.client.batch_execute("CREATE ROLE t11_cross_reader;GRANT USAGE ON SCHEMA t11_cross_a TO t11_cross_reader;GRANT SELECT,INSERT ON t11_cross_a.same,t11_cross_b.secondary_only TO t11_cross_reader;SET ROLE t11_cross_reader").await.unwrap();
    let denied = postgres::inspect_schemas(&mut target.client, &config.target.schema, &additional)
        .await
        .unwrap();
    assert!(
        codes(plan::build(&config, &catalog, &denied))
            .contains(&"TARGET_SCHEMA_USAGE_DENIED".into())
    );
    target.client.batch_execute("RESET ROLE;GRANT USAGE ON SCHEMA t11_cross_b TO t11_cross_reader;SET ROLE t11_cross_reader").await.unwrap();
    let read_only =
        postgres::inspect_schemas(&mut target.client, &config.target.schema, &additional)
            .await
            .unwrap();
    plan::build(&config, &catalog, &read_only).unwrap();
    config.target.on_existing = ExistingPolicy::Recreate;
    config.migration.mode = MigrationMode::SchemaOnly;
    assert!(
        codes(plan::build(&config, &catalog, &read_only))
            .contains(&"TARGET_SCHEMA_PRIVILEGE".into())
    );
    target.client.batch_execute("RESET ROLE").await.unwrap();
    assert_eq!(
        target
            .client
            .query_one("SELECT id FROM t11_cross_a.same", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        99
    );
    assert_eq!(
        target
            .client
            .query_one("SELECT id FROM t11_cross_b.secondary_only", &[])
            .await
            .unwrap()
            .get::<_, i32>(0),
        199
    );
    target.client.batch_execute("DROP SCHEMA t11_cross_a CASCADE;DROP SCHEMA t11_cross_b CASCADE;DROP ROLE t11_cross_reader").await.unwrap();
    source.query_drop("DROP TABLE T11CrossA").await.unwrap();
    source.query_drop("DROP TABLE T11CrossB").await.unwrap();
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
