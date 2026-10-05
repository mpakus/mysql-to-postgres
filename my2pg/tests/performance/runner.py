#!/usr/bin/env python3
"""Correctness-gated repeated MySQL-to-PostgreSQL benchmark runner.

Run against a ready, uniquely owned fixture from tests/support/harness.py.
The runner owns only its source table, target schemas, and artifact directory;
it never starts or stops the shared database project.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import re
import shutil
import subprocess
import statistics
import sys
import tempfile
import threading
import time
import uuid
from urllib.parse import urlsplit, urlunsplit

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests" / "support"))
import harness  # noqa: E402

HERE = Path(__file__).resolve().parent
CORPUS = json.loads((HERE / "corpus.json").read_text())
REDACT = re.compile(r"(mysql|postgres(?:ql)?)://[^\s'\"]+", re.IGNORECASE)
REQUIRED_BENCHMARK_PHASES = frozenset({"catalog", "copy", "index_constraints", "verification"})
PHASE_OPERATION_CLASSES = {
    "catalog": frozenset({"source_mysql_catalog", "target_postgresql_schema_table_prepare"}),
    "copy": frozenset({"row_copy"}),
    "index_constraints": frozenset({"primary_keys", "indexes", "foreign_keys", "check_constraints"}),
    "verification": frozenset({"mysql_source_oracle", "postgresql_target_oracle"}),
}


def workload_tables(workload):
    spec = CORPUS["workloads"][workload]
    return spec["tables"] if "tables" in spec else [spec["table"]]


def workload_profile(workload, profile):
    return CORPUS["workloads"][workload].get("profiles", CORPUS["profiles"])[profile]


def workload_row_count(workload, rows):
    if workload == "many_small_relational_tables":
        return ((rows + len(workload_tables(workload)) - 1) // len(workload_tables(workload))) * len(workload_tables(workload))
    return rows * len(workload_tables(workload))


def workload_good_row_count(workload, rows, bad_rate="1"):
    if workload == "bad_rows_fixed_rate":
        return rows - len(expected_bad_row_ids(rows, bad_rate))
    return workload_row_count(workload, rows)


def workload_table_values(workload):
    tables = workload_tables(workload)
    return {
        "table": tables[0],
        "include_tables": json.dumps(tables),
        "pgloader_table_pattern": ", ".join(f"~/^{re.escape(table)}$/" for table in tables),
    }


def bad_rate_denominator(bad_rate):
    denominators = {"0.1": 1000, "1": 100, "5": 20}
    try:
        return denominators[str(bad_rate)]
    except KeyError as error:
        raise ValueError("bad-row rate must be 0.1, 1, or 5 percent") from error


def expected_bad_row_ids(rows, bad_rate):
    return list(range(bad_rate_denominator(bad_rate), rows + 1,
                      bad_rate_denominator(bad_rate)))


def validate_bad_row_schedule(actual_ids, rows, bad_rate):
    expected_ids = expected_bad_row_ids(rows, bad_rate)
    if actual_ids != expected_ids:
        raise ValueError("MySQL rejected source IDs differ from the deterministic bad-row schedule")
    return expected_ids


def expected_foreign_keys(workload):
    if workload != "many_small_relational_tables":
        return ""
    tables = workload_tables(workload)
    return ";".join(f"{tables[index]}:parent_id->{tables[index - 1]}:id" for index in range(1, len(tables)))


def source_seed_sql(rows, seed, workload="one_large_narrow_integer", bad_rate="1"):
    if rows < 1 or rows > 100_000_000:
        raise ValueError("row count must be between 1 and 100000000")
    if workload not in CORPUS["workloads"]:
        raise ValueError(f"unknown corpus workload: {workload}")
    digits = 3 if rows <= 1_000 else 5 if rows <= 100_000 else 6 if rows <= 1_000_000 else 7 if rows <= 10_000_000 else 8
    aliases = [f"d{index}" for index in range(digits)]
    joins = " CROSS JOIN ".join("(SELECT digit FROM source.t21_digits) AS " + alias for alias in aliases)
    number = " + ".join(f"{alias}.digit * {10 ** index}" for index, alias in enumerate(aliases))
    if workload == "several_large_tables":
        statements = [f"DROP TABLE IF EXISTS source.{name};" for name in workload_tables(workload)]
        statements += [
            "DROP TABLE IF EXISTS source.t21_digits;",
            "CREATE TABLE source.t21_digits (digit TINYINT NOT NULL PRIMARY KEY) ENGINE=InnoDB;",
            "INSERT INTO source.t21_digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);",
        ]
        for index, name in enumerate(workload_tables(workload)):
            statements.extend([
                f"CREATE TABLE source.{name} (id BIGINT NOT NULL PRIMARY KEY, bucket INTEGER NOT NULL, "
                "measure BIGINT NOT NULL, payload BIGINT NOT NULL) ENGINE=InnoDB;",
                f"INSERT INTO source.{name} (id,bucket,measure,payload) SELECT seqs.id, MOD(seqs.id,1024), "
                f"MOD(seqs.id * 48271 + {seed + index},2147483647), "
                f"MOD(seqs.id * 69621 + {seed + index * 17} * 17,2147483629) "
                f"FROM (SELECT ({number}) + 1 AS id FROM {joins}) AS seqs WHERE seqs.id <= {rows} "
                "ORDER BY seqs.id;",
            ])
        statements.append("DROP TABLE source.t21_digits;")
        return "\n".join(statements)
    if workload == "wide_utf8_text_json_decimal":
        return f"""DROP TABLE IF EXISTS source.t21_wide_rows;
DROP TABLE IF EXISTS source.t21_digits;
SET NAMES utf8mb4;
CREATE TABLE source.t21_digits (digit TINYINT NOT NULL PRIMARY KEY) ENGINE=InnoDB;
INSERT INTO source.t21_digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);
CREATE TABLE source.t21_wide_rows (
    id BIGINT NOT NULL PRIMARY KEY,
    label VARCHAR(24) CHARACTER SET utf8mb4 NOT NULL,
    detail TEXT CHARACTER SET utf8mb4 NOT NULL,
    json_doc JSON NOT NULL,
    amount DECIMAL(30,12) NOT NULL
) ENGINE=InnoDB;
INSERT INTO source.t21_wide_rows (id,label,detail,json_doc,amount)
SELECT seqs.id, CONCAT('雪🧪-', LPAD(seqs.id, 8, '0')),
       REPEAT(CONCAT('mañana-é-雪-', LPAD(seqs.id, 8, '0'), '|'), 64),
       JSON_OBJECT('value', seqs.id, 'tag', '雪'),
       CAST(seqs.id AS DECIMAL(30,12)) / 1000
FROM (
    SELECT ({number}) + 1 AS id FROM {joins}
) AS seqs
WHERE seqs.id <= {rows}
ORDER BY seqs.id;
DROP TABLE source.t21_digits;"""
    if workload == "binary_large_rows":
        return f"""DROP TABLE IF EXISTS source.t21_binary_rows;
DROP TABLE IF EXISTS source.t21_digits;
CREATE TABLE source.t21_digits (digit TINYINT NOT NULL PRIMARY KEY) ENGINE=InnoDB;
INSERT INTO source.t21_digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);
CREATE TABLE source.t21_binary_rows (
    id BIGINT NOT NULL PRIMARY KEY,
    payload LONGBLOB NOT NULL,
    optional_bytes VARBINARY(16) NULL
) ENGINE=InnoDB;
INSERT INTO source.t21_binary_rows (id,payload,optional_bytes)
SELECT seqs.id,
       REPEAT(CONCAT(UNHEX(LPAD(HEX(MOD(seqs.id,256)),2,'0')),UNHEX('00ff5c80')),16384),
       CASE MOD(seqs.id,5) WHEN 0 THEN NULL WHEN 1 THEN UNHEX('') ELSE UNHEX(LPAD(HEX(seqs.id),32,'0')) END
FROM (
    SELECT ({number}) + 1 AS id FROM {joins}
) AS seqs
WHERE seqs.id <= {rows}
ORDER BY seqs.id;
DROP TABLE source.t21_digits;"""
    if workload == "bad_rows_fixed_rate":
        denominator = bad_rate_denominator(bad_rate)
        return f"""DROP TABLE IF EXISTS source.t21_bad_rows;
DROP TABLE IF EXISTS source.t21_digits;
SET NAMES utf8mb4;
CREATE TABLE source.t21_digits (digit TINYINT NOT NULL PRIMARY KEY) ENGINE=InnoDB;
INSERT INTO source.t21_digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);
CREATE TABLE source.t21_bad_rows (
    id BIGINT NOT NULL PRIMARY KEY,
    payload TEXT CHARACTER SET utf8mb4 NOT NULL
) ENGINE=InnoDB;
INSERT INTO source.t21_bad_rows (id,payload)
SELECT seqs.id,
       IF(MOD(seqs.id,{denominator})=0,
          CONCAT('bad-',CAST(seqs.id AS CHAR),CHAR(0),'-tail'),
          CONCAT('good-',CAST(seqs.id AS CHAR)))
FROM (
    SELECT ({number}) + 1 AS id FROM {joins}
) AS seqs
WHERE seqs.id <= {rows}
ORDER BY seqs.id;
DROP TABLE source.t21_digits;"""
    if workload == "many_small_relational_tables":
        per_table_rows = (rows + len(workload_tables(workload)) - 1) // len(workload_tables(workload))
        digits = 3 if per_table_rows <= 1_000 else 5 if per_table_rows <= 100_000 else 6 if per_table_rows <= 1_000_000 else 7 if per_table_rows <= 10_000_000 else 8
        aliases = [f"d{index}" for index in range(digits)]
        joins = " CROSS JOIN ".join("(SELECT digit FROM source.t21_digits) AS " + alias for alias in aliases)
        number = " + ".join(f"{alias}.digit * {10 ** index}" for index, alias in enumerate(aliases))
        statements = [f"DROP TABLE IF EXISTS source.{name};" for name in reversed(workload_tables(workload))]
        statements += [
            "DROP TABLE IF EXISTS source.t21_digits;",
            "CREATE TABLE source.t21_digits (digit TINYINT NOT NULL PRIMARY KEY) ENGINE=InnoDB;",
            "INSERT INTO source.t21_digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);",
        ]
        for index, name in enumerate(workload_tables(workload)):
            foreign_key = (f", CONSTRAINT fk_t21_rel_{index:02d} FOREIGN KEY (parent_id) "
                           f"REFERENCES source.{workload_tables(workload)[index - 1]} (id)") if index else ""
            statements.extend([
                f"CREATE TABLE source.{name} (id INTEGER NOT NULL PRIMARY KEY, parent_id INTEGER NULL, "
                f"bucket INTEGER NOT NULL, measure BIGINT NOT NULL, payload BIGINT NOT NULL{foreign_key}) ENGINE=InnoDB;",
                f"INSERT INTO source.{name} (id,parent_id,bucket,measure,payload) SELECT seqs.id, "
                f"IF(seqs.id=1,NULL,seqs.id-1), MOD(seqs.id,1024), "
                f"MOD(seqs.id * 48271 + {seed + index},2147483647), "
                f"MOD(seqs.id * 69621 + {seed + index * 17} * 17,2147483629) "
                f"FROM (SELECT ({number}) + 1 AS id FROM {joins}) AS seqs WHERE seqs.id <= {per_table_rows} "
                "ORDER BY seqs.id;",
            ])
        statements.append("DROP TABLE source.t21_digits;")
        return "\n".join(statements)
    table = CORPUS["workloads"][workload]["table"]
    if workload == "sparse_skewed_unsigned_primary_keys":
        id_expr = "CAST(seqs.id AS UNSIGNED) * 100000"
        bucket_expr = "IF(MOD(seqs.id, 10) < 9, 0, MOD(seqs.id, 16) + 1)"
        id_type = "BIGINT UNSIGNED"
    else:
        id_expr = "seqs.id"
        bucket_expr = "MOD(seqs.id, 1024)"
        id_type = "BIGINT"
    return f"""DROP TABLE IF EXISTS source.{table};
DROP TABLE IF EXISTS source.t21_digits;
CREATE TABLE source.t21_digits (digit TINYINT NOT NULL PRIMARY KEY) ENGINE=InnoDB;
INSERT INTO source.t21_digits VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9);
CREATE TABLE source.{table} (
    id {id_type} NOT NULL PRIMARY KEY,
    bucket INTEGER NOT NULL,
    measure BIGINT NOT NULL,
    payload BIGINT NOT NULL
) ENGINE=InnoDB;
INSERT INTO source.{table} (id, bucket, measure, payload)
SELECT {id_expr}, {bucket_expr}, MOD(seqs.id * 48271 + {seed}, 2147483647),
       MOD(seqs.id * 69621 + {seed} * 17, 2147483629)
FROM (
    SELECT ({number}) + 1 AS id FROM {joins}
) AS seqs
WHERE seqs.id <= {rows}
ORDER BY seqs.id;
DROP TABLE source.t21_digits;"""


def source_canonical_query(workload="one_large_narrow_integer", bad_rate="1"):
    tables = workload_tables(workload)
    if workload == "wide_utf8_text_json_decimal":
        return "SELECT CONCAT(CAST(id AS BINARY), _binary'|', CAST(LOWER(HEX(label)) AS BINARY), _binary'|', " \
            "CAST(LOWER(HEX(detail)) AS BINARY), _binary'|', " \
            "CAST(JSON_UNQUOTE(JSON_EXTRACT(json_doc, '$.value')) AS BINARY), _binary'|', " \
            "CAST(LOWER(HEX(JSON_UNQUOTE(JSON_EXTRACT(json_doc, '$.tag')))) AS BINARY), _binary'|', " \
            "CAST(amount AS BINARY)) " \
            "FROM `source`.`t21_wide_rows` ORDER BY id"
    if workload == "binary_large_rows":
        return "SELECT CONCAT(CAST(id AS CHAR), '|', OCTET_LENGTH(payload), '|', MD5(payload), '|', " \
            "IF(optional_bytes IS NULL, 'NULL', CONCAT('BYTES:', LOWER(HEX(optional_bytes))))) " \
            "FROM `source`.`t21_binary_rows` ORDER BY id"
    if workload == "bad_rows_fixed_rate":
        denominator = bad_rate_denominator(bad_rate)
        return f"SELECT CONCAT(CAST(id AS CHAR), '|', HEX(payload)) " \
            f"FROM `source`.`t21_bad_rows` WHERE MOD(id,{denominator})<>0 ORDER BY id"
    if workload == "many_small_relational_tables":
        selects = [
            f"SELECT CONCAT('{table}|', CAST(id AS CHAR), '|', COALESCE(CAST(parent_id AS CHAR), 'NULL'), '|', "
            f"CAST(bucket AS CHAR), '|', CAST(measure AS CHAR), '|', CAST(payload AS CHAR)) "
            f"AS canonical_row FROM `source`.`{table}`"
            for table in tables
        ]
        return "SELECT canonical_row FROM (" + " UNION ALL ".join(selects) + ") AS all_rows ORDER BY canonical_row"
    if len(tables) > 1:
        selects = [
            f"SELECT CONCAT('{table}|', CAST(id AS CHAR), '|', CAST(bucket AS CHAR), '|', "
            f"CAST(measure AS CHAR), '|', CAST(payload AS CHAR)) AS canonical_row FROM `source`.`{table}`"
            for table in tables
        ]
        return "SELECT canonical_row FROM (" + " UNION ALL ".join(selects) + ") AS all_rows ORDER BY canonical_row"
    table = tables[0]
    return f"SELECT CONCAT(CAST(id AS CHAR), ',', CAST(bucket AS CHAR), ',', CAST(measure AS CHAR), ',', CAST(payload AS CHAR)) FROM `source`.`{table}` ORDER BY id"


def target_canonical_query(schema, workload="one_large_narrow_integer", bad_rate="1"):
    tables = workload_tables(workload)
    if workload == "wide_utf8_text_json_decimal":
        return f"SELECT id::text || '|' || encode(convert_to(label,'UTF8'),'hex') || '|' || " \
            f"encode(convert_to(detail,'UTF8'),'hex') || '|' || (json_doc->>'value') || '|' || " \
            f"encode(convert_to(json_doc->>'tag','UTF8'),'hex') || '|' || amount::text " \
            f"FROM {schema}.t21_wide_rows ORDER BY id"
    if workload == "binary_large_rows":
        return f"SELECT id::text || '|' || octet_length(payload)::text || '|' || md5(payload) || '|' || " \
            f"CASE WHEN optional_bytes IS NULL THEN 'NULL' ELSE 'BYTES:' || encode(optional_bytes,'hex') END " \
            f"FROM {schema}.t21_binary_rows ORDER BY id"
    if workload == "bad_rows_fixed_rate":
        return f"SELECT id::text || '|' || upper(encode(convert_to(payload,'UTF8'),'hex')) " \
            f"FROM {schema}.t21_bad_rows ORDER BY id"
    if workload == "many_small_relational_tables":
        selects = [
            f"SELECT '{table}|' || id::text || '|' || COALESCE(parent_id::text, 'NULL') || '|' || "
            f"bucket::text || '|' || measure::text || '|' || payload::text AS canonical_row FROM {schema}.{table}"
            for table in tables
        ]
        return "SELECT canonical_row FROM (" + " UNION ALL ".join(selects) + ") AS all_rows ORDER BY canonical_row"
    if len(tables) > 1:
        selects = [
            f"SELECT '{table}|' || id::text || '|' || bucket::text || '|' || measure::text || '|' || payload::text "
            f"AS canonical_row FROM {schema}.{table}"
            for table in tables
        ]
        return "SELECT canonical_row FROM (" + " UNION ALL ".join(selects) + ") AS all_rows ORDER BY canonical_row"
    table = tables[0]
    return (
        f"SELECT id::text || ',' || bucket::text || ',' || measure::text || ',' || payload::text "
        f"FROM {schema}.{table} ORDER BY id"
    )


def stream_digest(command, env=None):
    digest = hashlib.sha256()
    count = 0
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    assert process.stdout is not None
    for line in process.stdout:
        digest.update(line)
        count += 1
    stderr = process.stderr.read() if process.stderr else b""
    code = process.wait()
    if code:
        raise RuntimeError(f"oracle query failed ({code}): {redact(stderr.decode(errors='replace'))}")
    return {"rows": count, "sha256": digest.hexdigest()}


def redact(value, secrets=()):
    value = REDACT.sub("[redacted-url]", value)
    for secret in sorted((item for item in secrets if item), key=len, reverse=True):
        value = value.replace(secret, "[redacted]")
    return value


def command_output(args):
    try:
        return harness.command(args, check=False).stdout.strip() or None
    except OSError:
        return None


def source_oracle(metadata, workload="one_large_narrow_integer", bad_rate="1", rows=None):
    query = source_canonical_query(workload, bad_rate)
    tables = workload_tables(workload)
    id_type = "numeric" if workload == "sparse_skewed_unsigned_primary_keys" else "bigint"
    command = compose_argv(metadata, "exec", "-T", "mysql", "mysql", "-umy2pg",
                           "-pintegration-only", "--database=source", "--batch",
                           "--skip-column-names", "-e", query)
    result = stream_digest(command, compose_environment(metadata))
    aggregates = []
    columns = []
    primary_keys = []
    for table in tables:
        if workload == "bad_rows_fixed_rate":
            denominator = bad_rate_denominator(bad_rate)
            aggregate = harness.sql(metadata, "mysql", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', MIN(id), ',', MAX(id)) FROM source.{table} WHERE MOD(id,{denominator})<>0""")
            payload_bytes = harness.sql(metadata, "mysql", f"""SELECT SUM(OCTET_LENGTH(payload)) FROM source.{table} WHERE MOD(id,{denominator})<>0""")
            rejected = harness.command(compose_argv(metadata, "exec", "-T", "mysql", "mysql", "-umy2pg",
                "-pintegration-only", "--database=source", "--batch", "--skip-column-names", "-e",
                f"SELECT CAST(id AS CHAR), HEX(payload) FROM source.{table} WHERE LOCATE(CHAR(0),payload)>0 ORDER BY id"))
            if rejected.returncode:
                raise RuntimeError("MySQL rejected-row oracle query failed")
            rejected_rows = []
            for line in rejected.stdout.splitlines():
                if not line.strip():
                    continue
                fields = line.split("\t")
                if len(fields) != 2:
                    raise ValueError("MySQL rejected-row oracle returned an invalid row")
                rejected_rows.append({"id": int(fields[0]), "payload_hex": fields[1].lower()})
            result["rejected_rows"] = rejected_rows
            result["rejected_ids"] = [row["id"] for row in rejected_rows]
            if rows is not None:
                validate_bad_row_schedule(result["rejected_ids"], rows, bad_rate)
            columns.append(f"{table}:id:bigint:NO,payload:text:NO")
        elif workload == "wide_utf8_text_json_decimal":
            aggregate = harness.sql(metadata, "mysql", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', SUM(amount), ',', MIN(id), ',', MAX(id)) FROM source.{table}""")
            payload_bytes = harness.sql(metadata, "mysql", f"""SELECT SUM(OCTET_LENGTH(CAST(id AS CHAR)) + OCTET_LENGTH(label) + OCTET_LENGTH(detail) + OCTET_LENGTH(JSON_UNQUOTE(JSON_EXTRACT(json_doc, '$.value'))) + OCTET_LENGTH(JSON_UNQUOTE(JSON_EXTRACT(json_doc, '$.tag'))) + OCTET_LENGTH(CAST(amount AS CHAR))) FROM source.{table}""")
            columns.append(f"{table}:id:bigint:NO,label:character varying:NO,detail:text:NO,json_doc:jsonb:NO,amount:numeric:NO")
        elif workload == "binary_large_rows":
            aggregate = harness.sql(metadata, "mysql", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', MIN(id), ',', MAX(id)) FROM source.{table}""")
            payload_bytes = harness.sql(metadata, "mysql", f"""SELECT SUM(OCTET_LENGTH(payload) + COALESCE(OCTET_LENGTH(optional_bytes),0)) FROM source.{table}""")
            columns.append(f"{table}:id:bigint:NO,payload:bytea:NO,optional_bytes:bytea:YES")
        else:
            aggregate = harness.sql(metadata, "mysql", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', SUM(bucket), ',', SUM(measure), ',', SUM(payload), ',', MIN(id), ',', MAX(id)) FROM source.{table}""")
            if workload == "many_small_relational_tables":
                columns.append(f"{table}:id:integer:NO,parent_id:integer:YES,bucket:integer:NO,measure:bigint:NO,payload:bigint:NO")
            else:
                columns.append(f"{table}:id:{id_type}:NO,bucket:integer:NO,measure:bigint:NO,payload:bigint:NO")
        aggregates.append(f"{table}:{aggregate}")
        primary_keys.append(f"{table}:id")
    result["aggregate"] = ";".join(aggregates) if len(tables) > 1 else aggregates[0].split(":", 1)[1]
    result["target_columns"] = ";".join(columns) if len(tables) > 1 else columns[0].split(":", 1)[1]
    result["primary_key"] = ";".join(primary_keys) if len(tables) > 1 else "id"
    result["table_count"] = len(tables)
    result["foreign_key_count"] = CORPUS["workloads"][workload].get("foreign_key_count", 0)
    result["foreign_keys"] = expected_foreign_keys(workload)
    if workload in {"wide_utf8_text_json_decimal", "binary_large_rows", "bad_rows_fixed_rate"}:
        result["payload_bytes"] = int(payload_bytes)
    return result


def target_oracle(metadata, schema, workload="one_large_narrow_integer", bad_rate="1"):
    tables = workload_tables(workload)
    query = target_canonical_query(schema, workload, bad_rate)
    command = compose_argv(metadata, "exec", "-T", "postgres", "psql", "-U", "my2pg", "-d", "target",
                           "-At", "-c", query)
    result = stream_digest(command, compose_environment(metadata))
    aggregates = []
    columns = []
    primary_keys = []
    for table in tables:
        if workload == "bad_rows_fixed_rate":
            aggregate = harness.sql(metadata, "postgres", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', MIN(id), ',', MAX(id)) FROM {schema}.{table}""")
            payload_bytes = harness.sql(metadata, "postgres", f"""SELECT SUM(OCTET_LENGTH(payload)) FROM {schema}.{table}""")
        elif workload == "wide_utf8_text_json_decimal":
            aggregate = harness.sql(metadata, "postgres", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', SUM(amount), ',', MIN(id), ',', MAX(id)) FROM {schema}.{table}""")
            payload_bytes = harness.sql(metadata, "postgres", f"""SELECT SUM(OCTET_LENGTH(id::text) + OCTET_LENGTH(label) + OCTET_LENGTH(detail) + OCTET_LENGTH(json_doc->>'value') + OCTET_LENGTH(json_doc->>'tag') + OCTET_LENGTH(amount::text)) FROM {schema}.{table}""")
        elif workload == "binary_large_rows":
            aggregate = harness.sql(metadata, "postgres", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', MIN(id), ',', MAX(id)) FROM {schema}.{table}""")
            payload_bytes = harness.sql(metadata, "postgres", f"""SELECT SUM(OCTET_LENGTH(payload) + COALESCE(OCTET_LENGTH(optional_bytes),0)) FROM {schema}.{table}""")
        else:
            aggregate = harness.sql(metadata, "postgres", f"""SELECT CONCAT(COUNT(*), ',', SUM(id), ',', SUM(bucket), ',', SUM(measure), ',', SUM(payload), ',', MIN(id), ',', MAX(id)) FROM {schema}.{table}""")
        target_columns = harness.sql(metadata, "postgres", f"""SELECT STRING_AGG(column_name || ':' || data_type || ':' || is_nullable, ',' ORDER BY ordinal_position)
FROM information_schema.columns WHERE table_schema='{schema}' AND table_name='{table}'""")
        primary_key = harness.sql(metadata, "postgres", f"""SELECT STRING_AGG(a.attname, ',' ORDER BY key.ordinality)
FROM pg_constraint c CROSS JOIN LATERAL UNNEST(c.conkey) WITH ORDINALITY AS key(attnum, ordinality)
JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=key.attnum
WHERE c.conrelid='{schema}.{table}'::regclass AND c.contype='p'""")
        aggregates.append(f"{table}:{aggregate}")
        columns.append(f"{table}:{target_columns}")
        primary_keys.append(f"{table}:{primary_key}")
    result["aggregate"] = ";".join(aggregates) if len(tables) > 1 else aggregates[0].split(":", 1)[1]
    result["target_columns"] = ";".join(columns) if len(tables) > 1 else columns[0].split(":", 1)[1]
    result["primary_key"] = ";".join(primary_keys) if len(tables) > 1 else "id"
    result["table_count"] = int(harness.sql(metadata, "postgres", f"SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='{schema}' AND table_type='BASE TABLE'"))
    if workload in {"wide_utf8_text_json_decimal", "binary_large_rows", "bad_rows_fixed_rate"}:
        result["payload_bytes"] = int(payload_bytes)
    result["foreign_key_count"] = int(harness.sql(metadata, "postgres", f"SELECT COUNT(*) FROM pg_constraint c JOIN pg_class r ON r.oid=c.conrelid JOIN pg_namespace n ON n.oid=r.relnamespace WHERE n.nspname='{schema}' AND c.contype='f'"))
    result["foreign_keys"] = harness.sql(metadata, "postgres", f"""SELECT COALESCE(STRING_AGG(src.relname || ':' || src_cols.names || '->' || dst.relname || ':' || dst_cols.names, ';' ORDER BY src.relname), '')
FROM pg_constraint fk
JOIN pg_class src ON src.oid=fk.conrelid
JOIN pg_namespace n ON n.oid=src.relnamespace
JOIN pg_class dst ON dst.oid=fk.confrelid
CROSS JOIN LATERAL (SELECT STRING_AGG(a.attname, ',' ORDER BY key.ordinality) AS names
    FROM UNNEST(fk.conkey) WITH ORDINALITY AS key(attnum, ordinality)
    JOIN pg_attribute a ON a.attrelid=src.oid AND a.attnum=key.attnum) AS src_cols
CROSS JOIN LATERAL (SELECT STRING_AGG(a.attname, ',' ORDER BY key.ordinality) AS names
    FROM UNNEST(fk.confkey) WITH ORDINALITY AS key(attnum, ordinality)
    JOIN pg_attribute a ON a.attrelid=dst.oid AND a.attnum=key.attnum) AS dst_cols
WHERE n.nspname='{schema}' AND fk.contype='f'""")
    return result


def resource_pins(metadata):
    ids = harness.validate_owned(metadata)
    details = []
    for container in ids:
        item = json.loads(harness.command(["docker", "inspect", container]).stdout)[0]
        host = item["HostConfig"]
        config = item["Config"]
        details.append({
            "container_name": item.get("Name", "").lstrip("/"),
            "service": config.get("Labels", {}).get("com.docker.compose.service"),
            "image_id": item.get("Image"),
            "memory_bytes": host.get("Memory", 0),
            "nano_cpus": host.get("NanoCpus", 0),
            "cpu_quota": host.get("CpuQuota", 0),
            "cpu_period": host.get("CpuPeriod", 0),
        })
    limits_declared = bool(details) and all(
        item["memory_bytes"] > 0 and (item["nano_cpus"] > 0 or item["cpu_quota"] > 0)
        for item in details
    )
    limits_equal = limits_declared and len({(item["memory_bytes"], item["nano_cpus"],
                                             item["cpu_quota"], item["cpu_period"]) for item in details}) == 1
    return {"containers": details, "limits_declared": limits_declared, "limits_equal": limits_equal}


def make_config(template, values, output):
    text = template.format_map(values)
    output.write_text(text)
    output.chmod(0o600)


def phase_metrics(log_text, report_file, loader_kind="command", verification_elapsed_millis=None,
                  pgloader_summary_file=None):
    intervals = []
    if loader_kind == "pgloader-v4":
        try:
            summary = json.loads(Path(pgloader_summary_file).read_text())
        except (TypeError, OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            return {"status": "unsupported", "phase_intervals": [], "phase_seconds": {},
                    "phase_operation_classes": {},
                    "reason": f"pgloader phase summary could not be read: {error}"}
        intervals = summary.get("benchmark_phases", []) if isinstance(summary, dict) else []
        if not isinstance(intervals, list):
            return {"status": "unsupported", "phase_intervals": [], "phase_seconds": {},
                    "phase_operation_classes": {},
                    "reason": "pgloader benchmark_phases is not an array"}
    else:
        for line in log_text.splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(event, dict):
                continue
            if event.get("kind") == "phase_interval":
                intervals.append(event)
    if (verification_elapsed_millis is not None and intervals
            and not any(event.get("phase") == "verification" for event in intervals)):
        latest_end = max(
            (event.get("end_elapsed_millis", -1) for event in intervals
             if type(event.get("end_elapsed_millis")) is int),
            default=-1,
        )
        if latest_end >= 0:
            intervals.append({
                "kind": "phase_interval", "phase": "verification",
                "clock": "run_monotonic_millis", "scope": "run",
                "start_elapsed_millis": latest_end,
                "end_elapsed_millis": latest_end + verification_elapsed_millis,
                "operation_classes": sorted(PHASE_OPERATION_CLASSES["verification"]),
                "normalization": "independent source and target oracle measured after loader; positioned after migration intervals",
            })
    metrics = {"status": "unsupported", "phase_intervals": intervals,
               "phase_seconds": {}, "phase_operation_classes": {}}
    if report_file.exists():
        try:
            report = json.loads(report_file.read_text())
            metrics["loader_elapsed_millis"] = report.get("elapsed_millis")
            metrics["copy_elapsed_millis_by_table"] = {
                table.get("source_name", table.get("id", "unknown")): table.get("copy_elapsed_millis")
                for table in report.get("tables", [])
            }
        except (json.JSONDecodeError, OSError):
            pass
    if not intervals:
        metrics["reason"] = "explicit run-wide phase intervals are missing"
        return metrics
    seen = set()
    for interval in intervals:
        phase = interval.get("phase")
        start_ms = interval.get("start_elapsed_millis")
        end_ms = interval.get("end_elapsed_millis")
        operations = interval.get("operation_classes")
        if phase not in REQUIRED_BENCHMARK_PHASES:
            metrics["reason"] = "phase interval has an unknown or malformed phase"
            return metrics
        if phase in seen:
            metrics["reason"] = f"duplicate {phase} phase interval"
            return metrics
        seen.add(phase)
        if interval.get("clock") != "run_monotonic_millis" or interval.get("scope") != "run":
            metrics["reason"] = f"{phase} phase interval does not use the run-wide monotonic clock and scope"
            return metrics
        if (type(start_ms) is not int or type(end_ms) is not int
                or start_ms < 0 or end_ms < start_ms):
            metrics["reason"] = f"{phase} phase interval has invalid or negative boundaries"
            return metrics
        if (not isinstance(operations, list) or not operations
                or any(not isinstance(item, str) or not item for item in operations)
                or len(operations) != len(set(operations))
                or frozenset(operations) != PHASE_OPERATION_CLASSES[phase]):
            metrics["reason"] = f"{phase} phase interval has missing or incompatible operation classes"
            return metrics
        metrics["phase_seconds"][phase] = (end_ms - start_ms) / 1000
        metrics["phase_operation_classes"][phase] = sorted(operations)
    if seen != REQUIRED_BENCHMARK_PHASES:
        missing = sorted(REQUIRED_BENCHMARK_PHASES - seen)
        metrics["reason"] = "canonical phase intervals are incomplete: " + ", ".join(missing)
        return metrics
    metrics["status"] = "supported"
    return metrics


def pgloader_summary_metrics(summary_file):
    """Read pgloader's source-native summary without treating it as normalized T21 phases."""
    metrics = {"status": "unsupported", "phase_totals_nanos": {}}
    try:
        summary = json.loads(Path(summary_file).read_text())
    except FileNotFoundError:
        metrics["reason"] = "pgloader summary file is missing"
        return metrics
    except (json.JSONDecodeError, OSError, UnicodeDecodeError) as error:
        metrics["reason"] = f"pgloader summary could not be read: {error}"
        return metrics
    if not isinstance(summary, dict):
        metrics["reason"] = "pgloader summary root is not an object"
        return metrics

    grand_total = summary.get("grand-total")
    phases = summary.get("phases")
    if not isinstance(grand_total, dict) or not isinstance(phases, dict):
        metrics["reason"] = "pgloader summary is missing grand-total or phases objects"
        return metrics
    required_totals = ("total-nanos", "rows", "bytes")
    if any(type(grand_total.get(key)) is not int or grand_total[key] < 0 for key in required_totals):
        metrics["reason"] = "pgloader summary grand-total values are missing, malformed, or negative"
        return metrics

    for phase in ("pre", "data", "post"):
        group = phases.get(phase)
        total = group.get("total") if isinstance(group, dict) else None
        nanos = total.get("total-nanos") if isinstance(total, dict) else None
        if type(nanos) is int and nanos >= 0:
            metrics["phase_totals_nanos"][phase] = nanos
    if set(metrics["phase_totals_nanos"]) != {"pre", "data", "post"}:
        metrics["reason"] = "pgloader summary phase totals are missing, malformed, or negative"
        return metrics

    post = phases.get("post")
    entries = post.get("tables") if isinstance(post, dict) else None
    copy_entries = [entry for entry in entries if isinstance(entry, dict)
                    and entry.get("label") == "COPY Wall-Clock Time"] if isinstance(entries, list) else []
    if len(copy_entries) != 1:
        metrics["reason"] = "pgloader summary must contain exactly one COPY Wall-Clock Time entry"
        return metrics
    copy_nanos = copy_entries[0].get("total-time")
    if type(copy_nanos) is not int or copy_nanos < 0:
        metrics["reason"] = "pgloader COPY wall time is malformed or negative"
        return metrics

    metrics.update({"status": "observed",
                    "loader_elapsed_nanos": grand_total["total-nanos"],
                    "rows": grand_total["rows"],
                    "bytes": grand_total["bytes"],
                    "copy_wall_nanos": copy_nanos})
    return metrics


def phases_are_comparable(runs):
    """Fail closed unless every run exposes the complete normalized phase set."""
    phase_scopes = []
    for run in runs:
        metrics = run.get("phase_metrics", {})
        phases = metrics.get("phase_seconds", {})
        operations = metrics.get("phase_operation_classes", {})
        if (metrics.get("status") != "supported" or not isinstance(phases, dict)
                or frozenset(phases) != REQUIRED_BENCHMARK_PHASES
                or not isinstance(operations, dict)
                or frozenset(operations) != REQUIRED_BENCHMARK_PHASES):
            return False
        phase_scopes.append(operations)
    return bool(phase_scopes) and all(scopes == phase_scopes[0] for scopes in phase_scopes[1:])


def report_path_for_run(run_dir):
    """Find My2pg's report in its generated per-run artifact directory."""
    candidates = list((Path(run_dir) / "report").rglob("report.json"))
    if len(candidates) > 1:
        raise ValueError("My2pg run produced multiple report.json artifacts")
    return candidates[0] if candidates else Path(run_dir) / "report" / "report.json"


def loader_invocation(loader, values, run_dir):
    fields = dict(values)
    fields["loader_name"] = loader["name"]
    fields["row_error_policy"] = loader.get("settings", {}).get("row_error_policy", "stop")
    argv = [str(part).format_map(fields) for part in loader["argv"]]
    if "config_template" in loader:
        path = run_dir / "migration.toml"
        make_config(loader["config_template"], fields, path)
        if fields["row_error_policy"] == "reject":
            config_text = path.read_text()
            config_text = config_text.replace('on_row_error = "stop"\nmax_rejected_rows = 0',
                                              'on_row_error = "reject"\nmax_rejected_rows = 12500')
            path.write_text(config_text)
            path.chmod(0o600)
        fields["config_file"] = "/bench/migration.toml" if fields.get("container_paths") else str(path)
        argv = [str(part).format_map(fields) for part in loader["argv"]]
    if "load_template" in loader:
        path = run_dir / "migration.load"
        make_config(loader["load_template"], fields, path)
        fields["load_file"] = str(path)
        argv = [str(part).format_map(fields) for part in loader["argv"]]
    if not argv or any("://" in part for part in argv):
        raise ValueError("loader argv must not contain inline connection URLs; use environment variables")
    return argv


def compose_argv(metadata, *args):
    return ["docker", "compose", "-f", str(harness.COMPOSE), "-p", metadata["project"], *args]


def compose_environment(metadata):
    env = os.environ.copy()
    env.update(metadata.get("compose_env", {}))
    return env


def normalize_wait4_maxrss(value, system=None):
    if not isinstance(value, (int, float)) or value <= 0:
        return None
    system = system or platform.system()
    if system == "Darwin":
        return int(value)
    if system == "Linux":
        return int(value) * 1024
    return None


def wait4_child(process):
    while True:
        try:
            pid, status, usage = os.wait4(process.pid, os.WNOHANG)
            break
        except InterruptedError:
            continue
    if pid == 0:
        return None
    process.returncode = os.waitstatus_to_exitcode(status)
    return process.returncode, normalize_wait4_maxrss(usage.ru_maxrss)


def run_command(argv, env, container_ids, log_path, secrets=()):
    started = time.monotonic()
    process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
    log_file = log_path.open("w")
    log_path.chmod(0o600)
    def copy_redacted_output():
        assert process.stdout is not None
        for line in process.stdout:
            log_file.write(redact(line.decode(errors="replace"), secrets))
        process.stdout.close()
        log_file.flush()
    drain = threading.Thread(target=copy_redacted_output, daemon=True)
    drain.start()
    container_samples = {}
    use_wait4 = (platform.system() in {"Darwin", "Linux"} and hasattr(os, "wait4"))
    process_rss = {"method": "wait4" if use_wait4 else "ps_sample",
                   "scope": "direct_child", "available": False, "bytes": None}
    container_cpu_core_seconds = 0.0
    next_sample = started
    last_sample = started
    next_process_sample = started
    while True:
        if use_wait4:
            waited = wait4_child(process)
            if waited is not None:
                code, peak_rss_bytes = waited
                if peak_rss_bytes is not None:
                    process_rss["available"] = True
                    process_rss["bytes"] = peak_rss_bytes
                break
        elif process.poll() is not None:
            break
        now = time.monotonic()
        if container_ids and now >= next_sample:
            result = harness.command(["docker", "stats", "--no-stream", "--format", "{{.Name}}|{{.MemUsage}}|{{.CPUPerc}}", *container_ids], check=False)
            for line in result.stdout.splitlines():
                parts = line.split("|")
                if len(parts) != 3:
                    continue
                name = parts[0].strip()
                memory = re.match(r"\s*([0-9.]+)(B|KiB|MiB|GiB)\s*/", parts[1])
                cpu = re.search(r"([0-9.]+)%", parts[2])
                if not memory or not cpu:
                    continue
                multiplier = {"B": 1, "KiB": 1024, "MiB": 1024 ** 2, "GiB": 1024 ** 3}[memory[2]]
                sample = container_samples.setdefault(name, {"peak_memory_bytes": 0, "cpu_core_seconds_sampled": 0.0})
                sample["peak_memory_bytes"] = max(sample["peak_memory_bytes"], int(float(memory[1]) * multiplier))
                interval = max(0.0, now - last_sample)
                sample["cpu_core_seconds_sampled"] += float(cpu[1]) / 100 * interval
                container_cpu_core_seconds += float(cpu[1]) / 100 * interval
            next_sample = now + 1.0
            last_sample = now
        if not use_wait4 and now >= next_process_sample:
            ps = harness.command(["ps", "-o", "rss=", "-p", str(process.pid)], check=False)
            try:
                sampled_rss = int(ps.stdout.strip()) * 1024
                if sampled_rss > 0 and sampled_rss > (process_rss["bytes"] or 0):
                    process_rss["available"] = True
                    process_rss["bytes"] = sampled_rss
            except ValueError:
                pass
            next_process_sample = now + 1.0
        time.sleep(0.1)
    code = process.wait()
    drain.join()
    log_file.close()
    log_path.chmod(0o600)
    return {
        "exit_code": code,
        "wall_seconds": time.monotonic() - started,
        "container_cpu_core_seconds_sampled": round(container_cpu_core_seconds, 4),
        "direct_child_peak_rss": process_rss,
        "container_resource_samples": container_samples,
    }


def drop_target_schema(metadata, schema):
    if not re.fullmatch(r"t21_[a-z0-9_]+", schema):
        raise ValueError("refusing to drop target schema outside the runner's t21_ namespace")
    harness.sql(metadata, "postgres", f'DROP SCHEMA IF EXISTS "{schema}" CASCADE')


def load_manifest(path):
    manifest = json.loads(path.read_text())
    if manifest.get("format") != 1 or not manifest.get("loaders"):
        raise ValueError("loader manifest format 1 must contain at least one loader")
    if not isinstance(manifest.get("comparison_contract"), dict):
        raise ValueError("loader manifest must declare its comparison_contract")
    resource_limits = manifest.get("resource_limits")
    strict_resources = len(manifest["loaders"]) > 1 or any(
        loader.get("kind") == "pgloader-v4" for loader in manifest["loaders"])
    if strict_resources:
        if not isinstance(resource_limits, dict) or set(resource_limits) != {"cpu_cores", "memory_bytes"}:
            raise ValueError("comparison manifest must declare cpu_cores and memory_bytes resource_limits")
        if not isinstance(resource_limits["cpu_cores"], (int, float)) or resource_limits["cpu_cores"] <= 0:
            raise ValueError("resource_limits.cpu_cores must be positive")
        if not isinstance(resource_limits["memory_bytes"], int) or resource_limits["memory_bytes"] <= 0:
            raise ValueError("resource_limits.memory_bytes must be a positive integer")
    required_settings = {"source", "target", "mode", "source_consistency", "source_tls", "target_tls",
                         "fresh_target_schema", "table_workers", "readers_per_table", "index_workers",
                         "row_error_policy", "durability", "indexes", "verification"}
    missing = sorted(required_settings - set(manifest["comparison_contract"]))
    if missing:
        raise ValueError("comparison_contract is missing required settings: " + ", ".join(missing))
    names = [loader.get("name") for loader in manifest["loaders"]]
    if len(names) != len(set(names)) or any(not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", name or "") for name in names):
        raise ValueError("loader names must be unique lowercase identifiers")
    for loader in manifest["loaders"]:
        if not isinstance(loader.get("argv"), list) or not loader["argv"]:
            raise ValueError(f"loader {loader.get('name')!r} needs a nonempty argv array")
        if "config_template" not in loader and "load_template" not in loader and loader.get("kind") != "pgloader-v4":
            raise ValueError(f"loader {loader['name']} needs a config_template or load_template")
        if loader.get("settings") != manifest["comparison_contract"]:
            raise ValueError(f"loader {loader['name']} settings differ from comparison_contract")
        if not isinstance(loader.get("identity"), dict):
            raise ValueError(f"loader {loader['name']} needs a reproducible identity block")
        if strict_resources and loader.get("resource_limits") != resource_limits:
            raise ValueError(f"loader {loader['name']} resource_limits differ from comparison resource_limits")
        if strict_resources and loader.get("resource_enforcement") != "docker-container":
            raise ValueError(f"loader {loader['name']} must use the docker-container resource boundary")
        if loader.get("kind", "command") not in {"command", "my2pg-container", "pgloader-v4"}:
            raise ValueError(f"loader {loader['name']} has an unsupported adapter kind")
        if strict_resources and loader.get("kind") not in {"my2pg-container", "pgloader-v4"}:
            raise ValueError(f"loader {loader['name']} is not run inside an enforceable Docker container")
        if loader.get("kind") == "pgloader-v4":
            settings = loader.get("settings")
            if not isinstance(settings, dict):
                raise ValueError("pgloader-v4 settings must declare batch_rows, batch_bytes, and queue_batches")
            integer_limits = {
                "batch_rows": 2**31 - 1,
                "batch_bytes": 2**63 - 1,
                "queue_batches": 2**31 - 1,
            }
            for key, maximum in integer_limits.items():
                value = settings.get(key)
                if type(value) is not int or value <= 0 or value > maximum:
                    raise ValueError(f"pgloader-v4 settings.{key} must be a positive integer no greater than {maximum}")
            identity = loader["identity"]
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", identity.get("image_id", "")):
                raise ValueError("pgloader-v4 identity must pin an exact Docker image_id")
            if not re.fullmatch(r"[0-9a-f]{40}", identity.get("revision", "")):
                raise ValueError("pgloader-v4 identity must pin a 40-character source revision")
    return manifest


def _loopback_endpoint(raw_url, expected_scheme, expected_database):
    parsed = urlsplit(raw_url)
    if parsed.scheme != expected_scheme or parsed.hostname not in {"127.0.0.1", "localhost"}:
        raise ValueError(f"{expected_scheme} endpoint must use a loopback host and verified-TLS fixture certificate")
    if not parsed.port or parsed.path != "/" + expected_database:
        raise ValueError(f"{expected_scheme} endpoint has an unexpected port or database")
    if parsed.username is None or parsed.password is None:
        raise ValueError(f"{expected_scheme} endpoint must provide fixture credentials")
    if any(ord(char) < 32 or ord(char) == 127 for char in parsed.username + parsed.password):
        raise ValueError("fixture credentials may not contain control characters")
    return parsed


def _pgpass_escape(value):
    return value.replace("\\", "\\\\").replace(":", "\\:")


def docker_service_url(raw_url, expected_scheme, expected_database, service, port):
    parsed = _loopback_endpoint(raw_url, expected_scheme, expected_database)
    user_info = parsed.netloc.rsplit("@", 1)[0]
    return urlunsplit((parsed.scheme, f"{user_info}@{service}:{port}", parsed.path, parsed.query, ""))


def write_env_file(path, values):
    lines = []
    for key, value in sorted(values.items()):
        if not re.fullmatch(r"[A-Z_][A-Z0-9_]*", key) or any(char in str(value) for char in "\r\n\0"):
            raise ValueError("container environment values must be single-line KEY=value entries")
        lines.append(f"{key}={value}")
    path.write_text("\n".join(lines) + "\n")
    path.chmod(0o600)


def limited_container_argv(image, run_id, command, run_dir, env_file, ca_file, metadata, limits,
                           extra_mounts=()):
    if not re.fullmatch(r"[0-9]{2}-[a-z0-9_-]+-(?:correctness|warmup|measured)-[0-9]{2}", run_id):
        raise ValueError("invalid benchmark run ID for container ownership")
    name = "my2pg-t21-" + run_id.lower().replace("_", "-")
    cpu_cores = limits["cpu_cores"]
    memory_bytes = limits["memory_bytes"]
    if cpu_cores <= 0 or memory_bytes <= 0:
        raise ValueError("Docker loader resource limits must be positive")
    uid = os.getuid() if hasattr(os, "getuid") else 10001
    gid = os.getgid() if hasattr(os, "getgid") else 10001
    argv = [
        "docker", "run", "--name", name,
        "--network", metadata["project"] + "_default",
        "--label", "org.my2pg.t21=true",
        "--cpus", str(cpu_cores),
        "--memory", str(memory_bytes),
        "--memory-swap", str(memory_bytes),
        "--user", f"{uid}:{gid}",
        "--env-file", str(env_file),
        "--mount", f"type=bind,src={run_dir.resolve()},dst=/bench",
        "--mount", f"type=bind,src={ca_file.resolve()},dst=/tls/ca.pem,readonly",
        "--entrypoint", command[0], image, *command[1:],
    ]
    for host_path, container_path in extra_mounts:
        host_path = Path(host_path)
        if not host_path.is_absolute() or not host_path.is_dir() or not container_path.startswith("/"):
            raise ValueError("extra loader mounts require an absolute existing host directory and container path")
        argv[argv.index("--entrypoint"):argv.index("--entrypoint")] = [
            "--mount", f"type=bind,src={host_path.resolve()},dst={container_path}",
        ]
    return name, argv


def build_my2pg_benchmark_image():
    image = "my2pg-t21:" + uuid.uuid4().hex[:12]
    revision = command_output(["git", "rev-parse", "HEAD"]) or "working-tree"
    built = harness.command(["docker", "build", "--tag", image, "--build-arg", f"VCS_REF={revision}",
                             "--file", str(ROOT / "Dockerfile"), str(ROOT)], check=False)
    if built.returncode:
        raise RuntimeError("could not build the pinned My2pg benchmark runtime image")
    inspected = harness.command(["docker", "image", "inspect", image], check=False)
    if inspected.returncode:
        raise RuntimeError("built My2pg benchmark image could not be inspected")
    image_id = json.loads(inspected.stdout)[0].get("Id")
    hashed = harness.command(["docker", "run", "--rm", "--entrypoint", "/usr/bin/sha256sum",
                              image, "/usr/local/bin/my2pg"], check=False)
    match = re.match(r"([0-9a-f]{64})\s", hashed.stdout)
    if hashed.returncode or not image_id or not match:
        raise RuntimeError("built My2pg benchmark image has no verifiable binary identity")
    return {"image": image, "image_id": image_id, "binary_sha256": match[1], "source_revision": revision}


def build_pgloader_t21_image(loader):
    """Build the pinned pgloader source with My2pg's out-of-tree phase patch."""
    reference = ROOT.parent / "pgloader"
    source = reference / "clojure"
    identity = loader["identity"]
    revision = command_output(["git", "-C", str(reference), "rev-parse", "HEAD"])
    dirty = command_output(["git", "-C", str(reference), "status", "--porcelain"])
    if revision != identity.get("revision") or dirty:
        raise ValueError("pgloader reference checkout must be clean at the manifest-pinned revision")
    patch_file = HERE / "patches" / "pgloader-t21-phase-intervals.patch"
    patch_sha256 = hashlib.sha256(patch_file.read_bytes()).hexdigest()
    image = "my2pg-pgloader-t21:" + patch_sha256[:12]
    with tempfile.TemporaryDirectory(prefix="my2pg-pgloader-build-") as directory:
        context = Path(directory)
        for name in ("Dockerfile", "deps.edn", "build.clj"):
            shutil.copy2(source / name, context / name)
        for name in ("src", "resources", "test"):
            shutil.copytree(source / name, context / name)
        dockerfile = context / "Dockerfile"
        dockerfile_text = dockerfile.read_text()
        build_step = "RUN clojure -T:build uber"
        if dockerfile_text.count(build_step) != 1:
            raise ValueError("pinned pgloader Dockerfile build step changed")
        dockerfile.write_text(dockerfile_text.replace(
            build_step,
            "RUN clojure -M:test -m cognitect.test-runner -n pgloader.summary-test\n"
            "RUN clojure -M:test -m cognitect.test-runner -n pgloader.load-file.parser-test\n\n"
            + build_step,
            1,
        ))
        check = subprocess.run(["git", "apply", "--recount", "--check", str(patch_file)],
                               cwd=context, capture_output=True, text=True, check=False)
        if check.returncode:
            raise ValueError("pgloader phase patch does not apply to the pinned source: "
                             + check.stderr.strip())
        applied = subprocess.run(["git", "apply", "--recount", str(patch_file)],
                                 cwd=context, capture_output=True, text=True, check=False)
        if applied.returncode:
            raise ValueError("could not apply the out-of-tree pgloader phase patch: "
                             + applied.stderr.strip())
        built = harness.command(["docker", "build", "--tag", image, "--file",
                                 str(context / "Dockerfile"), str(context)], check=False)
        if built.returncode:
            detail = (built.stderr or built.stdout).strip()
            raise RuntimeError("could not build the instrumented pgloader image"
                               + (": " + detail if detail else ""))
    inspected = harness.command(["docker", "image", "inspect", image], check=False)
    if inspected.returncode:
        raise RuntimeError("instrumented pgloader image could not be inspected")
    image_id = json.loads(inspected.stdout)[0].get("Id")
    if not image_id:
        raise RuntimeError("instrumented pgloader image has no verifiable image identity")
    return {"image": image, "image_id": image_id, "source_revision": revision,
            "instrumentation_patch_sha256": patch_sha256,
            "source_image": identity.get("image"), "source_image_id": identity.get("image_id")}


def inspect_container_limits(name, expected_image_id, limits):
    inspected = harness.command(["docker", "inspect", name], check=False)
    if inspected.returncode:
        return {"verified": False, "reason": "loader container was not inspectable after exit"}
    item = json.loads(inspected.stdout)[0]
    host = item["HostConfig"]
    nano_cpus = host.get("NanoCpus", 0)
    quota, period = host.get("CpuQuota", 0), host.get("CpuPeriod", 0)
    actual_cpus = nano_cpus / 1_000_000_000 if nano_cpus else quota / period if quota > 0 and period > 0 else None
    values = {
        "container_name": name,
        "image_id": item.get("Image"),
        "memory_bytes": host.get("Memory", 0),
        "memory_swap_bytes": host.get("MemorySwap", 0),
        "cpu_cores": actual_cpus,
        "exit_code": item.get("State", {}).get("ExitCode"),
        "oom_killed": item.get("State", {}).get("OOMKilled"),
    }
    values["verified"] = (
        values["image_id"] == expected_image_id
        and values["memory_bytes"] == limits["memory_bytes"]
        and values["memory_swap_bytes"] == limits["memory_bytes"]
        and isinstance(actual_cpus, (int, float))
        and abs(actual_cpus - limits["cpu_cores"]) < 0.000001
    )
    if not values["verified"]:
        values["reason"] = "container image or enforced CPU/memory/swap values differed from the manifest"
    return values


def run_limited_container(image, image_id, run_id, command, run_dir, env_file, ca_file,
                          metadata, limits, log_path, secrets=(), extra_mounts=()):
    name, argv = limited_container_argv(image, run_id, command, run_dir, env_file,
                                        ca_file, metadata, limits, extra_mounts)
    try:
        result = run_command(argv, os.environ.copy(), [name], log_path, secrets)
        resources = inspect_container_limits(name, image_id, limits)
    finally:
        removed = harness.command(["docker", "rm", "--force", name], check=False)
    if removed.returncode:
        resources["verified"] = False
        resources["cleanup_error"] = removed.stderr.strip() or "docker rm failed"
    result["container_resource_limits"] = resources
    if not resources["verified"]:
        result["exit_code"] = result["exit_code"] or 1
    return result, argv


def prepare_pgloader_v4(loader, values, metadata, temp_root, runtime_identity=None):
    """Build a v4 invocation with ephemeral credentials and verified TLS."""
    temp_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    identity = runtime_identity or loader["identity"]
    image = identity["image"]
    inspect = harness.command(["docker", "image", "inspect", image], check=False)
    if inspect.returncode:
        raise ValueError("pinned pgloader image is unavailable")
    image_id = json.loads(inspect.stdout)[0].get("Id")
    if image_id != identity["image_id"]:
        raise ValueError("pgloader image ID differs from the pinned artifact")
    container = harness.command(["docker", "create", "--entrypoint", "/bin/true", image], check=False)
    if container.returncode:
        raise ValueError("could not create a stopped container to extract the pinned pgloader JAR")
    container_id = container.stdout.strip()
    jar = temp_root / "pgloader.jar"
    try:
        container_inspect = harness.command(["docker", "inspect", container_id], check=False)
        if container_inspect.returncode or json.loads(container_inspect.stdout)[0].get("Image") != identity["image_id"]:
            raise ValueError("stopped pgloader container image differs from the pinned image ID")
        copied = harness.command(["docker", "cp", container_id + ":/app/pgloader.jar", str(jar)], check=False)
        if copied.returncode or not jar.is_file():
            raise ValueError("pinned pgloader image does not contain /app/pgloader.jar")
    finally:
        harness.command(["docker", "rm", container_id], check=False)
    version = harness.command(["docker", "run", "--rm", "--entrypoint", "java", image,
                              "-jar", "/app/pgloader.jar", "--version"], check=False)
    if version.returncode or not re.fullmatch(r"pgloader v4\.0\.0", version.stdout.strip()):
        raise ValueError("pinned pgloader JAR did not report the expected v4.0.0 version")
    source = _loopback_endpoint(metadata["env"]["MY2PG_MYSQL_URL"], "mysql", "source")
    target = _loopback_endpoint(metadata["env"]["MY2PG_POSTGRES_URL"], "postgresql", "target")
    ca_file = Path(metadata["env"].get("MY2PG_TLS_CA", ""))
    if not ca_file.is_file():
        raise ValueError("fixture CA certificate is missing; verified TLS cannot be configured")
    run_dir = Path(values["run_dir"])
    reject_dir = run_dir / "rejects"
    reject_dir.mkdir(mode=0o700, exist_ok=True)
    load_file = run_dir / "migration.load"
    settings = loader["settings"]
    load_text = (
        "load database\\n"
        f"     from mysql://{source.username}@mysql:3306/source?sslMode=VERIFY_IDENTITY\\n"
        f"     into postgresql://{target.username}@postgres:5432/target?sslmode=verify-full&sslrootcert=/tls/ca.pem\\n\\n"
        " WITH workers = 1,\\n"
        "      concurrency = 1,\\n"
        f"      batch rows = {settings['batch_rows']},\\n"
        f"      batch size = {settings['batch_bytes']},\\n"
        f"      prefetch rows = {settings['queue_batches']},\\n"
        "      rows per range = 25000,\\n"
        "      create tables,\\n"
        "      create indexes,\\n"
        "      quote identifiers\\n\\n"
        + (f" CAST column {values['table']}.payload to text\\n"
           if values.get("workload") == "bad_rows_fixed_rate" else
           f" CAST column {values['table']}.payload to bytea using hex-to-bytea,\\n"
           f"      column {values['table']}.optional_bytes to bytea using hex-to-bytea\\n"
           if values.get("workload") == "binary_large_rows" else "")
        + f" ALTER SCHEMA 'source' RENAME TO '{values['target_schema']}'\\n"
        + f" INCLUDING ONLY TABLE NAMES MATCHING {values.get('pgloader_table_pattern', repr(values.get('table', CORPUS['table'])))};\\n"
    ).replace("\\n", "\n")
    load_file.write_text(load_text)
    load_file.chmod(0o600)
    home = temp_root / "home"
    home.mkdir(mode=0o700)
    cnf = home / ".my.cnf"
    cnf.write_text(f"[client]\\nuser={source.username}\\npassword={source.password}\\n".replace("\\n", "\n"))
    cnf.chmod(0o600)
    pgpass = temp_root / ".pgpass"
    pgpass.write_text(":".join(["postgres", "5432", "target", target.username,
                                  _pgpass_escape(target.password)]) + "\n")
    pgpass.chmod(0o600)
    truststore = temp_root / "fixture-truststore.p12"
    password = "changeit"
    imported = harness.command(["keytool", "-importcert", "-noprompt", "-storetype", "PKCS12",
                                "-keystore", str(truststore), "-storepass", password,
                                "-alias", "fixture-ca", "-file", str(ca_file)], check=False)
    if imported.returncode:
        raise ValueError("could not create a temporary JVM truststore from the fixture CA")
    env_file = temp_root / "loader.env"
    write_env_file(env_file, {
        "HOME": "/bench/" + str(home.relative_to(run_dir)),
        "PGPASSFILE": "/bench/" + str(pgpass.relative_to(run_dir)),
        "JAVA_TOOL_OPTIONS": "-Xmx" + str(loader.get("jvm_heap_max_mib", 384)) + "m"
            + " -Djavax.net.ssl.trustStore=/bench/" + str(truststore.relative_to(run_dir))
            + " -Djavax.net.ssl.trustStoreType=PKCS12 -Djavax.net.ssl.trustStorePassword=" + password,
    })
    argv = ["java", "-jar", "/app/pgloader.jar"]
    if values.get("workload") == "bad_rows_fixed_rate":
        argv.extend(["--root-dir", "/tmp/pgloader"])
    argv.extend(["--summary", "/bench/pgloader-summary.json", "/bench/migration.load"])
    return (argv, env_file, {source.password, target.password},
            hashlib.sha256(jar.read_bytes()).hexdigest(), version.stdout.strip())


def sync_pgloader_rejects(reject_dir):
    """Flush pgloader's raw reject files and their directory before accepting a run."""
    reject_dir = Path(reject_dir)
    data_files = list(reject_dir.glob("*.reject.dat"))
    log_files = list(reject_dir.glob("*.reject.log"))
    if len(data_files) != 1 or len(log_files) != 1:
        raise ValueError("pgloader must produce exactly one reject data file and one reject log")
    started = time.monotonic()
    for path in data_files + log_files:
        descriptor = os.open(path, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    for path in (reject_dir, reject_dir.parent):
        descriptor = os.open(path, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    return time.monotonic() - started


def normalize_reject_receipt(loader, run_dir, expected_rows=None):
    """Return and validate rejected IDs and raw payload bytes from durable artifacts."""
    run_dir = Path(run_dir)
    if loader.get("kind") == "pgloader-v4":
        reject_dir = run_dir / "rejects"
        data_files = list(reject_dir.glob("*.reject.dat"))
        log_files = list(reject_dir.glob("*.reject.log"))
        if len(data_files) != 1 or len(log_files) != 1:
            raise ValueError("pgloader reject receipt files are missing or ambiguous")
        raw_rows = [row for row in data_files[0].read_bytes().split(b"\n\n") if row]
        rows = []
        for raw_row in raw_rows:
            fields = raw_row.split(b"\t", 1)
            if len(fields) != 2:
                raise ValueError("pgloader reject record does not contain ID and rejected payload")
            rows.append({"id": int(fields[0].decode("ascii")), "payload_hex": fields[1].hex()})
        ids = [row["id"] for row in rows]
        log_bytes = log_files[0].stat().st_size
        if ids and log_bytes == 0:
            raise ValueError("pgloader reject log is empty despite rejected data records")
        receipt = {"ids": ids, "rows": rows, "data_records": len(raw_rows), "log_bytes": log_bytes}
    else:
        reject_files = list((run_dir / "report").rglob("*.reject.jsonl"))
        if len(reject_files) != 1:
            raise ValueError("My2pg must produce exactly one reject JSONL file")
        rows = []
        for line in reject_files[0].read_text().splitlines():
            item = json.loads(line)
            if item.get("kind") != "conversion":
                raise ValueError("My2pg reject receipt contains a non-conversion rejection")
            values = item.get("values") or []
            if not values:
                raise ValueError("My2pg conversion rejection has no source row values")
            stored = values[0]
            if stored.get("kind") != "typed" or stored.get("value", {}).get("kind") not in {"uint", "int"}:
                raise ValueError("My2pg reject receipt does not contain a typed integer source ID")
            if len(values) < 2 or values[1].get("kind") != "bytes" or values[1].get("encoding") != "hex":
                raise ValueError("My2pg reject receipt does not contain raw source payload bytes")
            rows.append({"id": int(stored["value"]["value"]),
                         "payload_hex": str(values[1].get("value", "")).lower()})
        ids = [row["id"] for row in rows]
        report_path = reject_files[0].parent / "report.json"
        report = json.loads(report_path.read_text())
        rejected_count = sum(int(table.get("rejected_rows", 0)) for table in report.get("tables", []))
        if rejected_count != len(ids):
            raise ValueError("My2pg report rejected-row count differs from its reject JSONL records")
        receipt = {"ids": ids, "rows": rows, "data_records": len(ids), "reported_rejected_rows": rejected_count}
    if ids != sorted(ids) or len(ids) != len(set(ids)):
        raise ValueError("reject receipt IDs are not unique and ordered")
    if expected_rows is not None and rows != expected_rows:
        raise ValueError("reject receipt IDs or payload bytes differ from the MySQL source rows")
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("connections", type=Path, help="connections.json from the owned integration harness --start")
    parser.add_argument("--workload", choices=CORPUS["workloads"], default="one_large_narrow_integer")
    parser.add_argument("--profile", choices=CORPUS["profiles"], default="smoke")
    parser.add_argument("--bad-rate", choices=[str(rate) for rate in CORPUS["workloads"].get("bad_rows_fixed_rate", {}).get("rates_percent", [])], default="1",
                        help="fixed rejected-row percentage for bad_rows_fixed_rate")
    parser.add_argument("--loaders", type=Path, help="JSON command/config templates; see loaders.example.json")
    parser.add_argument("--my2pg-bin", type=Path, default=ROOT / "target/release/my2pg")
    parser.add_argument("--artifact-root", type=Path)
    execution = parser.add_mutually_exclusive_group()
    execution.add_argument("--prepare-only", action="store_true", help="seed and hash source data without starting loaders")
    execution.add_argument("--correctness-only", action="store_true",
                           help="run each configured loader once and gate correctness without accepting timings")
    args = parser.parse_args()
    metadata = json.loads(args.connections.read_text())
    harness.validate_owned(metadata)
    if metadata.get("state") != "ready":
        raise ValueError("benchmark requires an already-ready owned integration fixture")
    if not args.prepare_only and not args.loaders:
        raise ValueError("timed execution requires an explicit loader manifest")

    profile = workload_profile(args.workload, args.profile)
    rows, seed = profile["rows"], CORPUS["seed"]
    rate_label = "-rate" + args.bad_rate.replace(".", "p") if args.workload == "bad_rows_fixed_rate" else ""
    artifact = args.artifact_root or (Path(metadata["artifact_dir"]) / "t21" / (args.workload + "-" + args.profile + rate_label + "-" + uuid.uuid4().hex[:12]))
    artifact.mkdir(parents=True, exist_ok=False)
    artifact.chmod(0o700)
    pins = resource_pins(metadata)
    source_statement = source_seed_sql(rows, seed, args.workload, args.bad_rate)
    (artifact / "source-seed.sql").write_text(source_statement + "\n")
    (artifact / "source-seed.sql").chmod(0o600)
    harness.sql(metadata, "mysql", source_statement)
    expected = source_oracle(metadata, args.workload, args.bad_rate, rows=rows)
    expected_rejected_rows = expected.pop("rejected_rows", [])
    expected_rejected_ids = expected.pop("rejected_ids", [])
    expected_rows = workload_good_row_count(args.workload, rows, args.bad_rate)
    if expected["rows"] != expected_rows:
        raise RuntimeError(f"seed created {expected['rows']} rows; expected {expected_rows}")

    report = {
        "format": 1,
        "status": "prepared",
        "profile": args.profile,
        "workload": args.workload,
        "bad_row_rate_percent": args.bad_rate if args.workload == "bad_rows_fixed_rate" else None,
        "expected_rejected_ids": expected_rejected_ids,
        "corpus": CORPUS,
        "corpus_manifest_sha256": hashlib.sha256((HERE / "corpus.json").read_bytes()).hexdigest(),
        "seed_sql_sha256": hashlib.sha256(source_statement.encode()).hexdigest(),
        "expected_source": expected,
        "expected_payload_bytes": expected["payload_bytes"] if "payload_bytes" in expected
        else profile["expected_payload_bytes"] * expected_rows // rows,
        "host": {"os": platform.platform(), "architecture": platform.machine(), "cpu_model": platform.processor(),
                 "logical_cpu_count": os.cpu_count(), "python": sys.version.split()[0]},
        "storage_driver": command_output(["docker", "info", "--format", "{{.Driver}}"]),
        "source_target_versions": metadata.get("versions"),
        "application_binary": {
            "path": str(args.my2pg_bin.resolve()),
            "sha256": hashlib.sha256(args.my2pg_bin.read_bytes()).hexdigest() if args.my2pg_bin.is_file() else None,
            "git_revision": command_output(["git", "rev-parse", "HEAD"]),
            "rustc": command_output(["rustc", "-Vv"]),
            "cargo": command_output([str(ROOT / "bin/cargo"), "--version"]),
        },
        "database_settings": {
            "mysql": harness.sql(metadata, "mysql", "SELECT CONCAT_WS(',', @@global.innodb_flush_log_at_trx_commit, @@global.sync_binlog, @@global.innodb_doublewrite, @@global.max_allowed_packet, @@global.time_zone)"),
            "postgresql": harness.sql(metadata, "postgres", "SELECT STRING_AGG(name || '=' || setting, ',' ORDER BY name) FROM pg_settings WHERE name = ANY(ARRAY['fsync','full_page_writes','synchronous_commit','wal_sync_method','shared_buffers'])"),
        },
        "resource_pins": pins,
        "fixture_images": metadata.get("images"),
        "fixture_project": metadata["project"],
        "fixture_architecture": metadata.get("architecture"),
        "profile_digest": hashlib.sha256(json.dumps({"workload": args.workload, "profile": args.profile, "bad_rate": args.bad_rate if args.workload == "bad_rows_fixed_rate" else None, **profile, "seed": seed}, sort_keys=True).encode()).hexdigest(),
        "execution_mode": ("prepare-only" if args.prepare_only else
                           "correctness-only" if args.correctness_only else "benchmark"),
        "warmup_count": 0 if args.correctness_only else 1,
        "measured_runs_per_loader": 0 if args.correctness_only else 5,
        "timing_order_seed": seed,
        "correctness_gate": "independent MySQL/PostgreSQL ordered row streams plus row count, SQL aggregates, target columns and primary key; bad-row runs also match exact durable reject IDs and payload bytes",
        "runs": [],
        "timing_eligible": False,
        "limitations": ["Container CPU/memory limits are recorded from the owned fixture; absent limits disqualify acceptance timing.",
                        "The runner does not start or stop database fixtures; each loader runs in a disposable, resource-limited container.",
                        "Loader-reported phase markers and COPY durations are retained where the loader exposes them; wall time is measured around the complete command.",
                        "Container memory and CPU are sampled with one-second Docker stats; wait4 RSS covers the direct child process (the Docker client for containerized loaders)."]
    }
    if args.workload == "bad_rows_fixed_rate":
        report["expected_rejected_count"] = len(expected_rejected_ids)
    if args.prepare_only:
        report["status"] = "correctness-ready-source-only"
        (artifact / "results.json").write_text(json.dumps(report, indent=2) + "\n")
        print(artifact / "results.json")
        return

    loader_manifest = load_manifest(args.loaders)
    container_ids = harness.validate_owned(metadata)
    env = os.environ.copy()
    env.update(metadata.get("env", {}))
    env.update(metadata.get("compose_env", {}))
    loader_images = {}
    for loader in loader_manifest["loaders"]:
        if loader.get("kind") == "my2pg-container":
            loader_images[loader["name"]] = build_my2pg_benchmark_image()
        elif loader.get("kind") == "pgloader-v4":
            loader_images[loader["name"]] = build_pgloader_t21_image(loader)
    loader_manifest_bytes = args.loaders.read_bytes()
    (artifact / "loader-manifest.json").write_text(redact(loader_manifest_bytes.decode(errors="replace")))
    (artifact / "loader-manifest.json").chmod(0o600)
    report["comparison_contract"] = loader_manifest["comparison_contract"]
    report["resource_limits"] = loader_manifest.get("resource_limits")
    report["loader_equivalence_gaps"] = {
        loader["name"]: loader.get("unsupported_equivalence", []) for loader in loader_manifest["loaders"]
    }
    report["loader_runtime_images"] = loader_images
    loaders = loader_manifest["loaders"]
    if args.correctness_only:
        plan = [(loader, "correctness", 0) for loader in loaders]
    else:
        plan = [(loader, "warmup", 0) for loader in loaders]
        plan += [(loader, "measured", run) for run in range(1, 6) for loader in loaders]
    random.Random(seed).shuffle(plan)
    for sequence, (loader, kind, index) in enumerate(plan, 1):
        run_id = f"{sequence:02d}-{loader['name']}-{kind}-{index:02d}"
        run_dir = artifact / run_id
        run_dir.mkdir(mode=0o700)
        schema = "t21_" + re.sub(r"[^a-z0-9_]+", "_", loader["name"].lower()) + "_" + uuid.uuid4().hex[:8]
        drop_target_schema(metadata, schema)
        local_binary = loader.get("identity", {}).get("kind") == "local-binary"
        values = {"target_schema": schema,
                  "report_dir": str(run_dir / "report") if local_binary else "/bench/report",
                  "workload": args.workload,
                  "config_file": "/bench/migration.toml", "load_file": "/bench/migration.load",
                  "run_dir": str(run_dir),
                  "my2pg_bin": str(args.my2pg_bin.resolve()) if local_binary else "my2pg",
                  "source_ca_file": metadata["env"]["MY2PG_TLS_CA"] if local_binary else "/tls/ca.pem",
                  "target_ca_file": metadata["env"]["MY2PG_TLS_CA"] if local_binary else "/tls/ca.pem",
                  "source_host": "mysql", "target_host": "postgres", "source_database": "source",
                  "target_database": "target", **workload_table_values(args.workload),
                  "rows": str(rows), "seed": str(seed), "bad_rate": args.bad_rate}
        secrets = set()
        jar_sha256 = None
        loader_version = None
        with tempfile.TemporaryDirectory(prefix=".t21-loader-", dir=run_dir) as temporary:
            if loader.get("kind") == "pgloader-v4":
                argv, env_file, secrets, jar_sha256, loader_version = prepare_pgloader_v4(
                    loader, values, metadata, Path(temporary), loader_images[loader["name"]])
                result, argv = run_limited_container(
                    loader_images[loader["name"]]["image"], loader_images[loader["name"]]["image_id"],
                    run_id, argv, run_dir, env_file, Path(metadata["env"]["MY2PG_TLS_CA"]), metadata,
                    loader_manifest["resource_limits"], run_dir / "loader.log", secrets,
                    extra_mounts=[(run_dir / "rejects", "/tmp/pgloader")])
            elif loader.get("kind") == "my2pg-container":
                values["container_paths"] = True
                argv = loader_invocation(loader, values, run_dir)
                source_url = docker_service_url(metadata["env"]["MY2PG_MYSQL_URL"], "mysql", "source", "mysql", 3306)
                target_url = docker_service_url(metadata["env"]["MY2PG_POSTGRES_URL"], "postgresql", "target", "postgres", 5432)
                env_file = Path(temporary) / "loader.env"
                write_env_file(env_file, {"MY2PG_MYSQL_URL": source_url, "MY2PG_POSTGRES_URL": target_url})
                result, argv = run_limited_container(
                    loader_images[loader["name"]]["image"], loader_images[loader["name"]]["image_id"],
                    run_id, argv, run_dir, env_file, Path(metadata["env"]["MY2PG_TLS_CA"]), metadata,
                    loader_manifest["resource_limits"], run_dir / "loader.log")
            else:
                argv = loader_invocation(loader, values, run_dir)
                loader_env = env
                result = run_command(argv, loader_env, container_ids, run_dir / "loader.log", secrets)
        report_file = report_path_for_run(run_dir)
        summary_file = run_dir / "pgloader-summary.json"
        if loader.get("kind") == "pgloader-v4":
            result["pgloader_summary_metrics"] = pgloader_summary_metrics(summary_file)
        reject_receipt = None
        reject_sync_seconds = None
        if args.workload == "bad_rows_fixed_rate" and loader.get("kind") == "pgloader-v4":
            try:
                reject_sync_seconds = sync_pgloader_rejects(run_dir / "rejects")
                result["wall_seconds"] += reject_sync_seconds
            except (OSError, ValueError) as error:
                result["reject_sync_error"] = str(error)
                result["exit_code"] = 1
        observed = None
        verification_source = None
        verification_elapsed_millis = None
        verify_started = time.monotonic()
        accepted_exit_codes = {0, 3} if args.workload == "bad_rows_fixed_rate" else {0}
        if result["exit_code"] in accepted_exit_codes:
            verification_source = source_oracle(metadata, args.workload, args.bad_rate, rows=rows)
            observed = (target_oracle(metadata, schema, args.workload, args.bad_rate)
                        if args.workload == "bad_rows_fixed_rate"
                        else target_oracle(metadata, schema, args.workload))
            verification_elapsed_millis = int(round((time.monotonic() - verify_started) * 1000))
            if args.workload == "bad_rows_fixed_rate":
                reject_receipt = normalize_reject_receipt(loader, run_dir, expected_rejected_rows)
                reject_receipt["expected_ids"] = expected_rejected_ids
                reject_receipt["passed"] = (reject_receipt["ids"] == expected_rejected_ids
                                             and reject_receipt["rows"] == expected_rejected_rows)
        result["correctness_verify_seconds"] = (
            verification_elapsed_millis / 1000 if verification_elapsed_millis is not None else None
        )
        source_rows = (verification_source or {}).pop("rejected_rows", [])
        source_ids = (verification_source or {}).pop("rejected_ids", [])
        source_matches_seed = bool(verification_source and verification_source == expected
                                   and source_rows == expected_rejected_rows
                                   and source_ids == expected_rejected_ids)
        passed = bool(observed and observed == expected and source_matches_seed)
        if args.workload == "bad_rows_fixed_rate":
            passed = passed and bool(reject_receipt and reject_receipt.get("passed"))
        result["phase_metrics"] = phase_metrics(
            (run_dir / "loader.log").read_text(),
            report_file,
            loader.get("kind", "command"),
            verification_elapsed_millis,
            summary_file if loader.get("kind") == "pgloader-v4" else None,
        )
        record = {"run_id": run_id, "loader": loader["name"], "kind": kind, "iteration": index,
                  "loader_identity": loader.get("identity"),
                  "loader_runtime_image": loader_images.get(loader["name"]),
                  "argv_redacted": [redact(item, secrets) for item in argv],
                  "loader_artifact_sha256": jar_sha256, "loader_version": loader_version, **result,
                  "correctness": "passed" if passed else "failed" if observed else "not_run",
                  "verification_source_matches_seed": source_matches_seed,
                  "verification_source_sha256": (verification_source or {}).get("sha256"),
                  "expected": expected, "observed": observed,
                  "pgloader_summary_path": str(summary_file) if summary_file.exists() else None,
                  "pgloader_summary_sha256": hashlib.sha256(summary_file.read_bytes()).hexdigest()
                      if summary_file.exists() else None,
                  "reject_receipt": reject_receipt, "reject_sync_seconds": reject_sync_seconds,
                  "report_path": str(report_file) if report_file.exists() else None,
                  "report_sha256": hashlib.sha256(report_file.read_bytes()).hexdigest() if report_file.exists() else None,
                  "loader_log_sha256": hashlib.sha256((run_dir / "loader.log").read_bytes()).hexdigest()}
        report["runs"].append(record)
        report["status"] = "running"
        (artifact / "results.json").write_text(json.dumps(report, indent=2) + "\n")
        drop_target_schema(metadata, schema)
        if result["exit_code"] not in accepted_exit_codes or not passed:
            report["status"] = "correctness-failed"
            report["timing_eligible"] = False
            (artifact / "results.json").write_text(json.dumps(report, indent=2) + "\n")
            raise RuntimeError(f"{run_id} failed; timings are not accepted; inspect its retained artifacts")

    report["status"] = "completed-correctness-only" if args.correctness_only else "completed-correctness-gated"
    report["measured_summary"] = {}
    for loader in loaders:
        runs = [item["wall_seconds"] for item in report["runs"]
                if item["loader"] == loader["name"] and item["kind"] == "measured" and item["correctness"] == "passed"]
        report["measured_summary"][loader["name"]] = {
            "count": len(runs), "median_wall_seconds": statistics.median(runs) if runs else None,
            "min_wall_seconds": min(runs) if runs else None, "max_wall_seconds": max(runs) if runs else None,
            "spread_seconds": max(runs) - min(runs) if runs else None,
        }
    database_limits_declared = bool(pins.get("limits_declared"))
    process_limits_enforced = bool(loader_manifest.get("resource_limits")) and all(
        loader.get("resource_enforcement") == "docker-container" for loader in loaders) and all(
        run.get("container_resource_limits", {}).get("verified") for run in report["runs"])
    phase_metrics_comparable = phases_are_comparable(report["runs"])
    loader_settings_equivalent = all(not loader.get("unsupported_equivalence") for loader in loaders)
    correctness_complete = all(run["correctness"] == "passed" for run in report["runs"])
    report["equivalence_checks"] = {
        "correctness_complete": correctness_complete,
        "database_resource_limits_declared": database_limits_declared,
        "loader_cpu_memory_limits_enforced": process_limits_enforced,
        "phase_metrics_comparable": phase_metrics_comparable,
        "loader_settings_equivalent": loader_settings_equivalent,
        "at_least_two_loaders": len(loaders) >= 2,
    }
    report["timing_eligible"] = not args.correctness_only and all(report["equivalence_checks"].values())
    if args.correctness_only:
        report["limitations"].append("Correctness-only mode runs one pass per loader; no warmup or measured iterations are collected, so it cannot support timing claims.")
    if len(loaders) < 2:
        report["limitations"].append("Only one loader was run; this produces diagnostic measurements, not a comparative speed claim.")
    if not database_limits_declared:
        report["limitations"].append("At least one owned source/target database container lacks a declared CPU or memory limit; timing is ineligible.")
    if not process_limits_enforced:
        report["limitations"].append("At least one loader lacks enforced matching CPU and memory limits; comparison timing is ineligible.")
    if not phase_metrics_comparable:
        report["limitations"].append("At least one loader does not expose normalized phase metrics; phase comparison is unsupported.")
    if not loader_settings_equivalent:
        report["limitations"].append("At least one loader lacks controls in the declared comparison contract; comparative timing is ineligible.")
    (artifact / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print(artifact / "results.json")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"T21 runner error: {error}", file=sys.stderr)
        sys.exit(1)
