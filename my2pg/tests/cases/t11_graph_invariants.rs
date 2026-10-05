use my2pg::{
    config::TargetConfig,
    postgres::{self, catalog_graph},
};
use tokio_postgres::Client;

const GRAPH: &str = include_str!("../../src/postgres/observed_dependency_graph.sql");
const TYPES: &str = include_str!("../../src/postgres/observed_types.sql");

async fn target(schema: &str) -> postgres::TargetConnection {
    let config: TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_POSTGRES_URL","schema":schema,
        "ca_file":std::env::var("MY2PG_TLS_CA").expect("owned TLS fixture required")
    }))
    .unwrap();
    postgres::connect(
        &config,
        &std::env::var("MY2PG_POSTGRES_URL").expect("owned target required"),
    )
    .await
    .unwrap()
}
async fn root(client: &Client) -> u32 {
    client
        .query_one("SELECT 't11_inv_graph.root'::regclass::oid", &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
#[ignore = "requires owned PostgreSQL TLS fixture"]
async fn namespace_only_type_headers_refuse_dangling_permission_owner_and_extension_facts() {
    let target = target("t11_inv_header").await;
    let client = &target.client;
    client
        .batch_execute("SET statement_timeout='5s'; CREATE SCHEMA t11_inv_header")
        .await
        .unwrap();
    let valid = catalog_graph::inspect_types(client, "t11_inv_header")
        .await
        .unwrap();
    assert_eq!(valid.len(), 1);
    assert_eq!(valid[0].type_oid, None);
    let complete = client.query(TYPES, &[&"t11_inv_header"]).await.unwrap();
    for missing in complete[0].columns() {
        let projection = complete[0]
            .columns()
            .iter()
            .filter(|column| column.name() != missing.name())
            .map(|column| format!("\"{}\"", column.name()))
            .collect::<Vec<_>>()
            .join(",");
        let query = format!("SELECT {projection} FROM ({TYPES}) observation");
        let rows = client.query(&query, &[&"t11_inv_header"]).await.unwrap();
        assert!(
            catalog_graph::decode_type_part(&rows[0]).is_err(),
            "missing alias {} was defaulted",
            missing.name()
        );
    }
    for (original, replacement) in [
        (
            "CASE WHEN t.oid IS NOT NULL THEN pg_catalog.pg_has_role(t.typowner, 'USAGE') END AS can_alter",
            "true AS can_alter",
        ),
        (
            "t.typowner AS type_owner_role_oid",
            "123::oid AS type_owner_role_oid",
        ),
        (
            "t.typrelid AS type_relation_oid",
            "123::oid AS type_relation_oid",
        ),
        (
            "ext.oid AS extension_oid, ext.extname::text AS extension_name",
            "123::oid AS extension_oid, 'orphan'::text AS extension_name",
        ),
    ] {
        assert!(TYPES.contains(original), "projection spelling changed");
        let query = TYPES.replace(original, replacement);
        let rows = client.query(&query, &[&"t11_inv_header"]).await.unwrap();
        assert!(
            catalog_graph::decode_type_part(&rows[0]).is_err(),
            "accepted orphan projection {original}"
        );
    }
    client
        .batch_execute("DROP SCHEMA t11_inv_header")
        .await
        .unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned PostgreSQL TLS fixture"]
async fn graph_edges_require_incident_raw_identity_and_coherent_owner_extension_fields() {
    let target = target("t11_inv_graph").await;
    let client = &target.client;
    client.batch_execute("SET statement_timeout='5s'; CREATE SCHEMA t11_inv_graph; CREATE TABLE t11_inv_graph.root(id integer PRIMARY KEY); CREATE VIEW t11_inv_graph.reader AS SELECT id FROM t11_inv_graph.root").await.unwrap();
    let id = root(client).await;
    let roots = vec![id];
    catalog_graph::inspect_dependency_graph(client, &roots, &[])
        .await
        .unwrap();
    let replacements = [
        (
            "edge.direction AS dependency_direction",
            "CASE WHEN edge.direction IS NOT NULL THEN 'sideways'::text END AS dependency_direction",
        ),
        (
            "edge.objid AS dependent_oid, edge.objsubid AS dependent_sub_id",
            "CASE WHEN edge.objid IS NOT NULL THEN 999::oid END AS dependent_oid, edge.objsubid AS dependent_sub_id",
        ),
        (
            "edge.refobjid AS referenced_oid, edge.refobjsubid AS referenced_sub_id",
            "CASE WHEN edge.refobjid IS NOT NULL THEN 998::oid END AS referenced_oid, edge.refobjsubid AS referenced_sub_id",
        ),
        (
            "own_rel.oid AS node_owner_relation_oid",
            "NULL::oid AS node_owner_relation_oid",
        ),
        (
            "member_ext.oid AS node_extension_oid, member_ext.extname::text AS node_extension_name",
            "123::oid AS node_extension_oid, NULL::text AS node_extension_name",
        ),
    ];
    // Each endpoint rewrite is paired with the other, so no outgoing or incoming
    // edge can incidentally remain valid against its observed node.
    let unrelated = GRAPH
        .replace(replacements[1].0, replacements[1].1)
        .replace(replacements[2].0, replacements[2].1);
    for query in [
        GRAPH.replace(replacements[0].0, replacements[0].1),
        unrelated,
        GRAPH.replace(replacements[3].0, replacements[3].1),
        GRAPH.replace(replacements[4].0, replacements[4].1),
    ] {
        assert_ne!(query, GRAPH);
        let rows = client
            .query(&query, &[&roots, &Vec::<u32>::new()])
            .await
            .unwrap();
        assert!(
            catalog_graph::decode_dependency_graph(&rows).is_err(),
            "accepted contradictory graph projection"
        );
    }
    client
        .batch_execute("DROP SCHEMA t11_inv_graph CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires owned PostgreSQL TLS fixture"]
async fn native_system_column_dependencies_preserve_exact_negative_attribute_ids() {
    let target = target("t11_inv_system").await;
    let client = &target.client;
    client.batch_execute("SET statement_timeout='5s'; CREATE SCHEMA t11_inv_system; CREATE TABLE t11_inv_system.root(id integer); CREATE VIEW t11_inv_system.reader AS SELECT ctid AS row_location,tableoid AS relation_identity,xmin AS row_version FROM t11_inv_system.root").await.unwrap();
    let id: u32 = client
        .query_one("SELECT 't11_inv_system.root'::regclass::oid", &[])
        .await
        .unwrap()
        .get(0);
    let attributes=client.query("SELECT a.attname::text,a.attnum,i.type,i.identity FROM pg_catalog.pg_attribute a CROSS JOIN LATERAL pg_catalog.pg_identify_object('pg_catalog.pg_class'::regclass,a.attrelid,a.attnum) i WHERE a.attrelid=$1 AND a.attname IN ('ctid','tableoid','xmin') ORDER BY a.attname",&[&id]).await.unwrap();
    assert_eq!(attributes.len(), 3);
    let mut expected = std::collections::BTreeSet::new();
    for row in &attributes {
        let name: String = row.get(0);
        let ordinal: i16 = row.get(1);
        assert!(ordinal < 0);
        assert_eq!(row.get::<_, String>(2), "table column");
        assert!(row.get::<_, String>(3).ends_with(&name));
        expected.insert(i32::from(ordinal));
        let count:i64=client.query_one("SELECT count(*) FROM pg_catalog.pg_depend WHERE refclassid='pg_catalog.pg_class'::regclass AND refobjid=$1 AND refobjsubid=$2",&[&id,&i32::from(ordinal)]).await.unwrap().get(0);
        assert!(count > 0, "native view dependency absent for {name}");
    }
    let graph = catalog_graph::inspect_dependency_graph(client, &[id], &[])
        .await
        .unwrap();
    let observed: std::collections::BTreeSet<_> = graph
        .parts
        .iter()
        .filter_map(|part| {
            part.edge.as_ref().and_then(|edge| {
                if edge.dependency.referenced_oid == Some(id)
                    && edge.dependency.referenced_catalog.as_deref() == Some("pg_catalog.pg_class")
                {
                    edge.dependency.referenced_sub_id.filter(|sub| *sub < 0)
                } else {
                    None
                }
            })
        })
        .collect();
    assert_eq!(observed, expected);
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM t11_inv_system.root", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    client
        .batch_execute("DROP SCHEMA t11_inv_system CASCADE")
        .await
        .unwrap();
    target.close().await.unwrap();
}
