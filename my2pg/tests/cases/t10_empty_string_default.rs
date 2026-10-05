use my2pg::{config::MigrationConfig, mysql, plan, postgres};
use mysql_async::prelude::Queryable;
fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}
type Defaults = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);
#[tokio::test]
#[ignore = "requires owned native MySQL 5.7/8.0/8.4 and PostgreSQL 16/17/18 verified TLS fixture"]
async fn native_empty_text_defaults_remain_distinct_without_source_rows_or_writes() {
    let names = [
        format!("t10_empty_{}_empty", std::process::id()),
        format!("t10_empty_{}_populated", std::process::id()),
    ];
    let schema = format!("t10_empty_defaults_{}", std::process::id());
    let mut config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,"source":{"url_env":"MY2PG_MYSQL_URL","ca_file":required("MY2PG_TLS_CA"),"consistency":"frozen"},
        "target":{"url_env":"MY2PG_POSTGRES_URL","ca_file":required("MY2PG_TLS_CA"),"schema":schema},
        "tables":{"include":names}
    })).unwrap();
    let mut source = mysql::connect(&config.source, &required("MY2PG_MYSQL_ROOT_URL"), 1048576)
        .await
        .unwrap();
    let session: (String, String, String, String, String, String) = source
        .query_first("SELECT VERSION(),@@version_comment,@@character_set_client,@@character_set_connection,@@character_set_results,@@sql_mode")
        .await
        .unwrap()
        .unwrap();
    eprintln!("native default inspection session: {session:?}");
    for name in &names {
        source.query_drop(format!("CREATE TABLE {}(id INT PRIMARY KEY,empty_value VARCHAR(8) NOT NULL DEFAULT '',nullable_empty VARCHAR(8) DEFAULT '',char_empty CHAR(3) DEFAULT '',enum_empty ENUM('','x') CHARACTER SET latin1 DEFAULT '',set_empty SET('a','b') CHARACTER SET latin1 DEFAULT '',binary_empty VARBINARY(3) NOT NULL DEFAULT '',binary_nul VARBINARY(3) NOT NULL DEFAULT 0x0061ff,unicode_value VARCHAR(16) CHARACTER SET utf8mb4 NOT NULL DEFAULT '😀Ω',literal_null VARCHAR(4) NOT NULL DEFAULT 'NULL',kept_value VARCHAR(20) NOT NULL DEFAULT 'kept',null_value VARCHAR(8) DEFAULT NULL,nullable_absent VARCHAR(8),absent VARCHAR(8) NOT NULL) ENGINE=InnoDB",mysql::quote_ident(name))).await.unwrap();
    }
    source
        .query_drop(format!(
            "INSERT INTO {}(id,absent) VALUES(1,'given')",
            mysql::quote_ident(&names[1])
        ))
        .await
        .unwrap();
    // A populated row lets us independently compare a source literal default.
    let direct: Option<String> = source
        .query_first(format!(
            "SELECT CONVERT(DEFAULT(empty_value) USING utf8mb4) FROM {} LIMIT 1",
            mysql::quote_ident(&names[1])
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(direct, Some(String::new()));
    let kept: (String, String, u8) = source
        .query_first(format!(
            "SELECT CONVERT(DEFAULT(t.kept_value) USING utf8mb4),HEX(DEFAULT(t.kept_value)),DEFAULT(t.kept_value) IS NULL FROM {} AS t LIMIT 1",
            mysql::quote_ident(&names[1])
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept, ("kept".into(), "6B657074".into(), 0));
    // Do not infer empty-table defaults by evaluating DEFAULT(column) over a
    // NULL-extended row; server versions differ there. Catalog inspection
    // below is the migration contract and must leave the source untouched.
    source
        .query_drop("START TRANSACTION READ ONLY")
        .await
        .unwrap();
    let catalog = mysql::inspect(&mut source).await.unwrap();
    for (index, name) in names.iter().enumerate() {
        let table = catalog.tables.iter().find(|t| t.name == *name).unwrap();
        for (column, expected) in [
            ("empty_value", ""),
            ("nullable_empty", ""),
            ("char_empty", ""),
            ("enum_empty", ""),
            ("set_empty", ""),
            ("binary_empty", "0x"),
            ("binary_nul", "0x0061FF"),
            ("unicode_value", "😀Ω"),
            ("literal_null", "NULL"),
            ("kept_value", "kept"),
        ] {
            assert_eq!(
                table
                    .columns
                    .iter()
                    .find(|c| c.name == column)
                    .unwrap()
                    .default
                    .as_deref(),
                Some(expected),
                "{name}.{column}"
            );
        }
        for column in ["null_value", "nullable_absent", "absent"] {
            assert_eq!(
                table
                    .columns
                    .iter()
                    .find(|c| c.name == column)
                    .unwrap()
                    .default,
                None,
                "{name}.{column}"
            );
        }
        let count: u64 = source
            .query_first(format!("SELECT count(*) FROM {}", mysql::quote_ident(name)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            count, index as u64,
            "source inspection must not materialize/write a default row"
        );
    }
    source.query_drop("ROLLBACK").await.unwrap();
    // Evaluate actual source inserts only after read-only inspection, independently of generated DDL.
    source
        .query_drop(format!(
            "INSERT INTO {}(id,absent) VALUES(1,'given')",
            mysql::quote_ident(&names[0])
        ))
        .await
        .unwrap();
    for name in &names {
        let actual:Defaults=source.query_first(format!("SELECT empty_value,nullable_empty,char_empty,enum_empty,set_empty,HEX(binary_empty),HEX(binary_nul),null_value,nullable_absent FROM {} WHERE id=1",mysql::quote_ident(name))).await.unwrap().unwrap();
        assert_eq!(
            actual,
            (
                "".into(),
                "".into(),
                "".into(),
                "".into(),
                "".into(),
                "".into(),
                "0061FF".into(),
                None,
                None
            )
        );
        let unicode: String = source
            .query_first(format!(
                "SELECT unicode_value FROM {} WHERE id=1",
                mysql::quote_ident(name)
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unicode, "😀Ω");
    }
    config.migration.reset_sequences = false;
    let mut target = postgres::connect(&config.target, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let target_catalog = postgres::inspect(&mut target.client, &schema)
        .await
        .unwrap();
    let prepared = plan::build(&config, &catalog, &target_catalog).unwrap();
    for ddl in &prepared.ddl {
        target.client.batch_execute(&ddl.sql).await.unwrap();
    }
    for name in &names {
        let table = postgres::qualified(&schema, name);
        target
            .client
            .batch_execute(&format!("INSERT INTO {table}(id,absent) VALUES(1,'given')"))
            .await
            .unwrap();
        let row=target.client.query_one(&format!("SELECT empty_value,nullable_empty,char_empty::text,enum_empty::text,set_empty,binary_empty,binary_nul,unicode_value,literal_null,null_value,nullable_absent,kept_value FROM {table} WHERE id=1"),&[]).await.unwrap();
        for index in 0..4 {
            assert_eq!(row.get::<_, String>(index), "");
        }
        assert_eq!(row.get::<_, Vec<String>>(4), Vec::<String>::new());
        assert_eq!(row.get::<_, Vec<u8>>(5), Vec::<u8>::new());
        assert_eq!(row.get::<_, Vec<u8>>(6), [0, 0x61, 0xff]);
        assert_eq!(row.get::<_, String>(7), "😀Ω");
        assert_eq!(row.get::<_, String>(8), "NULL");
        assert_eq!(row.get::<_, Option<String>>(9), None);
        assert_eq!(row.get::<_, Option<String>>(10), None);
        assert_eq!(row.get::<_, String>(11), "kept");
    }
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
    for name in &names {
        source
            .query_drop(format!("DROP TABLE {}", mysql::quote_ident(name)))
            .await
            .unwrap();
    }
    source.disconnect().await.unwrap();
    target.close().await.unwrap();
}
