use my2pg::{config::SourceConfig, mysql};
use mysql_async::{Row, prelude::Queryable};
use std::env;

#[tokio::test]
#[ignore = "requires owned Oracle MySQL 5.7/8.0/8.4 and PostgreSQL 16/17/18 TLS fixture"]
async fn native_show_create_and_binary_metadata_lose_unused_labels_in_both_sql_modes() {
    let config: SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_MYSQL_URL", "consistency":"frozen",
        "ca_file":env::var("MY2PG_TLS_CA").unwrap()
    }))
    .unwrap();
    let mut source = mysql::connect(&config, &env::var("MY2PG_MYSQL_ROOT_URL").unwrap(), 65536)
        .await
        .unwrap();
    let version: String = source
        .query_first("SELECT VERSION()")
        .await
        .unwrap()
        .unwrap();
    assert!(
        version.starts_with("5.7.") || version.starts_with("8.0.") || version.starts_with("8.4."),
        "unreviewed Oracle MySQL version: {version}"
    );
    for mode in ["", "NO_BACKSLASH_ESCAPES"] {
        source
            .exec_drop("SET SESSION sql_mode=?", (mode,))
            .await
            .unwrap();
        source
            .query_drop("SET character_set_results=utf8mb4")
            .await
            .unwrap();
        source
            .query_drop("DROP TABLE IF EXISTS source.t10_label_show")
            .await
            .unwrap();
        let quote = |label: &str| {
            let label = if mode.is_empty() {
                label.replace('\\', "\\\\")
            } else {
                label.to_string()
            };
            format!("'{}'", label.replace('\'', "''"))
        };
        let labels = ["😀", "𐐷", "?", "quote'label", "slash\\label"];
        let declared = labels
            .iter()
            .map(|label| quote(label))
            .collect::<Vec<_>>()
            .join(",");
        source.query_drop(format!("CREATE TABLE source.t10_label_show (e ENUM({declared}), s SET({declared})) ENGINE=InnoDB CHARACTER SET utf8mb4")).await.unwrap();
        source
            .query_drop("START TRANSACTION READ ONLY")
            .await
            .unwrap();
        for results in ["utf8mb4", "binary"] {
            source
                .query_drop(format!("SET character_set_results={results}"))
                .await
                .unwrap();
            let columns: Vec<(String,String)> = source.query("SELECT COLUMN_NAME,COLUMN_TYPE FROM information_schema.COLUMNS WHERE TABLE_SCHEMA='source' AND TABLE_NAME='t10_label_show' ORDER BY ORDINAL_POSITION").await.unwrap();
            for (_, kind) in &columns {
                assert!(kind.contains("('?','?','?'"), "{mode}/{results}: {kind}");
                assert!(!kind.contains('😀') && !kind.contains('𐐷'));
            }
            let show: (String, String) = source
                .query_first("SHOW CREATE TABLE source.t10_label_show")
                .await
                .unwrap()
                .unwrap();
            println!(
                "mode={mode:?} results={results} text SHOW CREATE: {}",
                show.1
            );
            assert!(show.1.contains("enum('?','?','?'"));
            assert!(show.1.contains("set('?','?','?'"));
            assert!(!show.1.contains('😀') && !show.1.contains('𐐷'));
            let binary: Result<Option<(String, String)>, _> = source
                .exec_first("SHOW CREATE TABLE source.t10_label_show", ())
                .await;
            match binary {
                Ok(Some(binary)) => {
                    assert_eq!(binary, show);
                    println!("binary SHOW CREATE has identical loss");
                }
                Ok(None) => panic!("binary SHOW CREATE returned no row"),
                Err(error) => panic!("binary SHOW CREATE failed: {error}"),
            }
            let stmt = source
                .prep("SELECT e,s FROM source.t10_label_show LIMIT 0")
                .await
                .unwrap();
            assert_eq!(stmt.columns().len(), 2);
            println!(
                "mode={mode:?} results={results} prepared field definitions: {:?}",
                stmt.columns()
            );
            let rows: Vec<Row> = source.exec(&stmt, ()).await.unwrap();
            assert!(rows.is_empty());
            source.close(stmt).await.unwrap();
        }
        let unchanged: u64 = source
            .query_first("SELECT COUNT(*) FROM source.t10_label_show")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged, 0, "unused domains cannot be inferred from data");
        source.query_drop("ROLLBACK").await.unwrap();
        source
            .query_drop("DROP TABLE source.t10_label_show")
            .await
            .unwrap();
    }
    source.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned Oracle MySQL8.4 TLS fixture"]
async fn native_recursive_cte_widens_enum_set_and_cannot_recover_labels() {
    let config: SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_MYSQL_URL", "consistency":"frozen",
        "ca_file":env::var("MY2PG_TLS_CA").unwrap()
    }))
    .unwrap();
    let mut source = mysql::connect(&config, &env::var("MY2PG_MYSQL_ROOT_URL").unwrap(), 65536)
        .await
        .unwrap();
    source
        .query_drop("DROP TABLE IF EXISTS source.t10_label_recovery")
        .await
        .unwrap();
    source.query_drop("CREATE TABLE source.t10_label_recovery (e ENUM('','😀','?','3','quote''label','slash\\\\label'), s SET('😀','?','3','quote''label','slash\\\\label')) ENGINE=InnoDB CHARACTER SET utf8mb4").await.unwrap();
    source
        .query_drop("START TRANSACTION READ ONLY")
        .await
        .unwrap();
    let count: u64 = source
        .query_first("SELECT COUNT(*) FROM source.t10_label_recovery")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count, 0);
    let mut observations = Vec::new();
    for (column, count) in [("e", 6_u64), ("s", 5)] {
        let assignment = if column == "e" {
            "n+1"
        } else {
            "CAST(1 AS UNSIGNED) << n"
        };
        let sql = format!(
            "WITH RECURSIVE labels(n,value) AS (SELECT 0,t.`{column}` FROM source.t10_label_recovery AS t RIGHT JOIN (SELECT 1) AS anchor ON FALSE UNION ALL SELECT n+1,{assignment} FROM labels WHERE n<{count}) SELECT n,CONVERT(value USING utf8mb4),HEX(value),value+0 FROM labels WHERE n>0 ORDER BY n"
        );
        let result = source.query::<Row, _>(&sql).await;
        println!("read-only {column} query: {sql}");
        match result {
            Ok(rows) => {
                for row in &rows {
                    println!("read-only {column} result: {row:?}");
                }
                observations.push(
                    rows.into_iter()
                        .map(|row| row.get::<String, _>(1).unwrap())
                        .collect::<Vec<_>>(),
                );
            }
            Err(error) => {
                println!("read-only {column} error: {error}");
                observations.push(Vec::new());
            }
        }
    }
    let unchanged: u64 = source
        .query_first("SELECT COUNT(*) FROM source.t10_label_recovery")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged, 0);
    source.query_drop("ROLLBACK").await.unwrap();
    source
        .query_drop("DROP TABLE source.t10_label_recovery")
        .await
        .unwrap();
    source.disconnect().await.unwrap();
    assert_eq!(observations[0], ["1", "2", "3", "4", "5", "6"]);
    assert_eq!(observations[1], ["1", "2", "4", "8", "16"]);
}
