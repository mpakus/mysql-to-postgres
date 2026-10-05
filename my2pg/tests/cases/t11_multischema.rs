use my2pg::{config::TargetConfig, postgres};
use std::{collections::BTreeSet, env};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("required owned fixture variable {name}"))
}

#[tokio::test]
#[ignore = "requires owned native PostgreSQL TLS fixture"]
async fn namespace_union_preserves_distinct_names_privileges_defaults_and_graph_roots() {
    let config: TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_POSTGRES_URL", "schema":"t11_ns_a", "ca_file":required("MY2PG_TLS_CA")
    }))
    .unwrap();
    let mut target = postgres::connect(&config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    target.client.batch_execute(r#"
        CREATE SCHEMA t11_ns_a; CREATE SCHEMA t11_ns_b; CREATE SCHEMA t11_ns_empty;
        CREATE TYPE t11_ns_a."State" AS ENUM ('z','a');
        CREATE TYPE t11_ns_b."State" AS ENUM ('b');
        CREATE TABLE t11_ns_a.same(id integer PRIMARY KEY, choice t11_ns_a."State", CONSTRAINT actual_check CHECK(id>0));
        CREATE TABLE t11_ns_b.same(id integer PRIMARY KEY REFERENCES t11_ns_a.same(id), choice t11_ns_b."State");
        CREATE VIEW t11_ns_b.extra AS SELECT id FROM t11_ns_a.same;
        INSERT INTO t11_ns_a.same VALUES(1,'z'); INSERT INTO t11_ns_b.same VALUES(1,'b');
        SET extra_float_digits=-2; SET IntervalStyle='iso_8601';
        CREATE ROLE t11_ns_reader;
        GRANT USAGE ON SCHEMA t11_ns_a TO t11_ns_reader;
        GRANT SELECT ON ALL TABLES IN SCHEMA t11_ns_a TO t11_ns_reader;
    "#).await.unwrap();
    let names = [
        "t11_ns_b",
        "t11_ns_a",
        "t11_ns_b",
        "t11_ns_empty",
        "t11_ns_missing",
    ]
    .map(String::from);
    let catalog = postgres::inspect_schemas(&mut target.client, "t11_ns_a", &names)
        .await
        .unwrap();
    let namespaces = catalog.namespaces.as_ref().unwrap();
    assert_eq!(namespaces.len(), 4, "deduplicate requested namespaces");
    let primary = namespaces.iter().find(|n| n.schema == "t11_ns_a").unwrap();
    let secondary = namespaces.iter().find(|n| n.schema == "t11_ns_b").unwrap();
    assert!(primary.oid.is_some() && secondary.oid.is_some());
    assert_ne!(primary.oid, secondary.oid);
    assert!(primary.can_use && primary.can_create_objects);
    assert_eq!(primary.occupied_names["same"], "r");
    assert_eq!(secondary.occupied_names["same"], "r");
    assert_eq!(secondary.occupied_names["extra"], "v");
    assert!(
        !catalog.occupied_names.contains_key("extra"),
        "primary summary must retain its namespace"
    );
    for namespace in [primary, secondary] {
        assert!(namespace.occupied_types.contains("State"));
    }
    let empty = namespaces
        .iter()
        .find(|n| n.schema == "t11_ns_empty")
        .unwrap();
    let missing = namespaces
        .iter()
        .find(|n| n.schema == "t11_ns_missing")
        .unwrap();
    assert!(empty.schema_exists && empty.oid.is_some() && empty.occupied_names.is_empty());
    assert!(
        !missing.schema_exists
            && missing.oid.is_none()
            && !missing.can_use
            && !missing.can_create_objects
    );
    assert!(missing.occupied_names.is_empty() && missing.occupied_types.is_empty());
    assert_eq!(catalog.deparse.as_ref().unwrap().extra_float_digits, -2);
    assert_eq!(catalog.deparse.as_ref().unwrap().interval_style, "iso_8601");
    assert_eq!(catalog.tables.len(), 2);
    let relation_oids: BTreeSet<_> = catalog
        .tables
        .iter()
        .map(|t| t.observed.as_ref().unwrap().oid)
        .collect();
    let graph = catalog.dependency_graph.as_ref().unwrap();
    assert_eq!(graph.relation_root_count, 2);
    assert_eq!(
        graph
            .parts
            .iter()
            .filter(|p| p.relation_root)
            .map(|p| p.node_oid)
            .collect::<BTreeSet<_>>(),
        relation_oids
    );
    assert!(
        graph
            .parts
            .iter()
            .any(|p| p.node_schema.as_deref() == Some("t11_ns_b")
                && p.node_name.as_deref() == Some("extra"))
    );
    let types = catalog.type_parts.as_ref().unwrap();
    let enum_oids: BTreeSet<_> = types
        .iter()
        .filter(|p| p.type_name.as_deref() == Some("State"))
        .filter_map(|p| p.type_oid)
        .collect();
    assert_eq!(enum_oids.len(), 2);
    assert_eq!(catalog.enums["State"], ["z", "a"]);
    let constraints = &catalog
        .tables
        .iter()
        .find(|t| t.schema == "t11_ns_a")
        .unwrap()
        .observed
        .as_ref()
        .unwrap()
        .constraint_parts;
    assert!(
        constraints
            .iter()
            .filter(|p| p.kind == "c")
            .all(|p| p.check_expression.as_deref() == Some("(id > 0)"))
    );
    assert!(
        constraints
            .iter()
            .filter(|p| p.kind != "c")
            .all(|p| p.check_expression.is_none())
    );
    target
        .client
        .batch_execute("SET ROLE t11_ns_reader")
        .await
        .unwrap();
    let restricted = postgres::inspect_schemas(&mut target.client, "t11_ns_a", &names)
        .await
        .unwrap();
    let facts = restricted.namespaces.as_ref().unwrap();
    assert!(
        facts
            .iter()
            .find(|n| n.schema == "t11_ns_a")
            .unwrap()
            .can_use
    );
    assert!(
        !facts
            .iter()
            .find(|n| n.schema == "t11_ns_a")
            .unwrap()
            .can_create_objects
    );
    assert!(
        !facts
            .iter()
            .find(|n| n.schema == "t11_ns_b")
            .unwrap()
            .can_use
    );
    assert!(!restricted.can_create_schema);
    target
        .client
        .batch_execute("RESET ROLE; SET extra_float_digits=1; SET IntervalStyle='postgres'")
        .await
        .unwrap();
    let rows: i64 = target
        .client
        .query_one(
            "SELECT (SELECT count(*) FROM t11_ns_a.same)+(SELECT count(*) FROM t11_ns_b.same)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 2, "inspection must not mutate rows");
    let read_only: String = target
        .client
        .query_one("SHOW transaction_read_only", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        read_only, "off",
        "inspection rolls back its owned transaction"
    );
    target.client.batch_execute("DROP SCHEMA t11_ns_b CASCADE;DROP SCHEMA t11_ns_a CASCADE;DROP SCHEMA t11_ns_empty;DROP ROLE t11_ns_reader").await.unwrap();
    target.close().await.unwrap();
}
