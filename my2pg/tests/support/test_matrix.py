import copy
import json
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import matrix


def complete_pins():
    pins = {}
    for number, (key, version) in enumerate({**matrix.MYSQL, **matrix.POSTGRES,
                                              "pg16-nls": "16.", "pg16-postgis": "16."}.items(), 1):
        kind = "mysql" if key in matrix.MYSQL else "postgres"
        repository = "imresamu/postgis" if key == "pg16-postgis" else kind
        pins[key] = {"image": f"{repository}@sha256:{number:064x}", "kind": kind,
                     "version_prefix": version, "platform": "linux/arm64",
                     "tag_at_pin": f"{repository}:{version[:-1]}", "pin_date": "2026-10-02"}
    return pins


def artifact(name, kind, test):
    return {"reason": "compiler-artifact", "manifest_path": str(matrix.harness.ROOT / "Cargo.toml"),
            "target": {"name": name, "kind": [kind]}, "profile": {"test": test},
            "executable": sys.executable}


class MatrixTests(unittest.TestCase):
    def test_complete_metadata_enumerates_exact_cartesian_product(self):
        lanes, errors = matrix.validate_pins(complete_pins())
        self.assertEqual(errors, [])
        self.assertEqual([(x["mysql"], x["postgres"]) for x in lanes],
                         [(m, p) for m in matrix.MYSQL for p in matrix.POSTGRES])
        self.assertTrue(all(x["pin_metadata_valid"] and x["runtime"] == "not_run" for x in lanes))

    def test_nested_platform_pins_keep_arm_lanes_and_add_native_amd64_lanes(self):
        pins = complete_pins()
        for number, (key, arm) in enumerate(list(pins.items()), 1000):
            amd = {**arm, "image": arm["image"].split("@", 1)[0] + "@sha256:" + f"{number:064x}",
                   "platform": "linux/amd64"}
            pins[key] = {"platforms": {"linux/arm64": arm, "linux/amd64": amd}}
        # MySQL 5.7 is only pinned for AMD64; PostgreSQL keeps both native platforms.
        pins["mysql57"] = {"platforms": {"linux/amd64": {
            **pins["mysql57"]["platforms"]["linux/amd64"],
            "image": "mysql@sha256:" + f"{999:064x}"}}}
        lanes, errors = matrix.validate_pins(pins)
        self.assertEqual(errors, [])
        mysql57 = [lane for lane in lanes if lane["mysql"] == "mysql57"]
        self.assertEqual(len(mysql57), 3)
        self.assertTrue(all(lane["platform"] == "linux/amd64" for lane in mysql57))
        self.assertTrue(all(lane["mysql_pin_key"] == "mysql57@linux/amd64" for lane in mysql57))
        self.assertTrue(all(lane["postgres_pin_key"].endswith("@linux/amd64") for lane in mysql57))
        other_lanes = [lane for lane in lanes if lane["mysql"] != "mysql57"]
        self.assertEqual(len(other_lanes), 12)
        self.assertEqual({lane["platform"] for lane in other_lanes}, {"linux/amd64", "linux/arm64"})

    def test_nested_pin_platform_key_must_match_pin_metadata(self):
        pins = complete_pins()
        pins["mysql84"] = {"platforms": {"linux/amd64": {**pins["mysql84"],
                                                              "platform": "linux/arm64"}}}
        lanes, errors = matrix.validate_pins(pins)
        self.assertTrue(any("mysql84@linux/amd64" in error and "exactly match" in error for error in errors))
        self.assertFalse(any(lane["mysql"] == "mysql84" and lane["pin_metadata_valid"] for lane in lanes))

    def test_missing_pin_keeps_all_nine_lanes_and_fails(self):
        pins = complete_pins()
        del pins["mysql57"]
        lanes, errors = matrix.validate_pins(pins)
        self.assertEqual(len(lanes), 9)
        self.assertIn("mysql57: missing or malformed pin", errors)
        self.assertFalse(any(x["pin_metadata_valid"] for x in lanes if x["mysql"] == "mysql57"))

    def test_bad_metadata_never_counts_as_a_pin(self):
        variants = [("image", "mysql:8.4"), ("platform", "linux/s390x"), ("kind", "mariadb"),
                    ("version_prefix", "5.7."), ("tag_at_pin", "mysql:8.0"), ("pin_date", "tomorrow")]
        for field, value in variants:
            with self.subTest(field=field):
                pins = complete_pins()
                pins["mysql84"][field] = value
                lanes, errors = matrix.validate_pins(pins)
                self.assertTrue(errors)
                self.assertFalse(any(x["pin_metadata_valid"] for x in lanes if x["mysql"] == "mysql84"))

    def test_postgis_repository_is_accepted_only_for_its_reviewed_lane(self):
        pins = complete_pins()
        pins["pg16-postgis"].update({
            "image": "imresamu/postgis@sha256:" + "f" * 64,
            "tag_at_pin": "imresamu/postgis:16-3.4.4-alpine3.21",
        })
        self.assertEqual(matrix.validate_pins(pins)[1], [])

        pins["pg16"] = {**pins["pg16"],
                         "image": "imresamu/postgis@sha256:" + "e" * 64,
                         "tag_at_pin": "imresamu/postgis:16-3.4.4-alpine3.21"}
        _, errors = matrix.validate_pins(pins)
        self.assertIn("pg16@linux/arm64: immutable image digest required", errors)

    def test_duplicate_keys_images_and_unknown_entries_fail(self):
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            json.loads('{"mysql84":{},"mysql84":{}}', object_pairs_hook=matrix.unique_object)
        pins = complete_pins()
        pins["mysql80"]["image"] = pins["mysql84"]["image"]
        self.assertTrue(any("duplicate image" in e for e in matrix.validate_pins(pins)[1]))
        pins["mariadb"] = {}
        self.assertTrue(any("unreviewed" in e for e in matrix.validate_pins(pins)[1]))

    def test_native_pair_platform_mismatch_fails(self):
        pins = complete_pins()
        pins["mysql57"]["platform"] = "linux/amd64"
        lanes, errors = matrix.validate_pins(pins)
        self.assertEqual(sum("no matching native image platform" in e for e in errors), 3)
        self.assertFalse(any(x["pin_metadata_valid"] for x in lanes if x["mysql"] == "mysql57"))
        pins = complete_pins()
        pins["pg16-nls"]["platform"] = "linux/amd64"
        self.assertEqual(matrix.validate_pins(pins)[1], ["mysql84/pg16-nls: no matching native NLS image platform"])

    def test_list_parser_requires_exact_count_and_unique_ids(self):
        self.assertEqual(matrix.listed_cases("a::case: test\n\n1 test, 0 benchmarks\n"), ["a::case"])
        self.assertEqual(matrix.listed_cases("0 tests, 0 benchmarks\n"), [])
        for output in ("", "a: test\n0 tests, 0 benchmarks", "a: test\na: test\n2 tests, 0 benchmarks",
                       "running a\n1 test, 0 benchmarks", "1 test, 1 benchmark"):
            with self.subTest(output=output), self.assertRaises(ValueError):
                matrix.listed_cases(output)

    def test_required_discovery_and_macos_disposition(self):
        cases = list(matrix.harness.REQUIRED_CASES)
        self.assertEqual(matrix.validate_cases(cases, "Linux"), [])
        self.assertEqual(matrix.validate_cases(cases + list(matrix.MACOS_CASES), "Darwin"), [])
        self.assertTrue(matrix.validate_cases(cases, "Darwin"))
        self.assertTrue(matrix.validate_cases(cases[:-1], "Linux"))
        self.assertTrue(matrix.validate_cases(cases + cases[:1], "Linux"))
        self.assertTrue(matrix.validate_cases(cases + list(matrix.MACOS_CASES), "Linux"))
        self.assertEqual(matrix.validate_nls_cases([matrix.NLS_CASE]), [])
        self.assertTrue(matrix.validate_nls_cases([]))
        self.assertTrue(matrix.validate_nls_cases([matrix.NLS_CASE, matrix.NLS_CASE]))
        self.assertTrue(matrix.validate_nls_cases([matrix.NLS_CASE, "unrelated::case"]))
        with self.assertRaisesRegex(ValueError, "unknown discovered"):
            matrix.harness.mysql_case_plan("mysql84", [matrix.NLS_CASE])
        combined = "t12_uncertain_storage::sent_commit_uncertainty_survives_physical_failure_to_publish_new_report"
        self.assertIn(combined, matrix.MACOS_CASES)
        without_combined = [case for case in cases + list(matrix.MACOS_CASES) if case != combined]
        self.assertTrue(any(combined in e for e in matrix.validate_cases(without_combined, "Darwin")))

    def test_cargo_parser_missing_duplicate_malformed_targets_fail(self):
        normal = artifact("my2pg", "bin", False)
        for entries in ([normal], [normal, normal], [normal, artifact("nls", "test", True)],
                        [normal, {**artifact("my2pg", "lib", True), "executable": "relative"}],
                        [normal, {**artifact("my2pg", "lib", True), "profile": {"test": "true"}}]):
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                matrix.test_artifacts("\n".join(map(json.dumps, entries)))

    def test_cargo_parser_uses_only_actual_test_artifacts(self):
        entries = [artifact("my2pg", "bin", False), artifact("my2pg", "lib", True),
                   artifact("integration", "test", True), artifact("nls", "test", True)]
        # Real files with distinct executable identities; no process is executed.
        for entry, path in zip(entries, (sys.executable, "/bin/ls", "/bin/cat", "/bin/sh")):
            entry["executable"] = path
        production, tests = matrix.test_artifacts("\n".join(map(json.dumps, entries)))
        self.assertEqual(production, sys.executable)
        self.assertEqual([a["name"] for a in tests], ["my2pg", "integration", "nls"])
        duplicate = copy.deepcopy(entries[-1])
        with self.assertRaisesRegex(ValueError, "duplicate test artifact"):
            matrix.test_artifacts("\n".join(map(json.dumps, entries + [duplicate])))

    def test_compile_failure_is_reported_without_any_test_execution(self):
        with patch.object(matrix.harness.PINS.__class__, "read_text", return_value=json.dumps(complete_pins())), \
                patch.object(matrix.harness, "command", side_effect=RuntimeError("compile failed")) as process:
            report = matrix.check()
        self.assertEqual(report["database_tests_executed"], 0)
        self.assertFalse(report["support_certified"])
        self.assertEqual(process.call_count, 1)
        self.assertIn("--no-run", process.call_args.args[0])
        self.assertIn("--offline", process.call_args.args[0])
        self.assertEqual(report["errors"], ["test discovery: compile failed"])

    def test_malformed_registry_still_enumerates_lanes(self):
        with patch.object(matrix.harness.PINS.__class__, "read_text", return_value='{"mysql84":{},"mysql84":{}}'), \
                patch.object(matrix.harness, "command", side_effect=RuntimeError("blocked")):
            report = matrix.check()
        self.assertEqual(len(report["lanes"]), 9)
        self.assertTrue(any("duplicate JSON key" in e for e in report["errors"]))

    def test_full_mocked_preflight_only_builds_and_lists(self):
        entries = [artifact("my2pg", "bin", False), artifact("my2pg", "lib", True),
                   artifact("integration", "test", True), artifact("nls", "test", True)]
        for entry, path in zip(entries, (sys.executable, "/bin/ls", "/bin/cat", "/bin/sh")):
            entry["executable"] = path
        cases = [*matrix.harness.REQUIRED_CASES, *matrix.MACOS_CASES]
        def execute(args):
            if "--no-run" in args:
                return SimpleNamespace(stdout="\n".join(map(json.dumps, entries)))
            self.assertEqual(args[1:], ["--ignored", "--list"])
            names = cases if args[0] == "/bin/cat" else [matrix.NLS_CASE] if args[0] == "/bin/sh" else []
            return SimpleNamespace(stdout="".join(f"{name}: test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n")
        with patch.object(matrix.harness.PINS.__class__, "read_text", return_value=json.dumps(complete_pins())), \
                patch.object(matrix.platform, "system", return_value="Darwin"), \
                patch.object(matrix.harness, "command", side_effect=execute), \
                patch.object(matrix.harness, "start") as start:
            report = matrix.check()
        self.assertFalse(any("pending version applicability decisions" in error for error in report["errors"]))
        self.assertEqual(len(report["lanes"]), 9)
        self.assertEqual(report["database_tests_executed"], 0)
        self.assertFalse(report["support_certified"])
        self.assertTrue(all(count == 1 for count in report["required_case_counts"].values()))
        self.assertEqual(report["nls_case_counts"], {matrix.NLS_CASE: 1})
        self.assertEqual(report["discovered_ignored_case_count"], len(cases) + 1)
        self.assertEqual(report["mysql_version_applicability"]["mysql57"]["pending"], 0)
        self.assertEqual(
            report["mysql_version_applicability"]["mysql57"]["run"]
            + report["mysql_version_applicability"]["mysql57"]["not_applicable"],
            report["mysql_version_applicability"]["mysql57"]["discovered"],
        )
        self.assertEqual(report["mysql_version_applicability"]["mysql80"]["pending"], 0)
        start.assert_not_called()


if __name__ == "__main__":
    unittest.main()
