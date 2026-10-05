import hashlib
import importlib.util
import json
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


RUNNER_PATH = Path(__file__).with_name("runner.py")
SPEC = importlib.util.spec_from_file_location("t21_runner", RUNNER_PATH)
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)


EXPECTED_COLUMNS = "id:bigint:NO,bucket:integer:NO,measure:bigint:NO,payload:bigint:NO"
EXPECTED_SOURCE = {
    "rows": 1000,
    "sha256": "ordered-row-digest",
    "aggregate": "1000,500500,500500,44420637500,379282344500,1,1000",
    "target_columns": EXPECTED_COLUMNS,
    "primary_key": "id",
    "table_count": 1,
    "foreign_key_count": 0,
    "foreign_keys": "",
}
EXPECTED_TARGET = {
    **EXPECTED_SOURCE,
    "target_columns": EXPECTED_COLUMNS,
    "primary_key": "id",
}


class BenchmarkRunnerTests(unittest.TestCase):
    def test_corpus_and_seed_sql_digests_are_stable(self):
        corpus_path = RUNNER_PATH.with_name("corpus.json")
        self.assertEqual(
            hashlib.sha256(corpus_path.read_bytes()).hexdigest(),
            "3c77feaf3b2ec2f5025df7b519b5b7803ab8a5f93e265f4aa1d3a4a2ee209196",
        )
        for profile in runner.CORPUS["profiles"].values():
            sql = runner.source_seed_sql(profile["rows"], runner.CORPUS["seed"])
            self.assertEqual(sql, runner.source_seed_sql(profile["rows"], runner.CORPUS["seed"]))
        smoke_sql = runner.source_seed_sql(1000, runner.CORPUS["seed"])
        self.assertEqual(
            hashlib.sha256(smoke_sql.encode()).hexdigest(),
            "2c17500b4fc060816c3b93e2ab6a5d43fcebdd325847d814163c2ddef65c1d19",
        )

    def test_sparse_unsigned_workload_is_seeded_and_oracle_schema_is_exact(self):
        sql = runner.source_seed_sql(1000, runner.CORPUS["seed"], "sparse_skewed_unsigned_primary_keys")
        self.assertIn("id BIGINT UNSIGNED NOT NULL PRIMARY KEY", sql)
        self.assertIn("CAST(seqs.id AS UNSIGNED) * 100000", sql)
        self.assertIn("IF(MOD(seqs.id, 10) < 9, 0, MOD(seqs.id, 16) + 1)", sql)
        self.assertEqual(
            runner.source_seed_sql(1000, runner.CORPUS["seed"], "sparse_skewed_unsigned_primary_keys"), sql
        )
        source_query = runner.source_canonical_query("sparse_skewed_unsigned_primary_keys")
        target_query = runner.target_canonical_query("t21_sparse_1234", "sparse_skewed_unsigned_primary_keys")
        self.assertIn("CAST(id AS CHAR)", source_query)
        self.assertIn("id::text", target_query)
        self.assertNotEqual(source_query, target_query)
        self.assertEqual(
            runner.CORPUS["workloads"]["sparse_skewed_unsigned_primary_keys"]["table"],
            "t21_integer_rows",
        )

    def test_loader_configuration_uses_selected_workload_table(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            loader = {
                "name": "my2pg-test",
                "settings": {"row_error_policy": "reject"},
                "argv": ["my2pg", "run", "{config_file}"],
                "config_template": "include = {include_tables}\non_row_error = \"stop\"\nmax_rejected_rows = 0\n",
            }
            tables = runner.workload_tables("several_large_tables")
            host_argv = runner.loader_invocation(loader, {
                **runner.workload_table_values("several_large_tables"),
                "config_file": str(root / "migration.toml"),
                "target_schema": "t21_bench_1234",
            }, root)
            self.assertEqual(host_argv, ["my2pg", "run", str(root / "migration.toml")])
            container_argv = runner.loader_invocation(loader, {
                **runner.workload_table_values("several_large_tables"),
                "config_file": str(root / "migration.toml"),
                "target_schema": "t21_bench_1234",
                "container_paths": True,
            }, root)
            self.assertEqual(container_argv, ["my2pg", "run", "/bench/migration.toml"])
            self.assertTrue((root / "migration.toml").is_file())
            self.assertEqual((root / "migration.toml").read_text(),
                             f"include = {json.dumps(tables)}\non_row_error = \"reject\"\nmax_rejected_rows = 12500\n")

    def test_multi_table_workload_sql_and_oracles_cover_all_four_tables(self):
        workload = "several_large_tables"
        tables = runner.workload_tables(workload)
        sql = runner.source_seed_sql(1000, runner.CORPUS["seed"], workload)
        source_query = runner.source_canonical_query(workload)
        target_query = runner.target_canonical_query("t21_multi_1234", workload)
        self.assertEqual(len(tables), 4)
        self.assertEqual(sql.count("CREATE TABLE source.t21_large_"), 4)
        self.assertEqual(sql.count("INSERT INTO source.t21_large_"), 4)
        self.assertTrue(all(table in source_query and table in target_query for table in tables))
        self.assertEqual(runner.workload_row_count(workload, 1000), 4000)
        values = runner.workload_table_values(workload)
        self.assertEqual(json.loads(values["include_tables"]), tables)
        self.assertEqual(values["pgloader_table_pattern"], ", ".join(f"~/^{table}$/" for table in tables))

    def test_many_small_relational_workload_builds_a_valid_fk_chain(self):
        workload = "many_small_relational_tables"
        tables = runner.workload_tables(workload)
        sql = runner.source_seed_sql(1000, runner.CORPUS["seed"], workload)
        self.assertEqual(len(tables), 8)
        self.assertEqual(sql.count("FOREIGN KEY (parent_id)"), 7)
        self.assertEqual(sql.count("CREATE TABLE source.t21_rel_"), 8)
        self.assertEqual(sql.count("INSERT INTO source.t21_rel_"), 8)
        self.assertLess(sql.index("DROP TABLE IF EXISTS source.t21_rel_07"),
                        sql.index("DROP TABLE IF EXISTS source.t21_rel_00"))
        self.assertIn("IF(seqs.id=1,NULL,seqs.id-1)", sql)
        self.assertIn("COALESCE(CAST(parent_id AS CHAR), 'NULL')", runner.source_canonical_query(workload))
        self.assertIn("COALESCE(parent_id::text, 'NULL')", runner.target_canonical_query("t21_rel_1234", workload))
        self.assertEqual(runner.workload_row_count(workload, 1000), 1000)
        self.assertEqual(runner.expected_foreign_keys(workload),
                         ";".join(f"{tables[index]}:parent_id->{tables[index - 1]}:id" for index in range(1, 8)))

    def test_main_repeats_the_seeded_one_warmup_five_measured_plan(self):
        report = self.run_main()
        actual = [run["run_id"] for run in report["runs"]]
        plan = [("bench", "warmup", 0)]
        plan += [("bench", "measured", run) for run in range(1, 6)]
        random.Random(runner.CORPUS["seed"]).shuffle(plan)
        expected = [f"{sequence:02d}-{name}-{kind}-{iteration:02d}"
                    for sequence, (name, kind, iteration) in enumerate(plan, 1)]
        self.assertEqual(actual, expected)
        self.assertEqual([run["correctness"] for run in report["runs"]], ["passed"] * 6)
        self.assertEqual(sum(run["kind"] == "warmup" for run in report["runs"]), 1)
        self.assertEqual(sum(run["kind"] == "measured" for run in report["runs"]), 5)

    def test_main_runs_sparse_key_workload_through_same_correctness_gate(self):
        report = self.run_main(workload="sparse_skewed_unsigned_primary_keys")
        self.assertEqual(report["status"], "completed-correctness-gated")
        self.assertEqual(report["expected_source"]["target_columns"],
                         "id:numeric:NO,bucket:integer:NO,measure:bigint:NO,payload:bigint:NO")
        self.assertTrue(all(run["correctness"] == "passed" for run in report["runs"]))

    def test_main_runs_multi_table_workload_through_same_correctness_gate(self):
        report = self.run_main(workload="several_large_tables")
        self.assertEqual(report["status"], "completed-correctness-gated")
        self.assertEqual(report["expected_source"]["rows"], 4000)
        self.assertEqual(len(report["expected_source"]["target_columns"].split(";")), 4)
        self.assertTrue(all(run["correctness"] == "passed" for run in report["runs"]))

    def test_main_runs_relational_workload_through_fk_correctness_gate(self):
        report = self.run_main(workload="many_small_relational_tables")
        self.assertEqual(report["status"], "completed-correctness-gated")
        self.assertEqual(report["expected_source"]["table_count"], 8)
        self.assertEqual(report["expected_source"]["foreign_key_count"], 7)
        self.assertEqual(report["expected_source"]["foreign_keys"], runner.expected_foreign_keys("many_small_relational_tables"))
        self.assertEqual(len(report["expected_source"]["target_columns"].split(";")), 8)
        self.assertTrue(all(run["correctness"] == "passed" for run in report["runs"]))

    def test_wide_workload_covers_utf8_text_json_decimal_and_scaled_profiles(self):
        workload = "wide_utf8_text_json_decimal"
        sql = runner.source_seed_sql(100, runner.CORPUS["seed"], workload)
        source_query = runner.source_canonical_query(workload)
        target_query = runner.target_canonical_query("t21_wide_1234", workload)
        self.assertIn("VARCHAR(24) CHARACTER SET utf8mb4", sql)
        self.assertIn("TEXT CHARACTER SET utf8mb4", sql)
        self.assertIn("JSON_OBJECT('value', seqs.id, 'tag', '雪')", sql)
        self.assertIn("DECIMAL(30,12)", sql)
        self.assertIn("LOWER(HEX(label))", source_query)
        self.assertIn("LOWER(HEX(detail))", source_query)
        self.assertIn("LOWER(HEX(JSON_UNQUOTE(JSON_EXTRACT(json_doc, '$.tag'))))", source_query)
        self.assertIn("CAST(JSON_UNQUOTE(JSON_EXTRACT(json_doc, '$.value')) AS BINARY)", source_query)
        self.assertIn("_binary'|'", source_query)
        self.assertIn("encode(convert_to(json_doc->>'tag','UTF8'),'hex')", target_query)
        self.assertNotEqual(source_query, target_query)
        self.assertEqual(runner.workload_profile(workload, "memory")["rows"], 3_000_000)

    def test_main_runs_wide_workload_through_content_and_payload_gate(self):
        report = self.run_main(workload="wide_utf8_text_json_decimal")
        self.assertEqual(report["status"], "completed-correctness-gated")
        self.assertEqual(report["expected_source"]["rows"], 100)
        self.assertEqual(report["expected_payload_bytes"], report["expected_source"]["payload_bytes"])
        self.assertTrue(all(run["correctness"] == "passed" for run in report["runs"]))

    def test_binary_workload_covers_large_bytes_null_empty_and_scaled_profiles(self):
        workload = "binary_large_rows"
        sql = runner.source_seed_sql(100, runner.CORPUS["seed"], workload)
        source_query = runner.source_canonical_query(workload)
        target_query = runner.target_canonical_query("t21_binary_1234", workload)
        self.assertIn("payload LONGBLOB NOT NULL", sql)
        self.assertIn("optional_bytes VARBINARY(16) NULL", sql)
        self.assertIn("REPEAT(CONCAT(UNHEX(LPAD(HEX(MOD(seqs.id,256)),2,'0')),UNHEX('00ff5c80')),16384)", sql)
        self.assertIn("WHEN 0 THEN NULL WHEN 1 THEN UNHEX('')", sql)
        self.assertIn("OCTET_LENGTH(payload), '|', MD5(payload)", source_query)
        self.assertIn("LOWER(HEX(optional_bytes))", source_query)
        self.assertIn("octet_length(payload)::text || '|' || md5(payload)", target_query)
        self.assertIn("encode(optional_bytes,'hex')", target_query)
        self.assertNotEqual(source_query, target_query)
        self.assertEqual(runner.workload_profile(workload, "memory")["rows"], 30_000)
        self.assertEqual(30_000 * 81_920, 2_457_600_000)

    def test_main_runs_binary_workload_through_content_and_payload_gate(self):
        report = self.run_main(workload="binary_large_rows")
        self.assertEqual(report["status"], "completed-correctness-gated")
        self.assertEqual(report["expected_source"]["rows"], 100)
        self.assertEqual(report["expected_source"]["target_columns"],
                         "id:bigint:NO,payload:bytea:NO,optional_bytes:bytea:YES")
        self.assertEqual(report["expected_payload_bytes"], report["expected_source"]["payload_bytes"])
        self.assertTrue(all(run["correctness"] == "passed" for run in report["runs"]))

    def test_correctness_only_runs_each_loader_once_without_timing_eligibility(self):
        report = self.run_main(correctness_only=True)
        self.assertEqual(report["status"], "completed-correctness-only")
        self.assertEqual(report["execution_mode"], "correctness-only")
        self.assertEqual(report["warmup_count"], 0)
        self.assertEqual(report["measured_runs_per_loader"], 0)
        self.assertEqual(len(report["runs"]), 1)
        self.assertEqual(report["runs"][0]["kind"], "correctness")
        self.assertEqual(report["runs"][0]["correctness"], "passed")
        self.assertFalse(report["timing_eligible"])
        self.assertFalse(report["equivalence_checks"]["phase_metrics_comparable"])
        self.assertEqual(report["measured_summary"]["bench"]["count"], 0)

    def test_bad_row_workload_uses_fixed_nul_rates_and_good_row_oracles(self):
        workload = "bad_rows_fixed_rate"
        self.assertEqual(runner.bad_rate_denominator("0.1"), 1000)
        self.assertEqual(runner.bad_rate_denominator("1"), 100)
        self.assertEqual(runner.bad_rate_denominator("5"), 20)
        self.assertEqual(runner.expected_bad_row_ids(1000, "0.1"), [1000])
        self.assertEqual(runner.expected_bad_row_ids(1000, "1"), list(range(100, 1001, 100)))
        self.assertEqual(runner.workload_good_row_count(workload, 1000, "1"), 990)
        sql = runner.source_seed_sql(1000, runner.CORPUS["seed"], workload, "1")
        self.assertIn("CHAR(0)", sql)
        self.assertIn("MOD(seqs.id,100)=0", sql)
        source_query = runner.source_canonical_query(workload, "1")
        target_query = runner.target_canonical_query("t21_bad_1234", workload, "1")
        self.assertIn("WHERE MOD(id,100)<>0", source_query)
        self.assertNotIn("WHERE MOD", target_query)
        self.assertIn("upper(encode(convert_to(payload,'UTF8'),'hex'))", target_query)
        self.assertNotEqual(source_query, target_query)
        self.assertEqual(runner.workload_profile(workload, "memory")["rows"], 250_000)

    def test_bad_row_schedule_rejects_shifted_ids_even_when_count_matches(self):
        expected = runner.expected_bad_row_ids(1000, "1")
        self.assertEqual(runner.validate_bad_row_schedule(expected, 1000, "1"), expected)
        shifted = expected[:-1] + [999]
        with self.assertRaisesRegex(ValueError, "deterministic bad-row schedule"):
            runner.validate_bad_row_schedule(shifted, 1000, "1")

    def test_source_oracle_applies_schedule_check_to_mysql_reject_rows(self):
        shifted_ids = runner.expected_bad_row_ids(1000, "1")[:-1] + [999]
        output = "".join(f"{row_id}\t62616400\n" for row_id in shifted_ids)
        mysql_rows = subprocess.CompletedProcess(["mysql"], 0, output, "")
        with patch.object(runner, "stream_digest", return_value={"rows": 1000, "sha256": "source"}), \
                patch.object(runner.harness, "sql", return_value="990,495495,1,999"), \
                patch.object(runner.harness, "command", return_value=mysql_rows):
            with self.assertRaisesRegex(ValueError, "deterministic bad-row schedule"):
                runner.source_oracle({"project": "t21-test"}, "bad_rows_fixed_rate", "1", rows=1000)

    def test_reject_receipts_capture_exact_ids_and_pgloader_syncs_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected_rows = [
                {"id": 20, "payload_hex": b"bad-20\x00-tail".hex()},
                {"id": 40, "payload_hex": b"bad-40\x00-tail".hex()},
            ]
            report_dir = root / "report" / "run-0123456789abcdef"
            report_dir.mkdir(parents=True)
            (report_dir / "t_0123456789abcdef.reject.jsonl").write_text("\n".join([
                json.dumps({"kind": "conversion", "values": [
                    {"kind": "typed", "value": {"kind": "uint", "value": 20}},
                    {"kind": "bytes", "encoding": "hex", "value": expected_rows[0]["payload_hex"]},
                ]}),
                json.dumps({"kind": "conversion", "values": [
                    {"kind": "typed", "value": {"kind": "uint", "value": 40}},
                    {"kind": "bytes", "encoding": "hex", "value": expected_rows[1]["payload_hex"]},
                ]}),
            ]) + "\n")
            (report_dir / "report.json").write_text(json.dumps({"tables": [{"rejected_rows": 2}]}))
            my2pg = runner.normalize_reject_receipt({"kind": "my2pg-container"}, root, expected_rows)
            self.assertEqual(my2pg["ids"], [20, 40])
            self.assertEqual(my2pg["rows"], expected_rows)
            self.assertEqual(my2pg["reported_rejected_rows"], 2)
            self.assertEqual(runner.report_path_for_run(root), report_dir / "report.json")
            with self.assertRaisesRegex(ValueError, "payload bytes"):
                runner.normalize_reject_receipt(
                    {"kind": "my2pg-container"}, root,
                    [{"id": 20, "payload_hex": b"same-id-corruption".hex()}, expected_rows[1]],
                )

            reject_dir = root / "rejects"
            reject_dir.mkdir()
            (reject_dir / "source.t21_bad_rows.reject.dat").write_bytes(
                b"20\tbad-20\x00-tail\n\n40\tbad-40\x00-tail\n\n")
            (reject_dir / "source.t21_bad_rows.reject.log").write_text("reject 20\nreject 40\n")
            pgloader = runner.normalize_reject_receipt({"kind": "pgloader-v4"}, root, expected_rows)
            self.assertEqual(pgloader["ids"], [20, 40])
            self.assertEqual(pgloader["rows"], expected_rows)
            self.assertGreater(pgloader["log_bytes"], 0)
            with self.assertRaisesRegex(ValueError, "payload bytes"):
                runner.normalize_reject_receipt(
                    {"kind": "pgloader-v4"}, root,
                    [{"id": 20, "payload_hex": b"corrupted".hex()}, expected_rows[1]],
                )
            with patch.object(runner.os, "fsync", side_effect=OSError("sync failed")):
                with self.assertRaisesRegex(OSError, "sync failed"):
                    runner.sync_pgloader_rejects(reject_dir)
            self.assertGreaterEqual(runner.sync_pgloader_rejects(reject_dir), 0)
            (reject_dir / "source.t21_bad_rows.reject.dat").write_bytes(b"20\tbad-20\x00-tail\n\n")
            self.assertEqual(runner.normalize_reject_receipt(
                {"kind": "pgloader-v4"}, root, [expected_rows[0]])["ids"], [20])
            (reject_dir / "source.t21_bad_rows.reject.dat").unlink()
            with self.assertRaisesRegex(ValueError, "exactly one reject"):
                runner.sync_pgloader_rejects(reject_dir)

    def test_main_gates_bad_rows_on_target_content_and_exact_reject_receipt(self):
        report = self.run_main(workload="bad_rows_fixed_rate")
        self.assertEqual(report["expected_rejected_count"], 10)
        self.assertEqual(report["expected_rejected_ids"], list(range(100, 1001, 100)))
        self.assertEqual(report["expected_source"]["rows"], 990)
        self.assertTrue(all(run["correctness"] == "passed" for run in report["runs"]))
        self.assertTrue(all(run["reject_receipt"]["passed"] for run in report["runs"]))

    def test_main_rejects_each_changed_correctness_oracle_field(self):
        mismatches = {
            "rows": 999,
            "sha256": "different-row-digest",
            "aggregate": "different-aggregate",
            "target_columns": "id:text:NO",
            "primary_key": "payload",
            "table_count": 2,
            "foreign_key_count": 1,
            "foreign_keys": "wrong_parent:parent_id->other:id",
            "payload_bytes": 1,
        }
        for field, value in mismatches.items():
            with self.subTest(field=field):
                report = self.run_main(target_override={field: value}, expect_failure=True)
                self.assertEqual(report["status"], "correctness-failed")
                self.assertFalse(report["timing_eligible"])
                self.assertEqual(report["runs"][0]["correctness"], "failed")

    def test_drop_target_schema_rejects_outside_namespace_before_sql(self):
        metadata = {"project": "my2pg-test"}
        with patch.object(runner.harness, "sql") as execute_sql:
            with self.assertRaisesRegex(ValueError, "outside the runner's t21_ namespace"):
                runner.drop_target_schema(metadata, "public")
            execute_sql.assert_not_called()

    def test_wait4_captures_peak_rss_for_a_short_lived_child(self):
        self.assertEqual(runner.normalize_wait4_maxrss(2048, "Darwin"), 2048)
        self.assertEqual(runner.normalize_wait4_maxrss(2, "Linux"), 2048)
        self.assertIsNone(runner.normalize_wait4_maxrss(0, "Linux"))
        self.assertIsNone(runner.normalize_wait4_maxrss(2048, "Other"))
        if runner.platform.system() not in {"Darwin", "Linux"} or not hasattr(runner.os, "wait4"):
            self.skipTest("exact wait4 child RSS is supported on Darwin and Linux")

        with tempfile.TemporaryDirectory() as directory:
            log_path = Path(directory) / "loader.log"
            child = (
                "import time; data = bytearray(32 * 1024 * 1024); "
                "[data.__setitem__(i, 1) for i in range(0, len(data), 4096)]; "
                "time.sleep(0.05)"
            )
            result = runner.run_command([sys.executable, "-c", child], {}, [], log_path)

        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(result["direct_child_peak_rss"]["method"], "wait4")
        self.assertEqual(result["direct_child_peak_rss"]["scope"], "direct_child")
        self.assertTrue(result["direct_child_peak_rss"]["available"])
        self.assertGreaterEqual(result["direct_child_peak_rss"]["bytes"], 24 * 1024 * 1024)

    def test_pgloader_adapter_uses_dynamic_loopback_tls_and_ephemeral_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run_dir = root / "run"
            run_dir.mkdir()
            ca_file = root / "ca.pem"
            ca_file.write_text("test ca")
            values = {"run_dir": str(run_dir), "target_schema": "t21_test_1234",
                      **runner.workload_table_values("several_large_tables")}
            metadata = {
                "env": {
                    "MY2PG_MYSQL_URL": "mysql://my2pg:mysql-secret@127.0.0.1:43101/source",
                    "MY2PG_POSTGRES_URL": "postgresql://my2pg:postgres-secret@127.0.0.1:45432/target",
                    "MY2PG_TLS_CA": str(ca_file),
                },
                "project": "my2pg-test",
                "compose_env": {},
            }
            loader = {
                "identity": {"image": "pinned:test", "image_id": "sha256:" + "a" * 64},
                "resource_limits": {"cpu_cores": 1, "memory_bytes": 536870912},
                "settings": {"batch_rows": 12345, "batch_bytes": 8388608, "queue_batches": 3},
            }

            def fake_command(argv, check=True):
                if argv[:3] == ["docker", "image", "inspect"]:
                    return subprocess.CompletedProcess(argv, 0, json.dumps([{"Id": loader["identity"]["image_id"]}]), "")
                if argv[:2] == ["docker", "create"]:
                    return subprocess.CompletedProcess(argv, 0, "stopped-container\n", "")
                if argv[:2] == ["docker", "inspect"]:
                    return subprocess.CompletedProcess(argv, 0, json.dumps([{"Image": loader["identity"]["image_id"]}]), "")
                if argv[:2] == ["docker", "cp"]:
                    Path(argv[-1]).write_bytes(b"truthful pinned jar")
                    return subprocess.CompletedProcess(argv, 0, "", "")
                if argv[:3] == ["docker", "run", "--rm"]:
                    return subprocess.CompletedProcess(argv, 0, "pgloader v4.0.0\n", "")
                return subprocess.CompletedProcess(argv, 0, "", "")

            with patch.object(runner.harness, "command", side_effect=fake_command):
                argv, env, secrets, jar_digest, version = runner.prepare_pgloader_v4(
                    loader, values, metadata, run_dir / "private")
            load_file = run_dir / "migration.load"
            load_text = load_file.read_text()
            self.assertIn("mysql://my2pg@mysql:3306/source?sslMode=VERIFY_IDENTITY", load_text)
            self.assertIn("postgresql://my2pg@postgres:5432/target?sslmode=verify-full&sslrootcert=/tls/ca.pem", load_text)
            self.assertIn("INCLUDING ONLY TABLE NAMES MATCHING ~/^t21_large_00$/, ~/^t21_large_01$/", load_text)
            self.assertIn("batch rows = 12345", load_text)
            self.assertIn("batch size = 8388608", load_text)
            self.assertIn("prefetch rows = 3", load_text)
            self.assertNotIn("mysql-secret", load_text)
            self.assertNotIn("postgres-secret", load_text)
            self.assertEqual(load_file.stat().st_mode & 0o777, 0o600)
            self.assertEqual((run_dir / "private/home/.my.cnf").stat().st_mode & 0o777, 0o600)
            self.assertEqual((run_dir / "private/.pgpass").stat().st_mode & 0o777, 0o600)
            self.assertEqual((run_dir / "private/.pgpass").read_bytes(),
                             b"postgres:5432:target:my2pg:postgres-secret\n")
            env_text = env.read_text()
            self.assertIn("PGPASSFILE=/bench/private/.pgpass", env_text)
            self.assertIn("JAVA_TOOL_OPTIONS=-Xmx384m ", env_text)
            self.assertIn("trustStore=/bench/private/fixture-truststore.p12", env_text)
            self.assertNotIn("mysql-secret", env_text)
            self.assertNotIn("postgres-secret", env_text)
            self.assertNotIn("MY2PG_MYSQL_URL", env_text)
            self.assertIn("mysql-secret", secrets)
            self.assertIn("postgres-secret", secrets)
            self.assertEqual(jar_digest, hashlib.sha256(b"truthful pinned jar").hexdigest())
            self.assertEqual(version, "pgloader v4.0.0")
            self.assertEqual(argv, ["java", "-jar", "/app/pgloader.jar", "--summary",
                                    "/bench/pgloader-summary.json", "/bench/migration.load"])

            bad_values = {
                "run_dir": str(run_dir),
                "target_schema": "t21_bad_rows_test",
                "workload": "bad_rows_fixed_rate",
                **runner.workload_table_values("bad_rows_fixed_rate"),
            }
            bad_values["table"] = "t21_bad_rows"
            with patch.object(runner.harness, "command", side_effect=fake_command):
                bad_argv, *_ = runner.prepare_pgloader_v4(
                    loader, bad_values, metadata, run_dir / "private-bad")
            self.assertIn("CAST column t21_bad_rows.payload to text", load_file.read_text())
            self.assertEqual(bad_argv[-5:], ["--root-dir", "/tmp/pgloader", "--summary",
                                             "/bench/pgloader-summary.json", "/bench/migration.load"])
            _, bad_container_argv = runner.limited_container_argv(
                loader["identity"]["image"], "01-pgloader-v4-pinned-correctness-00", bad_argv,
                run_dir, run_dir / "private-bad/loader.env", ca_file, metadata,
                loader["resource_limits"], extra_mounts=[(run_dir / "rejects", "/tmp/pgloader")])
            self.assertIn(f"type=bind,src={(run_dir / 'rejects').resolve()},dst=/tmp/pgloader",
                          " ".join(bad_container_argv))

            binary_values = {
                "run_dir": str(run_dir),
                "target_schema": "t21_binary_test",
                "workload": "binary_large_rows",
                **runner.workload_table_values("binary_large_rows"),
            }
            binary_values["table"] = "t21_binary_rows"
            with patch.object(runner.harness, "command", side_effect=fake_command):
                runner.prepare_pgloader_v4(
                    loader, binary_values, metadata, run_dir / "private-binary")
            load_text = load_file.read_text()
            self.assertIn("CAST column t21_binary_rows.payload to bytea using hex-to-bytea,", load_text)
            self.assertIn("column t21_binary_rows.optional_bytes to bytea using hex-to-bytea", load_text)

    def test_pgloader_adapter_rejects_non_loopback_fixture_endpoints(self):
        with self.assertRaisesRegex(ValueError, "loopback host"):
            runner._loopback_endpoint("mysql://user:secret@mysql:3306/source", "mysql", "source")

    def test_comparison_manifest_rejects_nonmatching_resource_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = {"source": "mysql", "target": "postgresql", "mode": "full",
                        "source_consistency": "frozen", "source_tls": "verify_full", "target_tls": "verify_full",
                        "fresh_target_schema": True, "table_workers": 1, "readers_per_table": 1,
                        "index_workers": 1, "row_error_policy": "stop", "durability": "defaults",
                        "indexes": "primary", "verification": "external"}
            manifest = {"format": 1, "comparison_contract": settings,
                        "resource_limits": {"cpu_cores": 1, "memory_bytes": 512}, "loaders": []}
            for name in ["one", "two"]:
                manifest["loaders"].append({"name": name, "kind": "my2pg-container", "identity": {"kind": "test"},
                    "resource_limits": {"cpu_cores": 1 if name == "one" else 2, "memory_bytes": 512},
                    "resource_enforcement": "docker-container", "settings": settings, "argv": ["test"],
                    "config_template": ""})
            path = root / "loaders.json"
            path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "resource_limits differ"):
                runner.load_manifest(path)

    def test_database_resource_pins_allow_role_specific_memory_limits(self):
        containers = [
            {"Name": "/mysql", "Image": "mysql-image", "Config": {"Labels": {"com.docker.compose.service": "mysql"}},
             "HostConfig": {"Memory": 1024 ** 3, "NanoCpus": 1_000_000_000, "CpuQuota": 0, "CpuPeriod": 0}},
            {"Name": "/postgres", "Image": "postgres-image", "Config": {"Labels": {"com.docker.compose.service": "postgres"}},
             "HostConfig": {"Memory": 512 * 1024 ** 2, "NanoCpus": 0, "CpuQuota": 100_000, "CpuPeriod": 100_000}},
        ]
        metadata = {"project": "my2pg-resource-test"}
        with patch.object(runner.harness, "validate_owned", return_value=["mysql-id", "postgres-id"]), \
                patch.object(runner.harness, "command", side_effect=lambda args: subprocess.CompletedProcess(
                    args, 0, json.dumps([containers[0 if args[-1] == "mysql-id" else 1]]), "")):
            pins = runner.resource_pins(metadata)

        self.assertTrue(pins["limits_declared"])
        self.assertFalse(pins["limits_equal"])
        self.assertEqual([item["memory_bytes"] for item in pins["containers"]], [1024 ** 3, 512 * 1024 ** 2])

    def test_database_resource_pins_reject_any_unlimited_container(self):
        containers = [
            {"Name": "/mysql", "Image": "mysql-image", "Config": {"Labels": {}},
             "HostConfig": {"Memory": 1024 ** 3, "NanoCpus": 0, "CpuQuota": 0, "CpuPeriod": 0}},
            {"Name": "/postgres", "Image": "postgres-image", "Config": {"Labels": {}},
             "HostConfig": {"Memory": 0, "NanoCpus": 1_000_000_000, "CpuQuota": 0, "CpuPeriod": 0}},
        ]
        metadata = {"project": "my2pg-resource-test"}
        with patch.object(runner.harness, "validate_owned", return_value=["mysql-id", "postgres-id"]), \
                patch.object(runner.harness, "command", side_effect=lambda args: subprocess.CompletedProcess(
                    args, 0, json.dumps([containers[0 if args[-1] == "mysql-id" else 1]]), "")):
            pins = runner.resource_pins(metadata)

        self.assertFalse(pins["limits_declared"])

    def test_pgloader_phase_metrics_fail_closed_without_instrumented_summary(self):
        metrics = runner.phase_metrics("", Path("missing-report.json"), "pgloader-v4")
        self.assertEqual(metrics["status"], "unsupported")
        self.assertIn("phase summary could not be read", metrics["reason"])

    def test_pgloader_summary_metrics_parse_source_native_totals_without_normalizing_phases(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            summary.write_text(json.dumps({
                "grand-total": {"total-nanos": 7_000_000_000, "rows": 42, "bytes": 4096},
                "phases": {
                    "pre": {"total": {"total-nanos": 1_000_000_000}},
                    "data": {"total": {"total-nanos": 4_000_000_000}},
                    "post": {"total": {"total-nanos": 2_000_000_000}, "tables": [
                        {"label": "COPY Wall-Clock Time", "total-time": 3_500_000_000},
                    ]},
                },
            }))
            metrics = runner.pgloader_summary_metrics(summary)
            self.assertEqual(metrics, {
                "status": "observed",
                "phase_totals_nanos": {"pre": 1_000_000_000, "data": 4_000_000_000,
                                        "post": 2_000_000_000},
                "loader_elapsed_nanos": 7_000_000_000,
                "rows": 42,
                "bytes": 4096,
                "copy_wall_nanos": 3_500_000_000,
            })

    def test_pgloader_summary_metrics_fail_closed_on_missing_or_malformed_copy_entry(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            self.assertEqual(runner.pgloader_summary_metrics(summary)["status"], "unsupported")
            summary.write_text(json.dumps({
                "grand-total": {"total-nanos": 1, "rows": 1, "bytes": 1},
                "phases": {
                    "pre": {"total": {"total-nanos": 0}},
                    "data": {"total": {"total-nanos": 0}},
                    "post": {"total": {"total-nanos": 0}, "tables": [
                        {"label": "COPY Wall-Clock Time", "total-time": -1},
                    ]},
                },
            }))
            malformed = runner.pgloader_summary_metrics(summary)
            self.assertEqual(malformed["status"], "unsupported")
            self.assertIn("malformed or negative", malformed["reason"])

    def test_pgloader_summary_metrics_reject_malformed_json(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            summary.write_text("{not valid json")

            metrics = runner.pgloader_summary_metrics(summary)

            self.assertEqual(metrics["status"], "unsupported")
            self.assertIn("could not be read", metrics["reason"])

    def test_pgloader_summary_metrics_reject_invalid_utf8(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            summary.write_bytes(b"\xff")

            metrics = runner.pgloader_summary_metrics(summary)

            self.assertEqual(metrics["status"], "unsupported")
            self.assertIn("could not be read", metrics["reason"])

    def test_pgloader_summary_metrics_reject_missing_phase_total(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            summary.write_text(json.dumps({
                "grand-total": {"total-nanos": 7, "rows": 42, "bytes": 4096},
                "phases": {
                    "pre": {"total": {"total-nanos": 1}},
                    "data": {"total": {}},
                    "post": {"total": {"total-nanos": 2}, "tables": [
                        {"label": "COPY Wall-Clock Time", "total-time": 3},
                    ]},
                },
            }))

            metrics = runner.pgloader_summary_metrics(summary)

            self.assertEqual(metrics["status"], "unsupported")
            self.assertIn("phase totals are missing", metrics["reason"])

    def test_pgloader_summary_metrics_reject_duplicate_copy_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            summary.write_text(json.dumps({
                "grand-total": {"total-nanos": 7, "rows": 42, "bytes": 4096},
                "phases": {
                    "pre": {"total": {"total-nanos": 1}},
                    "data": {"total": {"total-nanos": 4}},
                    "post": {"total": {"total-nanos": 2}, "tables": [
                        {"label": "COPY Wall-Clock Time", "total-time": 3},
                        {"label": "COPY Wall-Clock Time", "total-time": 3},
                    ]},
                },
            }))

            metrics = runner.pgloader_summary_metrics(summary)

            self.assertEqual(metrics["status"], "unsupported")
            self.assertIn("exactly one COPY Wall-Clock Time", metrics["reason"])

    def test_pgloader_phase_metrics_normalize_instrumented_summary_intervals(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            intervals = [
                {"kind": "phase_interval", "phase": "catalog", "clock": "run_monotonic_millis",
                 "scope": "run", "start_elapsed_millis": 10, "end_elapsed_millis": 80,
                 "operation_classes": sorted(runner.PHASE_OPERATION_CLASSES["catalog"])},
                {"kind": "phase_interval", "phase": "copy", "clock": "run_monotonic_millis",
                 "scope": "run", "start_elapsed_millis": 80, "end_elapsed_millis": 200,
                 "operation_classes": sorted(runner.PHASE_OPERATION_CLASSES["copy"])},
                {"kind": "phase_interval", "phase": "index_constraints", "clock": "run_monotonic_millis",
                 "scope": "run", "start_elapsed_millis": 150, "end_elapsed_millis": 220,
                 "operation_classes": sorted(runner.PHASE_OPERATION_CLASSES["index_constraints"])},
            ]
            summary.write_text(json.dumps({"benchmark_phases": intervals}))

            metrics = runner.phase_metrics("", Path("missing-report.json"), "pgloader-v4", 30, summary)

            self.assertEqual(metrics["status"], "supported")
            self.assertEqual(metrics["phase_seconds"], {
                "catalog": 0.07, "copy": 0.12, "index_constraints": 0.07, "verification": 0.03,
            })
            self.assertLess(metrics["phase_intervals"][1]["start_elapsed_millis"],
                            metrics["phase_intervals"][2]["end_elapsed_millis"])

    def test_pgloader_phase_metrics_reject_missing_or_malformed_instrumented_summary(self):
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary.json"
            report = Path(directory) / "report.json"
            report.write_text("{}")
            summary.write_text(json.dumps({"benchmark_phases": "not-an-array"}))

            metrics = runner.phase_metrics("", report, "pgloader-v4", 10, summary)

            self.assertEqual(metrics["status"], "unsupported")
            self.assertIn("not an array", metrics["reason"])
            self.assertEqual(runner.phase_metrics("", report, "pgloader-v4", 10,
                                                  Path(directory) / "missing.json")["status"],
                             "unsupported")

    def test_phase_metrics_require_explicit_complete_run_wide_intervals(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            report.write_text(json.dumps({"elapsed_millis": 5000, "tables": []}))
            legacy = "\n".join([
                json.dumps({"kind": "phase", "phase": "finalize", "elapsed_millis": 3000}),
                json.dumps({"kind": "phase", "phase": "verify", "elapsed_millis": 4000}),
            ])
            self.assertEqual(runner.phase_metrics(legacy, report)["status"], "unsupported")

            def event(phase, start, end, scope="run", operations=None):
                return {
                    "kind": "phase_interval", "phase": phase,
                    "clock": "run_monotonic_millis", "scope": scope,
                    "start_elapsed_millis": start, "end_elapsed_millis": end,
                    "operation_classes": sorted(operations or runner.PHASE_OPERATION_CLASSES[phase]),
                }

            valid = "\n".join(json.dumps(item) for item in [
                event("catalog", 0, 200),
                event("copy", 200, 5000),
                event("index_constraints", 400, 5100),
                event("verification", 5100, 5300),
            ])
            supported = runner.phase_metrics(valid, report)
            self.assertEqual(supported["status"], "supported")
            self.assertEqual(supported["phase_seconds"], {
                "catalog": 0.2, "copy": 4.8, "index_constraints": 4.7, "verification": 0.2,
            })
            self.assertTrue(runner.phases_are_comparable([{"phase_metrics": supported}]))

            cases = [
                ([event("catalog", 0, 200)], "incomplete"),
                ([event("catalog", 0, 200), event("catalog", 0, 201)], "duplicate"),
                ([event("catalog", -1, 200)], "negative"),
                ([event("catalog", 201, 200)], "invalid"),
                ([event("catalog", 0, 200, scope="table")], "run-wide"),
                ([event("catalog", 0, 200, operations=["unknown"])], "operation classes"),
            ]
            for events, reason in cases:
                with self.subTest(reason=reason):
                    parsed = runner.phase_metrics("\n".join(json.dumps(item) for item in events), report)
                    self.assertEqual(parsed["status"], "unsupported")
                    self.assertIn(reason, parsed["reason"])

    def test_phase_metrics_add_runner_oracle_after_migration_intervals(self):
        migration_intervals = "\n".join(json.dumps({
            "kind": "phase_interval", "phase": phase,
            "clock": "run_monotonic_millis", "scope": "run",
            "start_elapsed_millis": start, "end_elapsed_millis": end,
            "operation_classes": sorted(runner.PHASE_OPERATION_CLASSES[phase]),
        }) for phase, start, end in [
            ("catalog", 0, 20), ("copy", 20, 100), ("index_constraints", 90, 110),
        ])
        metrics = runner.phase_metrics(migration_intervals, Path("missing-report.json"),
                                       verification_elapsed_millis=35)
        self.assertEqual(metrics["status"], "supported")
        self.assertEqual(metrics["phase_seconds"]["verification"], 0.035)
        verification = next(event for event in metrics["phase_intervals"]
                            if event["phase"] == "verification")
        self.assertEqual(verification["start_elapsed_millis"], 110)
        self.assertEqual(verification["end_elapsed_millis"], 145)
        self.assertIn("positioned after migration intervals", verification["normalization"])

    def test_phase_comparison_fails_closed_on_missing_empty_or_mismatched_sets(self):
        self.assertFalse(runner.phases_are_comparable([]))
        self.assertFalse(runner.phases_are_comparable([{"phase_metrics": {}}]))
        self.assertFalse(runner.phases_are_comparable([
            {"phase_metrics": {"status": "supported", "phase_seconds": {}}},
        ]))
        self.assertFalse(runner.phases_are_comparable([
            {"phase_metrics": {"status": "supported", "phase_seconds": {"copy": 1.0}}},
            {"phase_metrics": {"status": "unsupported", "phase_seconds": {}}},
        ]))
        self.assertFalse(runner.phases_are_comparable([
            {"phase_metrics": {"status": "supported", "phase_seconds": {"copy": 1.0}}},
            {"phase_metrics": {"status": "supported", "phase_seconds": {"copy": 1.0}}},
        ]))
        complete_phases = {phase: 1.0 for phase in runner.REQUIRED_BENCHMARK_PHASES}
        operation_classes = {
            phase: sorted(items) for phase, items in runner.PHASE_OPERATION_CLASSES.items()
        }
        self.assertTrue(runner.phases_are_comparable([
            {"phase_metrics": {"status": "supported", "phase_seconds": complete_phases,
                                "phase_operation_classes": operation_classes}},
            {"phase_metrics": {"status": "supported", "phase_seconds": dict(complete_phases),
                                "phase_operation_classes": dict(operation_classes)}},
        ]))
        mismatched_scopes = dict(operation_classes)
        mismatched_scopes["catalog"] = ["source_mysql_catalog"]
        self.assertFalse(runner.phases_are_comparable([
            {"phase_metrics": {"status": "supported", "phase_seconds": complete_phases,
                                "phase_operation_classes": operation_classes}},
            {"phase_metrics": {"status": "supported", "phase_seconds": complete_phases,
                                "phase_operation_classes": mismatched_scopes}},
        ]))

    def test_phase_metrics_exclude_table_scoped_copy_time_from_global_timeline(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            report.write_text(json.dumps({"elapsed_millis": 5000, "tables": [
                {"source_name": "users", "copy_elapsed_millis": 250},
            ]}))
            log = "\n".join([
                json.dumps({"kind": "phase", "phase": "copy", "table": "users",
                            "elapsed_millis": 250}),
                json.dumps({"kind": "phase", "phase": "finalize", "elapsed_millis": 3000}),
                json.dumps({"kind": "phase", "phase": "verify", "elapsed_millis": 4000}),
            ])

            metrics = runner.phase_metrics(log, report)

            self.assertEqual(metrics["status"], "unsupported")
            self.assertNotIn("copy", metrics["phase_seconds"])
            self.assertEqual(metrics["phase_seconds"], {})
            self.assertEqual(metrics["copy_elapsed_millis_by_table"], {"users": 250})
            self.assertFalse(runner.phases_are_comparable([{"phase_metrics": metrics}]))

    def test_docker_service_url_moves_owned_loopback_credentials_to_service_dns(self):
        actual = runner.docker_service_url(
            "mysql://my2pg:mysql-secret@127.0.0.1:43101/source?ssl-mode=VERIFY_IDENTITY",
            "mysql", "source", "mysql", 3306,
        )
        self.assertEqual(actual, "mysql://my2pg:mysql-secret@mysql:3306/source?ssl-mode=VERIFY_IDENTITY")

    def test_limited_container_argv_enforces_manifest_cpu_memory_and_swap_without_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            name, argv = runner.limited_container_argv(
                "my2pg:test", "01-my2pg-release-warmup-00", ["my2pg", "run", "/bench/migration.toml"],
                root, root / "loader.env", root / "ca.pem", {"project": "my2pg-test"},
                {"cpu_cores": 1, "memory_bytes": 536870912},
                extra_mounts=[(root, "/tmp/pgloader")],
            )
        self.assertEqual(name, "my2pg-t21-01-my2pg-release-warmup-00")
        for pair in [("--cpus", "1"), ("--memory", "536870912"), ("--memory-swap", "536870912")]:
            self.assertIn(pair[0], argv)
            self.assertEqual(argv[argv.index(pair[0]) + 1], pair[1])
        self.assertEqual(argv[argv.index("--network") + 1], "my2pg-test_default")
        self.assertIn("/tls/ca.pem,readonly", " ".join(argv))
        self.assertIn(f"type=bind,src={root.resolve()},dst=/tmp/pgloader", " ".join(argv))
        self.assertNotIn("mysql-secret", " ".join(argv))

    def test_container_inspection_verifies_actual_image_and_cgroup_limits(self):
        item = {"Image": "image-id", "HostConfig": {"Memory": 536870912, "MemorySwap": 536870912,
                "NanoCpus": 1000000000, "CpuQuota": 0, "CpuPeriod": 0},
                "State": {"ExitCode": 0, "OOMKilled": False}}
        result = subprocess.CompletedProcess(["docker", "inspect", "container"], 0, json.dumps([item]), "")
        with patch.object(runner.harness, "command", return_value=result):
            observed = runner.inspect_container_limits(
                "my2pg-t21-01-my2pg-release-warmup-00", "image-id",
                {"cpu_cores": 1, "memory_bytes": 536870912},
            )
        self.assertTrue(observed["verified"])
        item["HostConfig"]["Memory"] = 1024
        changed = subprocess.CompletedProcess(["docker", "inspect", "container"], 0, json.dumps([item]), "")
        with patch.object(runner.harness, "command", return_value=changed):
            observed = runner.inspect_container_limits(
                "my2pg-t21-01-my2pg-release-warmup-00", "image-id",
                {"cpu_cores": 1, "memory_bytes": 536870912},
            )
        self.assertFalse(observed["verified"])

    def test_limited_container_runner_inspects_and_removes_its_named_container(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            completed = {"exit_code": 0, "wall_seconds": 1.0, "container_resource_samples": {}}
            verified = {"verified": True, "image_id": "image-id", "memory_bytes": 512}
            removed = subprocess.CompletedProcess(["docker", "rm"], 0, "container-id", "")
            with patch.object(runner, "run_command", return_value=completed) as execute, \
                    patch.object(runner, "inspect_container_limits", return_value=verified) as inspect, \
                    patch.object(runner.harness, "command", return_value=removed) as cleanup:
                result, argv = runner.run_limited_container(
                    "image", "image-id", "01-my2pg-release-warmup-00", ["my2pg", "run", "/bench/migration.toml"],
                    root, root / "loader.env", root / "ca.pem", {"project": "my2pg-test"},
                    {"cpu_cores": 1, "memory_bytes": 512}, root / "loader.log")
        execute.assert_called_once()
        self.assertTrue(any("--memory-swap" in part for part in argv))
        inspect.assert_called_once_with("my2pg-t21-01-my2pg-release-warmup-00", "image-id",
                                        {"cpu_cores": 1, "memory_bytes": 512})
        cleanup.assert_called_once_with(["docker", "rm", "--force", "my2pg-t21-01-my2pg-release-warmup-00"], check=False)
        self.assertTrue(result["container_resource_limits"]["verified"])

    def test_container_image_build_records_image_and_binary_identity(self):
        image_id = subprocess.CompletedProcess(["docker", "inspect"], 0, json.dumps([{"Id": "sha256:image"}]), "")
        binary_hash = subprocess.CompletedProcess(["docker", "run"], 0, "a" * 64 + "  /usr/local/bin/my2pg\n", "")
        with patch.object(runner, "command_output", return_value="revision"), \
                patch.object(runner.harness, "command", side_effect=[subprocess.CompletedProcess([], 0, "", ""), image_id, binary_hash]):
            image = runner.build_my2pg_benchmark_image()
        self.assertEqual(image["image_id"], "sha256:image")
        self.assertEqual(image["binary_sha256"], "a" * 64)
        self.assertEqual(image["source_revision"], "revision")

    def test_pgloader_image_builder_applies_patch_in_temporary_source_copy(self):
        manifest = runner.load_manifest(RUNNER_PATH.with_name("loaders.comparison.example.json"))
        loader = next(item for item in manifest["loaders"] if item.get("kind") == "pgloader-v4")
        real_command = runner.harness.command

        def fake_docker(args, check=True):
            if args[0] == "git":
                return real_command(args, check=check)
            if args[:2] == ["docker", "build"]:
                return subprocess.CompletedProcess(args, 0, "", "")
            if args[:3] == ["docker", "image", "inspect"]:
                return subprocess.CompletedProcess(args, 0, json.dumps([{"Id": "sha256:instrumented"}]), "")
            raise AssertionError(f"unexpected Docker command: {args}")

        with patch.object(runner.harness, "command", side_effect=fake_docker):
            image = runner.build_pgloader_t21_image(loader)

        self.assertEqual(image["source_revision"], loader["identity"]["revision"])
        self.assertEqual(image["source_image_id"], loader["identity"]["image_id"])
        self.assertEqual(image["image_id"], "sha256:instrumented")
        self.assertEqual(len(image["instrumentation_patch_sha256"]), 64)

    def test_comparison_manifest_loads_with_pinned_v4_and_resource_gaps(self):
        manifest = runner.load_manifest(RUNNER_PATH.with_name("loaders.comparison.example.json"))
        pgloader = next(loader for loader in manifest["loaders"] if loader.get("kind") == "pgloader-v4")
        self.assertEqual(pgloader["identity"]["revision"], "231ab86778ca5ffd7de40878714760c8b4860cdf")
        self.assertEqual(pgloader["resource_enforcement"], "docker-container")
        self.assertTrue(pgloader["unsupported_equivalence"])
        self.assertTrue(any("randomizes batch row capacity" in gap for gap in pgloader["unsupported_equivalence"]))
        self.assertTrue(any("single row may exceed" in gap for gap in pgloader["unsupported_equivalence"]))
        self.assertNotIn("batch_bytes", pgloader["unsupported_equivalence"])
        self.assertNotIn("queue_batches", pgloader["unsupported_equivalence"])

    def test_pgloader_manifest_rejects_invalid_batch_controls(self):
        source = RUNNER_PATH.with_name("loaders.comparison.example.json").read_text()
        original = json.loads(source)
        cases = [
            ("batch_rows", 0),
            ("batch_rows", 2**31),
            ("batch_bytes", True),
            ("batch_bytes", 2**63),
            ("queue_batches", 0),
            ("queue_batches", 2**31),
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "loaders.json"
            for key, value in cases:
                candidate = json.loads(json.dumps(original))
                candidate["comparison_contract"][key] = value
                for loader in candidate["loaders"]:
                    loader["settings"][key] = value
                path.write_text(json.dumps(candidate))
                with self.subTest(key=key, value=value):
                    with self.assertRaisesRegex(ValueError, f"settings\\.{key}"):
                        runner.load_manifest(path)

    def test_loader_log_is_redacted_before_secret_output_is_written(self):
        with tempfile.TemporaryDirectory() as directory:
            log_path = Path(directory) / "loader.log"
            secret = "fixture-password"
            result = runner.run_command(
                [sys.executable, "-c", f"print('connected {secret} mysql://u:{secret}@127.0.0.1:3306/db')"],
                {}, [], log_path, {secret})
            retained = log_path.read_text()
        self.assertEqual(result["exit_code"], 0)
        self.assertNotIn(secret, retained)
        self.assertNotIn("mysql://", retained)
        self.assertIn("[redacted]", retained)

    def run_main(self, target_override=None, expect_failure=False, workload="one_large_narrow_integer",
                 correctness_only=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact_root = root / "artifacts"
            metadata = {
                "project": "my2pg-unit-test",
                "state": "ready",
                "artifact_dir": str(artifact_root),
                "versions": {"mysql": "test", "postgres": "test"},
                "architecture": "test",
                "env": {"MY2PG_TLS_CA": str(root / "tls" / "ca.pem")},
                "compose_env": {},
            }
            connections = root / "connections.json"
            connections.write_text(json.dumps(metadata))

            contract = {
                "source": "mysql",
                "target": "postgresql",
                "mode": "full",
                "source_consistency": "frozen",
                "source_tls": "verify_full",
                "target_tls": "verify_full",
                "fresh_target_schema": True,
                "table_workers": 1,
                "readers_per_table": 1,
                "index_workers": 1,
                "row_error_policy": "stop",
                "durability": "database defaults",
                "indexes": "primary key",
                "verification": "external oracle",
            }
            manifest = {
                "format": 1,
                "comparison_contract": contract,
                "loaders": [{
                    "name": "bench",
                    "identity": {"kind": "local-binary"},
                    "settings": contract,
                    "argv": ["{my2pg_bin}", "run", "{config_file}"],
                    "config_template": "schema = \"{target_schema}\"\nca_file = \"{source_ca_file}\"\nreport_dir = \"{report_dir}\"\n",
                }],
            }
            loaders = root / "loaders.json"
            loaders.write_text(json.dumps(manifest))
            command_args = [
                "runner.py", str(connections), "--loaders", str(loaders),
                "--my2pg-bin", str(root / "missing-binary"), "--workload", workload,
            ]
            if correctness_only:
                command_args.append("--correctness-only")
            observed = dict(EXPECTED_TARGET)
            expected_source = dict(EXPECTED_SOURCE)
            profile_rows = runner.workload_profile(workload, "smoke")["rows"]
            expected_source["rows"] = runner.workload_row_count(workload, profile_rows)
            observed["rows"] = expected_source["rows"]
            if len(runner.workload_tables(workload)) > 1:
                tables = runner.workload_tables(workload)
                expected_source["aggregate"] = ";".join(f"{table}:same" for table in tables)
                if workload == "many_small_relational_tables":
                    expected_source["target_columns"] = ";".join(
                        f"{table}:id:integer:NO,parent_id:integer:YES,bucket:integer:NO,measure:bigint:NO,payload:bigint:NO"
                        for table in tables
                    )
                else:
                    expected_source["target_columns"] = ";".join(f"{table}:{EXPECTED_COLUMNS}" for table in tables)
                expected_source["primary_key"] = ";".join(f"{table}:id" for table in tables)
                expected_source["table_count"] = len(tables)
                expected_source["foreign_key_count"] = runner.CORPUS["workloads"][workload].get("foreign_key_count", 0)
                expected_source["foreign_keys"] = runner.expected_foreign_keys(workload)
                observed = dict(expected_source)
            if workload == "wide_utf8_text_json_decimal":
                expected_source.update({
                    "aggregate": "100,same,0.1,1,100",
                    "target_columns": "id:bigint:NO,label:character varying:NO,detail:text:NO,json_doc:jsonb:NO,amount:numeric:NO",
                    "primary_key": "id",
                    "table_count": 1,
                    "foreign_key_count": 0,
                    "foreign_keys": "",
                    "payload_bytes": 123456,
                })
                observed = dict(expected_source)
            if workload == "binary_large_rows":
                expected_source.update({
                    "aggregate": "100,5050,1,100",
                    "target_columns": "id:bigint:NO,payload:bytea:NO,optional_bytes:bytea:YES",
                    "primary_key": "id",
                    "table_count": 1,
                    "foreign_key_count": 0,
                    "foreign_keys": "",
                    "payload_bytes": 123456,
                })
                observed = dict(expected_source)
            rejected_ids = []
            if workload == "bad_rows_fixed_rate":
                rejected_ids = runner.expected_bad_row_ids(profile_rows, "1")
                expected_source.update({
                    "rows": profile_rows - len(rejected_ids),
                    "aggregate": "990,495495,1,999",
                    "target_columns": "id:bigint:NO,payload:text:NO",
                    "primary_key": "id",
                    "table_count": 1,
                    "foreign_key_count": 0,
                    "foreign_keys": "",
                    "payload_bytes": 9000,
                })
                rejected_rows = [{"id": row_id, "payload_hex": f"bad-{row_id}\x00-tail".encode().hex()}
                                 for row_id in rejected_ids]
                source_with_rejects = {**expected_source, "rejected_ids": rejected_ids,
                                       "rejected_rows": rejected_rows}
                observed = dict(expected_source)
            else:
                source_with_rejects = expected_source
            if workload == "sparse_skewed_unsigned_primary_keys":
                expected_source["target_columns"] = (
                    "id:numeric:NO,bucket:integer:NO,measure:bigint:NO,payload:bigint:NO"
                )
                observed["target_columns"] = expected_source["target_columns"]
            if target_override:
                observed.update(target_override)
            run_ids = []
            invoked_binaries = []
            invoked_configs = []
            source_oracle_calls = []

            def fake_run_command(argv, env, containers, log_path, secrets=()):
                run_ids.append(log_path.parent.name)
                invoked_binaries.append(argv[0])
                invoked_configs.append((Path(argv[2]).read_text(), str(Path(argv[2]).parent / "report")))
                log_path.write_text("")
                if workload == "bad_rows_fixed_rate":
                    report_dir = log_path.parent / "report"
                    report_dir.mkdir()
                    records = [{"kind": "conversion", "values": [
                        {"kind": "typed", "value": {"kind": "uint", "value": row_id}},
                        {"kind": "bytes", "encoding": "hex",
                         "value": next(row["payload_hex"] for row in rejected_rows
                                       if row["id"] == row_id)},
                    ]} for row_id in rejected_ids]
                    (report_dir / "t_0123456789abcdef.reject.jsonl").write_text(
                        "".join(json.dumps(item) + "\n" for item in records))
                    (report_dir / "report.json").write_text(json.dumps({
                        "elapsed_millis": 100,
                        "tables": [{"rejected_rows": len(rejected_ids)}],
                    }))
                return {
                    "exit_code": 0,
                    "wall_seconds": 0.1,
                    "container_cpu_core_seconds_sampled": 0,
                    "direct_child_peak_rss": {
                        "method": "wait4", "scope": "direct_child", "available": True,
                        "bytes": 32 * 1024 * 1024,
                    },
                    "container_resource_samples": {},
                }

            patches = [
                patch.object(sys, "argv", command_args),
                patch.object(runner.harness, "validate_owned", return_value=["owned-test-container"]),
                patch.object(runner.harness, "sql", return_value=""),
                patch.object(runner, "resource_pins", return_value={"containers": [], "limits_declared": False}),
                patch.object(runner, "source_oracle",
                             side_effect=lambda *_args, **_kwargs:
                                 source_oracle_calls.append(True) or dict(source_with_rejects)),
                patch.object(runner, "target_oracle", side_effect=lambda *_args: dict(observed)),
                patch.object(runner, "command_output", return_value="test"),
                patch.object(runner, "run_command", side_effect=fake_run_command),
                patch("builtins.print"),
            ]
            with patches[0], patches[1], patches[2], patches[3], patches[4], patches[5], patches[6], patches[7], patches[8]:
                if expect_failure:
                    with self.assertRaisesRegex(RuntimeError, "timings are not accepted"):
                        runner.main()
                else:
                    runner.main()

            result_files = list((artifact_root / "t21").glob("*/results.json"))
            self.assertEqual(len(result_files), 1)
            report = json.loads(result_files[0].read_text())
            self.assertEqual([run["run_id"] for run in report["runs"]], run_ids)
            self.assertEqual(len(source_oracle_calls), len(run_ids) + 1)
            self.assertTrue(invoked_binaries)
            self.assertTrue(all(path == str((root / "missing-binary").resolve())
                                for path in invoked_binaries))
            self.assertTrue(all(f'ca_file = "{root / "tls" / "ca.pem"}"' in config
                                for config, _report_dir in invoked_configs))
            self.assertTrue(all(f'report_dir = "{report_dir}"' in config
                                for config, report_dir in invoked_configs))
            self.assertEqual(report["workload"], workload)
            report["_test_source_oracle_calls"] = len(source_oracle_calls)
            return report


if __name__ == "__main__":
    unittest.main()
