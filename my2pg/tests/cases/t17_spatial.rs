//! Spatial migration preserves per-value SRIDs through EWKT and requires PostGIS to be preinstalled.
use my2pg::{config::MigrationConfig, mysql, postgres};
use mysql_async::prelude::Queryable;
use std::{env, fs, path::Path};
use tokio::process::Command;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing native fixture {name}"))
}

fn config(source_table: &str, target_schema: &str, report_dir: &Path) -> MigrationConfig {
    serde_json::from_value(serde_json::json!({
        "version": 1,
        "source": {
            "url_env": "MY2PG_MYSQL_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "consistency": "frozen"
        },
        "target": {
            "url_env": "MY2PG_POSTGRES_URL",
            "ca_file": required("MY2PG_TLS_CA"),
            "schema": target_schema
        },
        "migration": {
            "mode": "full",
            "reset_sequences": false,
            "batch_rows": 2,
            "batch_bytes": 4096,
            "max_row_bytes": 2048,
            "memory_bytes": 262144
        },
        "tables": {
            "include": [source_table],
            "rename": [{"source": source_table, "target": "shapes"}]
        },
        "report": {
            "directory": report_dir,
            "console": "json",
            "progress": "never"
        }
    }))
    .unwrap()
}

async fn run(config: &MigrationConfig, directory: &Path) -> std::process::Output {
    fs::create_dir_all(directory).unwrap();
    let path = directory.join("migration.toml");
    fs::write(&path, toml::to_string(config).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_my2pg"))
        .arg("run")
        .arg(path)
        .args(["--output", "json", "--progress", "never"])
        .output()
        .await
        .unwrap()
}

async fn extension_exists(client: &tokio_postgres::Client) -> bool {
    client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname='postgis')",
            &[],
        )
        .await
        .unwrap()
        .get(0)
}

async fn schema_exists(client: &tokio_postgres::Client, schema: &str) -> bool {
    client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname=$1)",
            &[&schema],
        )
        .await
        .unwrap()
        .get(0)
}

async fn setup_source(table: &str) -> mysql::SourceConnection {
    let source_config: my2pg::config::SourceConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_MYSQL_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "consistency": "frozen"
    }))
    .unwrap();
    let mut source = mysql::connect(&source_config, &required("MY2PG_MYSQL_ROOT_URL"), 4096)
        .await
        .unwrap();
    let identifier = mysql::quote_ident(table);
    source
        .query_drop(format!("DROP TABLE IF EXISTS {identifier}"))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "CREATE TABLE {identifier}(id INT NOT NULL PRIMARY KEY, shape GEOMETRY NULL) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
    source
        .query_drop(format!(
            "INSERT INTO {identifier}(id,shape) VALUES \
             (1,ST_GeomFromText('POINT(1 2)',4326)), \
             (2,ST_GeomFromText('LINESTRING(1 2,3 4)',3857)), \
             (3,ST_GeomFromText('GEOMETRYCOLLECTION EMPTY',0)), \
             (4,ST_GeomFromText('MULTIPOLYGON(((0 0,1 0,1 1,0 1,0 0)))',0)), \
             (5,NULL), \
             (6,ST_GeomFromText('MULTIPOINT((10 20),(30 40))',4326)), \
             (7,ST_GeomFromText('GEOMETRYCOLLECTION(POINT(7 8))',4326))"
        ))
        .await
        .unwrap();
    source
}

#[tokio::test]
#[ignore = "requires owned MySQL and plain PostgreSQL TLS fixtures without PostGIS"]
async fn native_spatial_requires_preinstalled_postgis_and_preserves_nothing_without_it() {
    let suffix = std::process::id();
    let source_table = format!("T17SpatialMissing{suffix}");
    let target_schema = format!("t17_spatial_missing_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t17-spatial-missing-{suffix}"));
    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "schema": target_schema
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    if extension_exists(&target.client).await {
        return;
    }
    let mut source = setup_source(&source_table).await;
    assert!(!schema_exists(&target.client, &target_schema).await);
    let output = run(
        &config(&source_table, &target_schema, &directory.join("runs")),
        &directory,
    )
    .await;
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "missing PostGIS unexpectedly succeeded"
    );
    assert!(diagnostics.contains("POSTGIS_REQUIRED"), "{diagnostics}");
    assert!(!schema_exists(&target.client, &target_schema).await);
    assert!(
        !extension_exists(&target.client).await,
        "my2pg must not install PostGIS"
    );
    source
        .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&source_table)))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires owned MySQL and PostGIS-enabled PostgreSQL TLS fixtures"]
async fn native_spatial_values_preserve_shape_empty_null_and_srid_with_postgis() {
    let suffix = std::process::id();
    let source_table = format!("T17SpatialPostgis{suffix}");
    let target_schema = format!("t17_spatial_postgis_{suffix}");
    let directory = env::temp_dir().join(format!("my2pg-t17-spatial-postgis-{suffix}"));
    let target_config: my2pg::config::TargetConfig = serde_json::from_value(serde_json::json!({
        "url_env": "MY2PG_POSTGRES_URL",
        "ca_file": required("MY2PG_TLS_CA"),
        "schema": target_schema
    }))
    .unwrap();
    let target = postgres::connect(&target_config, &required("MY2PG_POSTGRES_URL"))
        .await
        .unwrap();
    if !extension_exists(&target.client).await {
        return;
    }
    let mut source = setup_source(&source_table).await;
    let expected: Vec<(i32, Option<String>, Option<String>)> = source
        .query::<(i32, Option<String>, Option<String>), _>(format!(
            "SELECT id,CONCAT('SRID=',ST_SRID(shape),';',ST_AsText(shape)),ST_GeometryType(shape) FROM {} ORDER BY id",
            mysql::quote_ident(&source_table)
        ))
        .await
        .unwrap();
    assert_eq!(expected.len(), 7);
    assert!(expected.iter().any(|(_, value, _)| value.is_none()));
    let output = run(
        &config(&source_table, &target_schema, &directory.join("runs")),
        &directory,
    )
    .await;
    assert!(
        output.status.success(),
        "spatial migration failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let relation = postgres::qualified(&target_schema, "shapes");
    let observed = target
        .client
        .query(
            &format!(
                "SELECT id, ST_AsEWKT(shape), ST_GeometryType(shape), ST_NDims(shape), ST_SRID(shape) FROM {relation} ORDER BY id"
            ),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(observed.len(), expected.len());
    for (row, (expected_id, expected_ewkt, expected_type)) in observed.iter().zip(expected) {
        let id: i32 = row.get(0);
        let ewkt: Option<String> = row.get(1);
        let shape_type: Option<String> = row.get(2);
        let dimensions: Option<i16> = row.get(3);
        let srid: Option<i32> = row.get(4);
        assert_eq!(id, expected_id);
        if id == 5 {
            assert!(shape_type.is_none() && dimensions.is_none() && srid.is_none());
            assert!(expected_ewkt.is_none() && expected_type.is_none());
        } else {
            let expected_ewkt = expected_ewkt.unwrap();
            let expected_type = expected_type.unwrap();
            assert!(ewkt.is_some(), "row {id} target EWKT serialization");
            let expected_srid = expected_ewkt
                .strip_prefix("SRID=")
                .and_then(|text| text.split_once(';'))
                .unwrap()
                .0
                .parse::<i32>()
                .unwrap();
            let expected_type = if expected_type.eq_ignore_ascii_case("GEOMCOLLECTION") {
                "GEOMETRYCOLLECTION"
            } else {
                expected_type.as_str()
            };
            assert_eq!(srid, Some(expected_srid), "row {id} SRID");
            assert_eq!(
                shape_type.as_deref().map(str::to_ascii_uppercase),
                Some(format!("ST_{expected_type}").to_ascii_uppercase()),
                "row {id} geometry type"
            );
            assert_eq!(dimensions, Some(2_i16), "row {id} dimensions");
            let equivalent: bool = target
                .client
                .query_one(
                    &format!(
                        "SELECT ST_Equals(shape,ST_GeomFromEWKT($1)) FROM {relation} WHERE id=$2"
                    ),
                    &[&expected_ewkt, &id],
                )
                .await
                .unwrap()
                .get(0);
            assert!(equivalent, "row {id} coordinates differ");
        }
    }
    assert!(
        extension_exists(&target.client).await,
        "my2pg removed PostGIS"
    );
    source
        .query_drop(format!("DROP TABLE {}", mysql::quote_ident(&source_table)))
        .await
        .unwrap();
    target
        .client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            my2pg::plan::quote_identifier(&target_schema)
        ))
        .await
        .unwrap();
}
