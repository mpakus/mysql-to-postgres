use my2pg::{config::MigrationConfig, model::RunReport, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, process::Command};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}

#[tokio::test]
#[ignore = "requires owned MySQL8.4 and PostgreSQL16 TLS fixtures"]
async fn native_cli_full_and_schema_only_preserve_renamed_composite_structures() {
    let directory = env::temp_dir().join(format!("my2pg-t11-structures-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let mut config: MigrationConfig = serde_json::from_value(serde_json::json!({
        "version":1,
        "source":{"url_env":"MY2PG_MYSQL_URL","consistency":"frozen","ca_file":required("MY2PG_TLS_CA")},
        "target":{"url_env":"MY2PG_POSTGRES_URL","schema":"t11_full_structures","ca_file":required("MY2PG_TLS_CA")},
        "migration":{"identifiers":"snake_case","batch_rows":2,"batch_bytes":4096,"max_row_bytes":1024,"memory_bytes":262144},
        "tables":{"include":["T11Parents","T11Children"],"rename":[
            {"source":"T11Parents","target":"parent_renamed"},
            {"source":"T11Children","target":"child_renamed"}
        ]},
        "report":{"directory":directory.join("runs"),"console":"json","progress":"never"}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1024)
        .await
        .unwrap();
    for sql in [
        "DROP TABLE IF EXISTS T11Children",
        "DROP TABLE IF EXISTS T11Parents",
        "CREATE TABLE T11Parents (IdentityKey INT NOT NULL AUTO_INCREMENT, SecondKey INT NOT NULL COMMENT 'column ''quote'' 界', UniqueA INT NOT NULL, UniqueB INT NOT NULL, Ordinal INT, PRIMARY KEY(IdentityKey,SecondKey), UNIQUE KEY uq_pair(UniqueA,UniqueB), KEY same_name(UniqueA DESC,Ordinal)) AUTO_INCREMENT=100 COMMENT='parent ''quote'' 界'",
        "CREATE TABLE T11Children (ChildKey INT PRIMARY KEY, ParentFirst INT, ParentSecond INT, KEY same_name(ParentFirst,ParentSecond), CONSTRAINT T11_child_parent_fk FOREIGN KEY(ParentFirst,ParentSecond) REFERENCES T11Parents(IdentityKey,SecondKey) ON UPDATE CASCADE ON DELETE SET NULL) COMMENT='child comment'",
        "INSERT INTO T11Parents VALUES (1,11,7,8,1),(2,22,17,18,2)",
        "INSERT INTO T11Children VALUES (1,1,11),(2,2,22)",
    ] {
        source.query_drop(sql).await.unwrap();
    }

    for (mode, schema, copied) in [
        ("full", "t11_full_structures", 4u64),
        ("schema_only", "t11_schema_structures", 0),
    ] {
        config.target.schema = schema.into();
        config.migration.mode = if mode == "full" {
            my2pg::config::MigrationMode::Full
        } else {
            my2pg::config::MigrationMode::SchemaOnly
        };
        config.report.directory = directory.join(mode);
        let target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
            .await
            .unwrap();
        target
            .client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await
            .unwrap();
        let path = directory.join(format!("{mode}.toml"));
        fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
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
            "{mode}: stderr={} stdout={}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let report: RunReport = serde_json::from_slice(
            &fs::read(
                fs::read_dir(&config.report.directory)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path()
                    .join("report.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(report.exit_code(), 0);
        assert_eq!(
            report
                .tables
                .iter()
                .map(|table| table.committed_rows)
                .sum::<u64>(),
            copied
        );
        assert_eq!(report.verification.tables_checked, 2);

        let indexes = target.client.query("SELECT t.relname, i.relname, x.indisprimary, x.indisunique, ARRAY(SELECT a.attname::text FROM unnest(x.indkey) WITH ORDINALITY k(num,ord) JOIN pg_attribute a ON a.attrelid=t.oid AND a.attnum=k.num ORDER BY k.ord), ARRAY(SELECT (o::int & 1)=1 FROM unnest(x.indoption) o) FROM pg_index x JOIN pg_class t ON t.oid=x.indrelid JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_class i ON i.oid=x.indexrelid WHERE n.nspname=$1 ORDER BY t.relname,i.relname", &[&schema]).await.unwrap();
        assert_eq!(indexes.len(), 5, "all selected indexes must exist");
        let mut ordinary_names = Vec::new();
        for row in &indexes {
            let table: String = row.get(0);
            let name: String = row.get(1);
            let primary: bool = row.get(2);
            let unique: bool = row.get(3);
            let columns: Vec<String> = row.get(4);
            let descending: Vec<bool> = row.get(5);
            match (table.as_str(), primary, unique) {
                ("parent_renamed", true, true) => {
                    assert_eq!(columns, ["identity_key", "second_key"])
                }
                ("parent_renamed", false, true) => assert_eq!(columns, ["unique_a", "unique_b"]),
                ("child_renamed", true, true) => assert_eq!(columns, ["child_key"]),
                ("parent_renamed", false, false) => {
                    assert_eq!(columns, ["unique_a", "ordinal"]);
                    assert_eq!(descending, [true, false]);
                    ordinary_names.push(name);
                }
                ("child_renamed", false, false) => {
                    assert_eq!(columns, ["parent_first", "parent_second"]);
                    assert_eq!(descending, [false, false]);
                    ordinary_names.push(name);
                }
                _ => panic!("unexpected index {table}.{name}"),
            }
        }
        assert_eq!(ordinary_names.len(), 2);
        assert_ne!(
            ordinary_names[0], ordinary_names[1],
            "same source index names need distinct target identities"
        );
        let fk=target.client.query_one("SELECT ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(num,ord) JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.num ORDER BY k.ord), rn.nspname::text, r.relname::text, ARRAY(SELECT a.attname::text FROM unnest(c.confkey) WITH ORDINALITY k(num,ord) JOIN pg_attribute a ON a.attrelid=c.confrelid AND a.attnum=k.num ORDER BY k.ord), c.confupdtype::text,c.confdeltype::text FROM pg_constraint c JOIN pg_class t ON t.oid=c.conrelid JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_class r ON r.oid=c.confrelid JOIN pg_namespace rn ON rn.oid=r.relnamespace WHERE n.nspname=$1 AND t.relname='child_renamed' AND c.contype='f'", &[&schema]).await.unwrap();
        assert_eq!(
            fk.get::<_, Vec<String>>(0),
            ["parent_first", "parent_second"]
        );
        assert_eq!(fk.get::<_, String>(1), schema);
        assert_eq!(fk.get::<_, String>(2), "parent_renamed");
        assert_eq!(fk.get::<_, Vec<String>>(3), ["identity_key", "second_key"]);
        assert_eq!(
            (fk.get::<_, String>(4), fk.get::<_, String>(5)),
            ("c".into(), "n".into())
        );
        let comments=target.client.query_one("SELECT obj_description(t.oid,'pg_class'), col_description(t.oid,a.attnum) FROM pg_class t JOIN pg_namespace n ON n.oid=t.relnamespace JOIN pg_attribute a ON a.attrelid=t.oid WHERE n.nspname=$1 AND t.relname='parent_renamed' AND a.attname='second_key'", &[&schema]).await.unwrap();
        assert_eq!(comments.get::<_, String>(0), "parent 'quote' 界");
        assert_eq!(comments.get::<_, String>(1), "column 'quote' 界");
        let parent = format!("{schema}.parent_renamed");
        let child = format!("{schema}.child_renamed");
        assert_eq!(
            target
                .client
                .query_one(&format!("SELECT COUNT(*) FROM {parent}"), &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            if copied == 0 { 0 } else { 2 }
        );
        assert_eq!(
            target
                .client
                .query_one(&format!("SELECT COUNT(*) FROM {child}"), &[])
                .await
                .unwrap()
                .get::<_, i64>(0),
            if copied == 0 { 0 } else { 2 }
        );
        if copied == 0 {
            target.client.batch_execute(&format!("INSERT INTO {parent} VALUES(1,11,7,8,1),(2,22,17,18,2);INSERT INTO {child} VALUES(1,1,11),(2,2,22)")).await.unwrap();
        }
        let rows = target.client.query(&format!("SELECT ARRAY[identity_key,second_key,unique_a,unique_b,ordinal] FROM {parent} ORDER BY identity_key"), &[]).await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.get::<_, Vec<i32>>(0))
                .collect::<Vec<_>>(),
            [vec![1, 11, 7, 8, 1], vec![2, 22, 17, 18, 2]]
        );
        let rows = target.client.query(&format!("SELECT ARRAY[child_key,parent_first,parent_second] FROM {child} ORDER BY child_key"), &[]).await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.get::<_, Vec<i32>>(0))
                .collect::<Vec<_>>(),
            [vec![1, 1, 11], vec![2, 2, 22]]
        );
        assert_eq!(target.client.query_one(&format!("SELECT unique_a,unique_b FROM {parent} WHERE identity_key=1 AND second_key=11"),&[]).await.unwrap().get::<_,i32>(0),7);
        let identity=target.client.query_one(&format!("INSERT INTO {parent}(second_key,unique_a,unique_b) VALUES(33,27,28) RETURNING identity_key"),&[]).await.unwrap().get::<_,i32>(0);
        assert_eq!(
            identity, 100,
            "source next value remains a lower bound even with deleted/unallocated high values"
        );
        for (sql, state) in [
            (
                format!("INSERT INTO {parent} VALUES(1,11,99,98,3)"),
                "23505",
            ),
            (format!("INSERT INTO {parent} VALUES(4,44,7,8,3)"), "23505"),
            (format!("INSERT INTO {child} VALUES(4,999,999)"), "23503"),
        ] {
            assert_eq!(
                target
                    .client
                    .batch_execute(&sql)
                    .await
                    .unwrap_err()
                    .code()
                    .unwrap()
                    .code(),
                state
            );
        }
        target
            .client
            .batch_execute(&format!(
                "UPDATE {parent} SET second_key=12 WHERE identity_key=1 AND second_key=11"
            ))
            .await
            .unwrap();
        let values = target
            .client
            .query_one(
                &format!("SELECT parent_first,parent_second FROM {child} WHERE child_key=1"),
                &[],
            )
            .await
            .unwrap();
        assert_eq!((values.get::<_, i32>(0), values.get::<_, i32>(1)), (1, 12));
        target
            .client
            .batch_execute(&format!(
                "DELETE FROM {parent} WHERE identity_key=1 AND second_key=12"
            ))
            .await
            .unwrap();
        let values = target
            .client
            .query_one(
                &format!("SELECT parent_first,parent_second FROM {child} WHERE child_key=1"),
                &[],
            )
            .await
            .unwrap();
        assert_eq!(
            (
                values.get::<_, Option<i32>>(0),
                values.get::<_, Option<i32>>(1)
            ),
            (None, None)
        );
        assert_eq!(
            target
                .client
                .query_one("SELECT sentinel FROM public.users WHERE id=999", &[])
                .await
                .unwrap()
                .get::<_, String>(0),
            "must-survive"
        );
        target.close().await.unwrap();
    }
    source.query_drop("DROP TABLE T11Children").await.unwrap();
    source.query_drop("DROP TABLE T11Parents").await.unwrap();
    source.disconnect().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}
