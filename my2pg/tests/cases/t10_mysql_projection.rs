use futures_util::TryStreamExt;
use my2pg::{
    config::SourceConfig,
    model::{MigrationPlan, RawValue, ValueKind},
    mysql,
};
use mysql_async::prelude::Queryable;
use std::env;

#[tokio::test]
#[ignore = "requires owned Oracle MySQL TLS fixture"]
async fn enum_projection_distinguishes_valid_empty_invalid_and_null_even_for_text_override() {
    let source: SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_MYSQL_URL", "consistency":"frozen",
        "ca_file": env::var("MY2PG_TLS_CA").unwrap()
    }))
    .unwrap();
    let mut admin = mysql::connect(&source, &env::var("MY2PG_MYSQL_ROOT_URL").unwrap(), 16384)
        .await
        .unwrap();
    admin.query_drop("SET SESSION sql_mode=''").await.unwrap();
    admin
        .query_drop("DROP TABLE IF EXISTS source.t10_enum_projection")
        .await
        .unwrap();
    admin.query_drop("CREATE TABLE source.t10_enum_projection(id INT PRIMARY KEY, state ENUM('','a,b','é') NULL) ENGINE=InnoDB CHARACTER SET utf8mb4").await.unwrap();
    admin.query_drop("INSERT INTO source.t10_enum_projection VALUES(1,''),(2,'a,b'),(3,'invalid-sentinel'),(4,NULL)").await.unwrap();
    let fixture: MigrationPlan =
        serde_json::from_str(include_str!("../contracts/plan.json")).unwrap();
    let mut table = fixture.tables[0].clone();
    table.source_name = "t10_enum_projection".into();
    table.columns.resize(2, table.columns[0].clone());
    table.columns[0].source_name = "id".into();
    table.columns[1].source_name = "state".into();
    table.columns[1].enum_labels = vec!["".into(), "a,b".into(), "é".into()];
    table.columns[1].charset = Some("utf8mb4".into());
    let mut reader = mysql::connect(&source, &env::var("MY2PG_MYSQL_URL").unwrap(), 16384)
        .await
        .unwrap();
    for kind in [ValueKind::Enum, ValueKind::Text] {
        table.columns[1].kind = kind;
        let mut stream = mysql::table_stream(&mut reader, "source", &table)
            .await
            .unwrap();
        let mut rows = Vec::new();
        while let Some(row) = stream.try_next().await.unwrap() {
            rows.push(mysql::raw_values(row, 16384).unwrap());
        }
        drop(stream);
        rows.sort_by_key(|row| match row[0] {
            RawValue::Int(id) => id as u64,
            RawValue::UInt(id) => id,
            _ => panic!("numeric source identity"),
        });
        assert_eq!(rows.len(), 4);
        for (row, expected) in rows[..3].iter().zip([1, 2, 0]) {
            let ordinal = match row[1] {
                RawValue::Int(value) => u64::try_from(value).unwrap(),
                RawValue::UInt(value) => value,
                _ => panic!("ENUM projection must be a numeric ordinal"),
            };
            assert_eq!(ordinal, expected);
        }
        assert_eq!(rows[3][1], RawValue::Null);
    }
    reader.disconnect().await.unwrap();
    admin
        .query_drop("DROP TABLE source.t10_enum_projection")
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
}
