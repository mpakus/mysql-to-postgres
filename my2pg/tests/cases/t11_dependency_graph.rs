use my2pg::{
    config::TargetConfig,
    model::{TargetDependencyGraph, TargetDependencyGraphPart},
    postgres::{self, catalog_graph},
};
use std::collections::BTreeSet;
use tokio_postgres::Client;

const GRAPH: &str = include_str!("../../src/postgres/observed_dependency_graph.sql");
const TYPES: &str = include_str!("../../src/postgres/observed_types.sql");

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing required {name}"))
}

fn config() -> TargetConfig {
    serde_json::from_value(serde_json::json!({
        "url_env":"MY2PG_POSTGRES_URL", "schema":"t11_graph", "ca_file":required("MY2PG_TLS_CA")
    }))
    .unwrap()
}

async fn oid(client: &Client, sql: &str) -> u32 {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

fn node<'a>(
    graph: &'a TargetDependencyGraph,
    catalog: &str,
    oid: u32,
) -> &'a TargetDependencyGraphPart {
    graph
        .parts
        .iter()
        .find(|p| p.node_catalog == catalog && p.node_oid == oid)
        .unwrap_or_else(|| panic!("missing {catalog} object {oid}"))
}

fn incoming(graph: &TargetDependencyGraph, dependent: u32, referenced: u32, sub: i32) -> bool {
    graph.parts.iter().any(|part| {
        part.edge.as_ref().is_some_and(|edge| {
            edge.dependency.dependency_catalog.as_deref() == Some("pg_catalog.pg_depend")
                && edge.dependency.dependency_direction.as_deref() == Some("incoming")
                && edge.dependency.dependent_oid == Some(dependent)
                && edge.dependency.referenced_oid == Some(referenced)
                && edge.dependency.referenced_sub_id == Some(sub)
        })
    })
}

#[tokio::test]
#[ignore = "requires owned PostgreSQL16 TLS fixture"]
async fn native_dependency_closure_types_headers_and_errors_are_distinct() {
    let target = postgres::connect(&config(), &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    let client = &target.client;
    client
        .batch_execute("SET statement_timeout='5s'; CREATE SCHEMA t11_graph; CREATE SCHEMA t11_graph_other; CREATE SCHEMA t11_graph_empty")
        .await
        .unwrap();
    let empty = catalog_graph::inspect_dependency_graph(client, &[], &[])
        .await
        .unwrap();
    assert_eq!(empty.relation_root_count, 0);
    assert_eq!(empty.type_root_count, 0);
    assert_eq!(empty.event_trigger_count, 0);
    assert!(empty.parts.is_empty());
    let missing = catalog_graph::inspect_types(client, "t11_graph_missing")
        .await
        .unwrap();
    assert!(missing.is_empty());
    let empty_types = catalog_graph::inspect_types(client, "t11_graph_empty")
        .await
        .unwrap();
    assert_eq!(empty_types.len(), 1);
    assert_eq!(empty_types[0].type_oid, None);
    assert!(empty_types[0].namespace_oid > 0);
    for roots in [vec![0_u32], vec![u32::MAX]] {
        assert!(
            catalog_graph::inspect_dependency_graph(client, &roots, &[])
                .await
                .is_err()
        );
        assert!(
            catalog_graph::inspect_dependency_graph(client, &[], &roots)
                .await
                .is_err()
        );
    }
    assert!(catalog_graph::decode_dependency_graph(&[]).is_err());
    let null_roots: Option<Vec<u32>> = None;
    let rows = client
        .query(GRAPH, &[&null_roots, &Vec::<u32>::new()])
        .await
        .unwrap();
    assert!(!rows[0].get::<_, bool>("roots_valid"));
    assert!(catalog_graph::decode_dependency_graph(&rows).is_err());

    client.batch_execute(r#"
        CREATE TYPE t11_graph."State" AS ENUM ('z', '', 'a,b', 'a\b');
        ALTER TYPE t11_graph."State" ADD VALUE 'before' BEFORE 'z';
        COMMENT ON TYPE t11_graph."State" IS 'ordered native enum';
        CREATE TYPE t11_graph_other."State" AS ENUM ('other');
        CREATE TYPE t11_graph.empty_enum AS ENUM ();
        CREATE TYPE t11_graph.unused_enum AS ENUM ('unused');
        ALTER EXTENSION plpgsql ADD TYPE t11_graph.unused_enum;
        CREATE TYPE t11_graph.composite AS (id integer, label text);
        CREATE DOMAIN t11_graph.positive AS integer DEFAULT 7 NOT NULL CHECK (VALUE > 0);
        ALTER DOMAIN t11_graph.positive ADD CONSTRAINT pending CHECK (VALUE < 100) NOT VALID;
        CREATE DOMAIN t11_graph_other.state_domain AS t11_graph."State";
        CREATE TABLE t11_graph.root (id serial PRIMARY KEY, "Second,key" integer UNIQUE, state t11_graph."State" DEFAULT 'z');
        CREATE TABLE t11_graph.peer (id integer PRIMARY KEY, root_id integer REFERENCES t11_graph.root(id));
        ALTER TABLE t11_graph.root ADD CONSTRAINT peer_key FOREIGN KEY ("Second,key") REFERENCES t11_graph.peer(id);
        INSERT INTO t11_graph.peer VALUES (1,NULL);
        INSERT INTO t11_graph.root("Second,key") VALUES (1);
        CREATE VIEW t11_graph_other.v1 AS SELECT id,"Second,key" FROM t11_graph.root;
        CREATE VIEW t11_graph_other.v2 AS SELECT * FROM t11_graph_other.v1;
        CREATE FUNCTION t11_graph_other.view_reader() RETURNS integer LANGUAGE SQL
            RETURN (SELECT count(*)::integer FROM t11_graph_other.v2);
        CREATE FUNCTION t11_graph_other.row_reader(t11_graph.root) RETURNS integer LANGUAGE SQL RETURN 1;
        CREATE FUNCTION t11_graph_other.cycle_a() RETURNS integer LANGUAGE SQL RETURN 1;
        CREATE FUNCTION t11_graph_other.cycle_b() RETURNS integer LANGUAGE SQL RETURN t11_graph_other.cycle_a();
        CREATE OR REPLACE FUNCTION t11_graph_other.cycle_a() RETURNS integer LANGUAGE SQL
            RETURN (SELECT COALESCE(max(id),0)+t11_graph_other.cycle_b() FROM t11_graph.root);
        CREATE TABLE t11_graph_other.shared_default(id bigint DEFAULT nextval('t11_graph.root_id_seq'::regclass));
        CREATE FUNCTION t11_graph_other.sequence_reader() RETURNS bigint LANGUAGE SQL
            RETURN nextval('t11_graph.root_id_seq'::regclass);
        CREATE TABLE t11_graph_other.function_default(id bigint DEFAULT t11_graph_other.sequence_reader());
        CREATE TABLE t11_graph_other.enum_user(state t11_graph."State"[], other t11_graph_other."State");
        CREATE FUNCTION t11_graph_other.enum_reader(t11_graph."State") RETURNS t11_graph."State" LANGUAGE SQL RETURN $1;
        CREATE POLICY selected_policy ON t11_graph.root USING (true);
        CREATE STATISTICS t11_graph.selected_stats ON id,"Second,key" FROM t11_graph.root;
        CREATE ROLE t11_graph_owner;
        GRANT USAGE,CREATE ON SCHEMA t11_graph TO t11_graph_owner;
        ALTER TABLE t11_graph.root OWNER TO t11_graph_owner;
        CREATE ROLE t11_graph_reader;
        GRANT SELECT ON t11_graph.root TO t11_graph_reader;
        REVOKE ALL ON TYPE t11_graph."State" FROM PUBLIC;
        CREATE FUNCTION t11_graph_other.on_ddl() RETURNS event_trigger LANGUAGE plpgsql AS $$BEGIN END$$;
        CREATE EVENT TRIGGER t11_graph_ddl ON ddl_command_start EXECUTE FUNCTION t11_graph_other.on_ddl();
        ALTER EVENT TRIGGER t11_graph_ddl DISABLE;
    "#).await.unwrap();

    let root = oid(client, "SELECT 't11_graph.root'::regclass::oid").await;
    let peer = oid(client, "SELECT 't11_graph.peer'::regclass::oid").await;
    let state = oid(client, "SELECT 't11_graph.\"State\"'::regtype::oid").await;
    let sequence = oid(client, "SELECT 't11_graph.root_id_seq'::regclass::oid").await;
    let v1 = oid(client, "SELECT 't11_graph_other.v1'::regclass::oid").await;
    let v2 = oid(client, "SELECT 't11_graph_other.v2'::regclass::oid").await;
    let function = oid(
        client,
        "SELECT 't11_graph_other.view_reader()'::regprocedure::oid",
    )
    .await;
    let cycle_a = oid(
        client,
        "SELECT 't11_graph_other.cycle_a()'::regprocedure::oid",
    )
    .await;
    let cycle_b = oid(
        client,
        "SELECT 't11_graph_other.cycle_b()'::regprocedure::oid",
    )
    .await;
    let row_reader = oid(
        client,
        "SELECT 't11_graph_other.row_reader(t11_graph.root)'::regprocedure::oid",
    )
    .await;
    let enum_reader = oid(
        client,
        "SELECT 't11_graph_other.enum_reader(t11_graph.\"State\")'::regprocedure::oid",
    )
    .await;
    let shared_default = oid(client, "SELECT oid FROM pg_catalog.pg_attrdef WHERE adrelid='t11_graph_other.shared_default'::regclass").await;
    let function_default = oid(client, "SELECT oid FROM pg_catalog.pg_attrdef WHERE adrelid='t11_graph_other.function_default'::regclass").await;
    let enum_user = oid(client, "SELECT 't11_graph_other.enum_user'::regclass::oid").await;
    let other_state = oid(client, "SELECT 't11_graph_other.\"State\"'::regtype::oid").await;
    let domain = oid(
        client,
        "SELECT 't11_graph_other.state_domain'::regtype::oid",
    )
    .await;
    let event = oid(
        client,
        "SELECT oid FROM pg_catalog.pg_event_trigger WHERE evtname='t11_graph_ddl'",
    )
    .await;
    let policy = oid(
        client,
        "SELECT oid FROM pg_catalog.pg_policy WHERE polname='selected_policy'",
    )
    .await;
    let statistics = oid(
        client,
        "SELECT oid FROM pg_catalog.pg_statistic_ext WHERE stxname='selected_stats'",
    )
    .await;
    let state_before: (i64, bool) = {
        let r = client
            .query_one(
                "SELECT last_value,is_called FROM t11_graph.root_id_seq",
                &[],
            )
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };

    client
        .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .unwrap();
    let graph = catalog_graph::inspect_dependency_graph(client, &[root, peer, root], &[state])
        .await
        .unwrap();
    assert_eq!(graph.relation_root_count, 2);
    assert_eq!(graph.type_root_count, 1);
    assert_eq!(graph.event_trigger_count, 1);
    assert!(node(&graph, "pg_catalog.pg_class", root).relation_root);
    assert!(node(&graph, "pg_catalog.pg_class", peer).relation_root);
    assert!(node(&graph, "pg_catalog.pg_type", state).type_root);
    for oid in [v1, v2] {
        let view = node(&graph, "pg_catalog.pg_class", oid);
        assert!(!view.relation_root);
        assert_eq!(view.node_relation_kind.as_deref(), Some("v"));
    }
    for oid in [function, cycle_a, cycle_b, row_reader, enum_reader] {
        let function = node(&graph, "pg_catalog.pg_proc", oid);
        assert_eq!(function.node_function_body_parsed, Some(true));
        assert!(
            function
                .node_function_definition
                .as_deref()
                .unwrap()
                .contains("CREATE OR REPLACE FUNCTION")
        );
    }
    assert!(incoming(&graph, cycle_b, cycle_a, 0));
    assert!(incoming(&graph, cycle_a, cycle_b, 0));
    assert!(graph.parts.iter().any(|p| p.edge.as_ref().is_some_and(
        |e| e.dependency.referenced_oid == Some(root)
            && e.dependency.referenced_sub_id == Some(2)
            && e.dependency.dependent_catalog.as_deref() == Some("pg_catalog.pg_rewrite")
    )));
    let seq = node(&graph, "pg_catalog.pg_class", sequence);
    assert_eq!(seq.node_owner_relation_oid, Some(root));
    assert_eq!(seq.node_owner_column_ordinal, Some(1));
    assert!(incoming(&graph, shared_default, sequence, 0));
    let external_default = node(&graph, "pg_catalog.pg_attrdef", shared_default);
    assert_eq!(
        external_default.node_owner_relation_schema.as_deref(),
        Some("t11_graph_other")
    );
    assert!(
        node(&graph, "pg_catalog.pg_attrdef", function_default)
            .node_owner_relation_oid
            .is_some()
    );
    assert!(
        node(&graph, "pg_catalog.pg_type", domain)
            .node_type_kind
            .as_deref()
            == Some("d")
    );
    assert!(graph.parts.iter().any(|p| {
        p.edge.as_ref().is_some_and(|e| {
            e.dependency.dependent_oid == Some(enum_user)
                && e.dependency.dependent_sub_id == Some(1)
        })
    }));
    assert!(
        !graph
            .parts
            .iter()
            .any(|p| p.node_catalog == "pg_catalog.pg_type" && p.node_oid == other_state)
    );
    let event_part = node(&graph, "pg_catalog.pg_event_trigger", event);
    assert!(event_part.global_event_root);
    assert_eq!(event_part.node_event_enabled.as_deref(), Some("D"));
    node(&graph, "pg_catalog.pg_policy", policy);
    node(&graph, "pg_catalog.pg_statistic_ext", statistics);
    let shared_kinds: BTreeSet<_> = graph
        .parts
        .iter()
        .filter_map(|p| p.edge.as_ref())
        .filter(|e| {
            e.dependency.dependency_catalog.as_deref() == Some("pg_catalog.pg_shdepend")
                && e.dependency.dependent_oid == Some(root)
        })
        .map(|e| e.dependency.dependency_kind.as_deref().unwrap())
        .collect();
    assert!(shared_kinds.contains("o"));
    assert!(shared_kinds.contains("a"));
    assert!(graph.parts.iter().filter_map(|p| p.edge.as_ref()).any(|e| {
        e.dependency.dependency_catalog.as_deref() == Some("pg_catalog.pg_shdepend")
            && e.dependency.referenced_type.as_deref() == Some("role")
            && e.dependency.referenced_name.as_deref() == Some("t11_graph_reader")
    }));
    assert!(!graph.parts.iter().any(|p| matches!(
        p.node_catalog.as_str(),
        "pg_catalog.pg_namespace" | "pg_catalog.pg_authid"
    )));
    // Recursive paths terminate without multiplying incident rows; real catalog duplicates remain.
    let incident: i64 = client
        .query_one(
            r#"
        SELECT (SELECT count(*) FROM pg_catalog.pg_depend WHERE
            (classid='pg_catalog.pg_proc'::regclass AND objid=$1::oid) OR
            (refclassid='pg_catalog.pg_proc'::regclass AND refobjid=$1::oid)) +
            (SELECT count(*) FROM pg_catalog.pg_shdepend WHERE
            dbid=(SELECT oid FROM pg_catalog.pg_database WHERE datname=current_database())
            AND classid='pg_catalog.pg_proc'::regclass AND objid=$1::oid)
    "#,
            &[&cycle_a],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        graph
            .parts
            .iter()
            .filter(|p| p.node_catalog == "pg_catalog.pg_proc" && p.node_oid == cycle_a)
            .count(),
        incident as usize
    );
    let signature_edges: i64 = client.query_one("SELECT count(*) FROM pg_catalog.pg_depend WHERE classid='pg_catalog.pg_proc'::regclass AND objid=$1::oid AND refclassid='pg_catalog.pg_type'::regclass AND refobjid=$2::oid", &[&enum_reader,&state]).await.unwrap().get(0);
    assert_eq!(signature_edges, 2);
    assert_eq!(
        graph
            .parts
            .iter()
            .filter(|p| p.node_catalog == "pg_catalog.pg_proc" && p.node_oid == enum_reader)
            .filter_map(|p| p.edge.as_ref())
            .filter(|e| e.dependency.referenced_oid == Some(state))
            .count(),
        signature_edges as usize
    );

    let types = catalog_graph::inspect_types(client, "t11_graph")
        .await
        .unwrap();
    let labels: Vec<_> = types
        .iter()
        .filter(|t| t.type_oid == Some(state))
        .map(|t| {
            (
                t.enum_label.as_deref().unwrap(),
                t.enum_label_oid.unwrap(),
                t.enum_sort_order.unwrap(),
            )
        })
        .collect();
    assert_eq!(
        labels.iter().map(|x| x.0).collect::<Vec<_>>(),
        ["before", "z", "", "a,b", "a\\b"]
    );
    assert!(labels[0].1 > labels[1].1);
    assert!(labels.windows(2).all(|w| w[0].2 < w[1].2));
    assert_eq!(
        types
            .iter()
            .find(|t| t.type_oid == Some(state))
            .unwrap()
            .type_comment
            .as_deref(),
        Some("ordered native enum")
    );
    let empty_enum = types
        .iter()
        .find(|t| t.type_name.as_deref() == Some("empty_enum"))
        .unwrap();
    assert_eq!(empty_enum.type_kind.as_deref(), Some("e"));
    assert_eq!(empty_enum.enum_label_oid, None);
    assert_eq!(empty_enum.enum_label, None);
    let unused = types
        .iter()
        .find(|t| t.type_name.as_deref() == Some("unused_enum"))
        .unwrap();
    assert_eq!(unused.extension_name.as_deref(), Some("plpgsql"));
    assert!(unused.extension_oid.is_some());
    let positive: Vec<_> = types
        .iter()
        .filter(|t| t.type_name.as_deref() == Some("positive"))
        .collect();
    assert!(positive.iter().all(|t| t.type_kind.as_deref() == Some("d")
        && t.domain_not_null == Some(true)
        && t.type_base_oid.is_some_and(|oid| oid > 0)));
    assert!(
        positive
            .iter()
            .any(|t| t.domain_constraint_name.as_deref() == Some("pending")
                && t.domain_constraint_validated == Some(false))
    );
    assert!(
        types
            .iter()
            .any(|t| t.type_name.as_deref() == Some("composite")
                && t.type_relation_oid.is_some_and(|oid| oid > 0))
    );
    assert!(types.iter().any(|t| t.type_element_oid == Some(state)));
    let other_types = catalog_graph::inspect_types(client, "t11_graph_other")
        .await
        .unwrap();
    assert!(
        other_types
            .iter()
            .any(|t| t.type_name.as_deref() == Some("State")
                && t.type_oid == Some(other_state)
                && t.enum_label.as_deref() == Some("other"))
    );

    let malformed = GRAPH.replace("AS node_catalog,", "AS missing_catalog,");
    let rows = client
        .query(&malformed, &[&vec![root], &Vec::<u32>::new()])
        .await
        .unwrap();
    assert!(catalog_graph::decode_dependency_graph(&rows).is_err());
    let malformed = GRAPH.replace("SELECT h.database_oid,", "SELECT 0::oid AS database_oid,");
    let rows = client
        .query(&malformed, &[&vec![root], &Vec::<u32>::new()])
        .await
        .unwrap();
    assert!(catalog_graph::decode_dependency_graph(&rows).is_err());
    let malformed = TYPES.replace("n.oid AS namespace_oid", "0::oid AS namespace_oid");
    let rows = client.query(&malformed, &[&"t11_graph"]).await.unwrap();
    assert!(catalog_graph::decode_type_part(&rows[0]).is_err());
    client.batch_execute("ROLLBACK").await.unwrap();
    let state_after = client
        .query_one(
            "SELECT last_value,is_called FROM t11_graph.root_id_seq",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        (state_after.get::<_, i64>(0), state_after.get::<_, bool>(1)),
        state_before
    );
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM t11_graph.root", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );

    client
        .batch_execute("SET ROLE t11_graph_reader")
        .await
        .unwrap();
    let denied = catalog_graph::inspect_types(client, "t11_graph")
        .await
        .unwrap();
    let visible_graph = catalog_graph::inspect_dependency_graph(client, &[root], &[state])
        .await
        .unwrap();
    assert!(!visible_graph.parts.is_empty());
    assert!(
        visible_graph
            .parts
            .iter()
            .filter_map(|p| p.edge.as_ref())
            .any(|e| e.dependency.referenced_name.as_deref() == Some("t11_graph_reader"))
    );
    assert!(!denied.is_empty());
    assert!(
        denied
            .iter()
            .all(|p| !p.namespace_can_use && !p.namespace_can_create)
    );
    assert!(
        denied
            .iter()
            .filter(|p| p.type_oid == Some(state))
            .all(|p| p.can_use == Some(false) && p.can_alter == Some(false))
    );
    let permission = client
        .query("SELECT * FROM t11_graph.root", &[])
        .await
        .unwrap_err();
    assert_eq!(
        permission.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    client.batch_execute("RESET ROLE").await.unwrap();
    // Scoped explicit cleanup; disposal also removes this fixture if any assertion fails.
    client
        .batch_execute(
            r#"
        DROP EVENT TRIGGER t11_graph_ddl;
        ALTER EXTENSION plpgsql DROP TYPE t11_graph.unused_enum;
        DROP OWNED BY t11_graph_reader;
        DROP ROLE t11_graph_reader;
    "#,
        )
        .await
        .unwrap();
}
