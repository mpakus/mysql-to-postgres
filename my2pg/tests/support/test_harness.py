import importlib.util
from pathlib import Path
import tempfile
import json
import sys
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("harness", Path(__file__).with_name("harness.py"))
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


class IsolationTests(unittest.TestCase):
    def test_required_acceptance_ids_follow_actual_platform_cfg(self):
        generic = (
            "t10_enum_metadata::supplementary_enum_set_labels_are_indistinguishable_from_question_marks_in_catalogs",
            "t11_identity_edges::native_runner_round_trips_unsigned_bigint_max_as_numeric",
            "t11_identity_edges::native_runner_refuses_nondefault_source_auto_increment_policy_before_ddl",
            "t11_identity_exhaustion::native_runner_refuses_source_auto_increment_beyond_smallint_sequence_max",
            "t13_snapshot_consistency::actual_run_keeps_the_original_source_snapshot_for_later_tables",
            "t13_external_mdl::cancellation_kills_reader_waiting_on_external_mysql_metadata_lock",
            "t13_snapshot_mdl::cancellation_terminates_single_snapshot_select_waiting_on_external_mdl",
            "t13_snapshot_mdl::cancellation_terminates_single_snapshot_reader_blocked_during_preflight",
            "t11_schema_only_recreate::native_schema_only_recreate_replaces_selected_table_without_copying_or_touching_sentinel",
            "t12_lost_commit_ack::completed_commit_with_dropped_ack_is_indeterminate_and_never_replayed",
        )
        macos = "t12_uncertain_storage::sent_commit_uncertainty_survives_physical_failure_to_publish_new_report"
        for system in ("Linux", "Darwin"):
            required = harness.required_cases(system, "mysql84")
            self.assertEqual(len(required), len(set(required)))
            self.assertTrue(all(case in required for case in generic))
            self.assertEqual(macos in required, system == "Darwin")
            index_case = "t17_index_expression::native_functional_index_requires_override_and_preserves_unique_behavior"
            required_ddl_case = "t12_required_ddl::native_required_check_ddl_failure_persists_failed_report_after_copy"
            self.assertNotIn(index_case, harness.required_cases(system, "mysql57"))
            self.assertIn(index_case, harness.required_cases(system, "mysql80"))
            self.assertIn(index_case, harness.required_cases(system, "mysql84"))
            self.assertIn(required_ddl_case, harness.required_cases(system, "mysql80"))
            self.assertIn(required_ddl_case, harness.required_cases(system, "mysql84"))
            self.assertNotIn(required_ddl_case, harness.required_cases(system, "mysql57"))
            self.assertIn(generic[0], harness.required_cases(system, "mysql57"))
            self.assertIn(generic[1], harness.required_cases(system, "mysql57"))
            no_postgis = harness.MYSQL80_NO_POSTGIS_REQUIRED_CASES[0]
            with_postgis = harness.MYSQL80_POSTGIS_REQUIRED_CASES[0]
            self.assertIn(no_postgis, harness.required_cases(system, "mysql80", "pg16"))
            self.assertNotIn(no_postgis, harness.required_cases(system, "mysql80", "pg16-postgis"))
            self.assertIn(with_postgis, harness.required_cases(system, "mysql80", "pg16-postgis"))
            self.assertNotIn(with_postgis, harness.required_cases(system, "mysql80", "pg16"))
        with patch.object(harness.platform, "system", return_value="Darwin"):
            self.assertEqual(harness.required_cases(), harness.required_cases("Darwin"))

    def test_runtime_missing_ignored_failed_duplicate_cases_fail_on_each_platform(self):
        artifact = {"reason":"compiler-artifact", "manifest_path":str(harness.ROOT / "Cargo.toml"),
                    "target":{"name":"my2pg","kind":["bin"]}, "profile":{"test":False}, "executable":sys.executable}
        for system in ("Linux", "Darwin"):
            required = harness.required_cases(system, "mysql80")
            plan = harness.mysql_case_plan("mysql80", list(harness.MYSQL_CASE_INVENTORY))
            runnable = harness.runnable_mysql_case_ids(plan, system)
            nonrequired = next(case for case in runnable if case not in required)
            for condition in ("ok", "missing", "ignored", "FAILED", "duplicate"):
                with self.subTest(system=system, condition=condition), tempfile.TemporaryDirectory() as directory:
                    cases = [f"test {case} ... ok\n" for case in runnable]
                    if condition == "missing":
                        cases = [line for line in cases if not line.startswith(f"test {nonrequired} ...")]
                    elif condition in ("ignored", "FAILED"):
                        cases = [f"test {nonrequired} ... {condition}\n"
                                 if line.startswith(f"test {nonrequired} ...") else line
                                 for line in cases]
                    elif condition == "duplicate":
                        cases.append(f"test {nonrequired} ... ok\n")
                    output = "".join(cases) + "test result: ok. 100 passed; 0 failed; 0 ignored;\n"
                    def command(args, **kwargs):
                        stdout = json.dumps(artifact) if "--no-run" in args else output if "--tests" in args else ""
                        return type("Result", (), {"stdout":stdout,"stderr":"","returncode":0})()
                    metadata = {"artifact_dir":directory, "env":{}, "lanes":["mysql80", "pg16"], "fixture_sha256":{}, "harness_sha256":"mock"}
                    with patch.object(harness, "command", side_effect=command), \
                            patch.object(harness, "start", return_value=metadata), \
                            patch.object(harness, "stop") as stop, \
                            patch.object(harness.platform, "system", return_value=system), \
                            patch.object(sys, "argv", ["harness.py"]), \
                            patch("builtins.print"):
                        if condition == "ok":
                            harness.main()
                        else:
                            with self.assertRaisesRegex(RuntimeError, "runnable native cases"):
                                harness.main()
                    evidence = json.loads(Path(directory,"rust-tests.json").read_text())
                    self.assertEqual(set(evidence["required_case_results"]), set(required))
                    self.assertEqual(set(evidence["runnable_case_results"]), set(runnable))
                    self.assertEqual(evidence["host"], system)
                    unavailable = harness.platform_unavailable_case_reasons(system)
                    self.assertEqual(evidence["platform_unavailable_case_reasons"], unavailable)
                    self.assertEqual(evidence["unmet_required_case_ids"], [])
                    self.assertEqual(evidence["unmet_runnable_case_ids"], [] if condition == "ok" else [nonrequired])
                    stop.assert_called_once_with(metadata)

    def test_failed_case_survives_interrupted_nocapture_status(self):
        output = "test binary_default::round_trip ... synthetic source output\nFAILED\n\nfailures:\n\nfailures:\n    binary_default::round_trip\n\ntest result: FAILED. 75 passed; 1 failed;\n"
        self.assertEqual(harness.failed_cases(output), ["binary_default::round_trip"])
        self.assertEqual(harness.failed_cases("test same::case ... FAILED\n"), ["same::case"])
        self.assertEqual(harness.failed_cases("test same::case ... ok\n"), [])

    def test_cleanup_refuses_foreign_project_before_docker_call(self):
        with patch.object(harness, "compose") as docker:
            with self.assertRaisesRegex(ValueError, "non-my2pg"):
                harness.validate_owned({"project": "strangler"})
            docker.assert_not_called()

    def test_cleanup_refuses_missing_owner_label(self):
        fake_ps = type("Result", (), {"stdout": "container"})()
        fake_inspect = type("Result", (), {"stdout": '[{"Config":{"Labels":{"com.docker.compose.project":"my2pg-test"}}}]'})()
        with patch.object(harness, "command", side_effect=[fake_ps, fake_inspect]):
            with self.assertRaisesRegex(ValueError, "ownership label"):
                harness.validate_owned({"project": "my2pg-test"})

    def test_unknown_requested_lane_is_failure_before_startup(self):
        with patch.object(harness, "command") as process:
            with self.assertRaisesRegex(ValueError, "no reviewed"):
                harness.start("mysql99", "pg16")
            process.assert_not_called()

    def test_seed_mismatch_is_failure(self):
        with tempfile.TemporaryDirectory() as path, patch.object(harness, "sql", return_value="2"):
            with self.assertRaises(AssertionError):
                harness.smoke({"artifact_dir": path, "lanes": ["mysql84", "pg16"]})

    def test_mysql_case_manifest_is_exact_and_5_7_static_dispositions_are_complete(self):
        cases = list(harness.MYSQL_CASE_INVENTORY)
        self.assertEqual(len(cases), len(set(cases)))
        self.assertNotIn(
            "t12_localized::actual_french_builtin_check_error_keeps_sqlstate_recovery_and_durable_reject",
            cases,
            "the separately feature-gated PostgreSQL NLS test is not a MySQL-version case",
        )
        for lane in ("mysql57", "mysql80", "mysql84"):
            self.assertEqual(set(harness.MYSQL_CASE_MANIFEST[lane]), set(cases))
        self.assertEqual(set(harness.MYSQL57_NOT_APPLICABLE), {
            case for case, (status, _) in harness.MYSQL_CASE_MANIFEST["mysql57"].items()
            if status == "not_applicable"
        })
        self.assertTrue(all(reason for reason in harness.MYSQL57_NOT_APPLICABLE.values()))
        self.assertEqual(set(harness.MYSQL57_NEEDS_RUNTIME), {
            case for case, (status, _) in harness.MYSQL_CASE_MANIFEST["mysql57"].items()
            if status == "pending"
        })
        self.assertTrue(all(reason for reason in harness.MYSQL57_NEEDS_RUNTIME.values()))
        plan = harness.mysql_case_plan("mysql57", cases)
        self.assertEqual(set(harness.unresolved_mysql_cases(plan)), set(harness.MYSQL57_NEEDS_RUNTIME))
        self.assertEqual(sum(status == "run" for status, _ in plan.values()), 133)
        self.assertEqual(len(harness.MYSQL57_NOT_APPLICABLE), 23)
        self.assertEqual(harness.MYSQL57_NEEDS_RUNTIME, {})
        self.assertEqual(harness.unresolved_mysql_cases(plan), [])
        self.assertEqual(len(harness.runnable_mysql_case_ids(plan, "Linux")), 129)
        self.assertEqual(len(harness.runnable_mysql_case_ids(plan, "Darwin")), 133)
        self.assertEqual(set(harness.platform_unavailable_case_reasons("Linux")), set(harness.MACOS_ONLY_CASES))
        self.assertEqual(harness.platform_unavailable_case_reasons("Darwin"), {})
        self.assertTrue(all(status == "run" for status, _ in harness.mysql_case_plan("mysql80", cases).values()))
        self.assertTrue(all(status == "run" for status, _ in harness.mysql_case_plan("mysql84", cases).values()))
        with self.assertRaisesRegex(ValueError, "unknown discovered"):
            harness.mysql_case_plan("mysql57", cases + ["new::unreviewed_case"])
        with self.assertRaisesRegex(ValueError, "duplicate discovered"):
            harness.mysql_case_plan("mysql57", cases + [cases[0]])
        removed = cases[0]
        del harness.MYSQL_CASE_MANIFEST["mysql80"][removed]
        with self.assertRaisesRegex(ValueError, "missing MySQL case dispositions"):
            harness.mysql_case_plan("mysql80", [removed])
        harness.MYSQL_CASE_MANIFEST["mysql80"][removed] = ("run", None)

    def test_case_manifest_rejects_unknown_status_and_reasonless_non_applicability(self):
        case = next(iter(harness.MYSQL_CASE_INVENTORY))
        original = harness.MYSQL_CASE_MANIFEST["mysql57"][case]
        try:
            harness.MYSQL_CASE_MANIFEST["mysql57"][case] = ("skip", "old broad skip")
            with self.assertRaisesRegex(ValueError, "unknown MySQL case disposition"):
                harness.mysql_case_plan("mysql57", [case])
            harness.MYSQL_CASE_MANIFEST["mysql57"][case] = ("not_applicable", " ")
            with self.assertRaisesRegex(ValueError, "concrete source feature/version reason"):
                harness.mysql_case_plan("mysql57", [case])
            harness.MYSQL_CASE_MANIFEST["mysql57"][case] = ("pending", " ")
            with self.assertRaisesRegex(ValueError, "concrete unresolved version evidence"):
                harness.mysql_case_plan("mysql57", [case])
        finally:
            harness.MYSQL_CASE_MANIFEST["mysql57"][case] = original

    def test_case_skip_selectors_are_exact_and_only_exclude_reviewed_features(self):
        plan = harness.mysql_case_plan("mysql57", list(harness.MYSQL57_NOT_APPLICABLE))
        self.assertEqual(set(harness.mysql_case_skip_selectors(plan)), set(harness.MYSQL57_NOT_APPLICABLE))
        with self.assertRaisesRegex(ValueError, "unknown discovered"):
            harness.mysql_case_plan("mysql57", ["t17_index_expression::unknown_case"])

    def test_platform_keyed_pin_selection_fails_closed(self):
        arm = {"image": "postgres@sha256:" + "a" * 64, "kind": "postgres",
               "version_prefix": "16.", "platform": "linux/arm64",
               "tag_at_pin": "postgres:16-alpine", "pin_date": "2026-10-02"}
        amd = {**arm, "image": "postgres@sha256:" + "b" * 64, "platform": "linux/amd64"}
        pins = {"pg16": {"platforms": {"linux/arm64": arm, "linux/amd64": amd}}}
        self.assertEqual(harness.select_pin(pins, "pg16", "postgres", "linux/amd64"), amd)
        self.assertEqual(harness.select_pin(pins, "pg16", "postgres", "linux/arm64"), arm)
        with self.assertRaisesRegex(ValueError, "no reviewed linux/s390x"):
            harness.select_pin(pins, "pg16", "postgres", "linux/s390x")
        with patch.object(harness.platform, "machine", return_value="x86_64"), \
                patch.object(harness.platform, "system", return_value="Linux"):
            self.assertEqual(harness.native_platform(), "linux/amd64")
        with patch.object(harness.platform, "machine", return_value="aarch64"), \
                patch.object(harness.platform, "system", return_value="Darwin"):
            self.assertEqual(harness.native_platform(), "linux/arm64")

    def test_mysql57_pending_contract_still_fails_closed_before_fixture_start(self):
        case = next(iter(harness.MYSQL_CASE_INVENTORY))
        original = harness.MYSQL_CASE_MANIFEST["mysql57"][case]
        try:
            harness.MYSQL_CASE_MANIFEST["mysql57"][case] = ("pending", "synthetic unresolved behavior")
            with patch.object(harness, "start") as start, \
                    patch.object(sys, "argv", ["harness.py", "mysql57", "pg16"]):
                with self.assertRaisesRegex(RuntimeError, "pending test applicability"):
                    harness.main()
                start.assert_not_called()
        finally:
            harness.MYSQL_CASE_MANIFEST["mysql57"][case] = original


if __name__ == "__main__":
    unittest.main()
