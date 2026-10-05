#!/usr/bin/env python3
"""Database-free T22 preparation: pin validation and list-only test discovery."""
import argparse
from collections import Counter
from datetime import date
import json
import os
from pathlib import Path
import platform
import re

import harness

MYSQL = {"mysql57": "5.7.", "mysql80": "8.0.", "mysql84": "8.4."}
POSTGRES = {"pg16": "16.", "pg17": "17.", "pg18": "18."}
NLS_CASE = "t12_localized::actual_french_builtin_check_error_keeps_sqlstate_recovery_and_durable_reject"
MACOS_CASES = harness.MACOS_ONLY_CASES
FEATURES = harness.NATIVE_FEATURES + ",nls-integration"


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def validate_pins(pins):
    """Validate every platform pin and report only compatible native lane pairs."""
    if not isinstance(pins, dict):
        raise ValueError("image registry must be an object")
    expected = {**MYSQL, **POSTGRES, "pg16-nls": "16.", "pg16-postgis": "16."}
    errors, valid, images = [], {}, {}
    for key, prefix in expected.items():
        entry = pins.get(key)
        variants = harness.pin_variants(entry)
        if not variants:
            errors.append(f"{key}: missing or malformed pin")
            continue
        kind = "mysql" if key in MYSQL else "postgres"
        repository = "imresamu/postgis" if key == "pg16-postgis" else kind
        for platform_name, pin in variants.items():
            label = f"{key}@{platform_name}"
            try:
                if not isinstance(pin, dict):
                    raise ValueError("platform pin must be an object")
                if platform_name not in ("linux/amd64", "linux/arm64") or pin.get("platform") != platform_name:
                    raise ValueError("pin platform must exactly match its linux/amd64 or linux/arm64 map key")
                if pin.get("kind") != kind or pin.get("version_prefix") != prefix:
                    raise ValueError("kind/version does not match required lane")
                image = pin.get("image", "")
                if not isinstance(image, str) or not re.fullmatch(re.escape(repository) + r"@sha256:[0-9a-f]{64}", image):
                    raise ValueError("immutable image digest required")
                if image in images:
                    raise ValueError(f"duplicate image pin also used by {images[image]}")
                images[image] = label
                tag = pin.get("tag_at_pin", "")
                if not isinstance(tag, str) or not re.fullmatch(re.escape(repository + ":" + prefix[:-1]) + r"(?:[.-][A-Za-z0-9_.-]+)?", tag):
                    raise ValueError("tag provenance does not match required version")
                if not isinstance(pin.get("pin_date"), str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", pin["pin_date"]):
                    raise ValueError("ISO pin_date required")
                date.fromisoformat(pin["pin_date"])
                valid.setdefault(key, {})[platform_name] = pin
            except ValueError as error:
                errors.append(f"{label}: {error}")
    for key in sorted(pins.keys() - expected.keys()):
        errors.append(f"{key}: unreviewed registry entry")
    if "mysql84" in valid and "pg16-nls" in valid and not (valid["mysql84"].keys() & valid["pg16-nls"].keys()):
        errors.append("mysql84/pg16-nls: no matching native NLS image platform")
    lanes = []
    for mysql in MYSQL:
        for postgres in POSTGRES:
            source, target = valid.get(mysql, {}), valid.get(postgres, {})
            common = sorted(source.keys() & target.keys())
            if not common:
                if source and target:
                    errors.append(f"{mysql}/{postgres}: no matching native image platform "
                                  f"(mysql={sorted(source)}, postgres={sorted(target)})")
                lanes.append({"mysql": mysql, "postgres": postgres, "mysql_pin_key": None,
                              "postgres_pin_key": None, "mysql_image": None, "postgres_image": None,
                              "pin_metadata_valid": False, "platform": None,
                              "runtime": "not_run", "compatibility": "unproved"})
                continue
            for platform_name in common:
                mysql_pin, postgres_pin = source[platform_name], target[platform_name]
                lanes.append({"mysql": mysql, "postgres": postgres,
                              "mysql_pin_key": f"{mysql}@{platform_name}",
                              "postgres_pin_key": f"{postgres}@{platform_name}",
                              "mysql_image": mysql_pin["image"], "postgres_image": postgres_pin["image"],
                              "pin_metadata_valid": True, "platform": platform_name,
                              "runtime": "not_run", "compatibility": "unproved"})
    return lanes, errors


def test_artifacts(output):
    # Reuse the mandatory harness's normal-worker selector, including path checks.
    production = harness.artifact_executable(output)
    artifacts, identities, paths = [], set(), set()
    for line in output.splitlines():
        if not line.lstrip().startswith("{"):
            continue
        item = json.loads(line, object_pairs_hook=unique_object)
        if item.get("reason") != "compiler-artifact" or item.get("manifest_path") != str(harness.ROOT / "Cargo.toml"):
            continue
        target, profile = item.get("target"), item.get("profile")
        if not isinstance(target, dict) or not isinstance(profile, dict) or type(profile.get("test")) is not bool:
            raise ValueError("malformed project Cargo artifact")
        if not profile["test"]:
            continue
        name, kind, executable = target.get("name"), target.get("kind"), item.get("executable")
        if not isinstance(name, str) or not name or kind not in (["lib"], ["bin"], ["test"]):
            raise ValueError("unknown test artifact target")
        if not isinstance(executable, str) or not Path(executable).is_absolute() or not Path(executable).is_file() or not os.access(executable, os.X_OK):
            raise ValueError("missing absolute executable test artifact")
        identity = (name, kind[0])
        if identity in identities or executable in paths:
            raise ValueError("duplicate test artifact")
        identities.add(identity)
        paths.add(executable)
        artifacts.append({"name": name, "kind": kind[0], "executable": executable})
    if ("my2pg", "lib") not in identities or ("nls", "test") not in identities or ("integration", "test") not in identities:
        raise ValueError("required library/integration/NLS test target absent")
    return str(production), artifacts


def listed_cases(output):
    cases = []
    totals = []
    for line in output.splitlines():
        if not line.strip():
            continue
        case = re.fullmatch(r"(\S+): test", line)
        total = re.fullmatch(r"(\d+) tests?, (\d+) benchmarks?", line)
        if case:
            cases.append(case[1])
        elif total and total[2] == "0":
            totals.append(int(total[1]))
        else:
            raise ValueError(f"malformed list-only output: {line}")
    if totals != [len(cases)]:
        raise ValueError("list-only case count mismatch or missing summary")
    if len(set(cases)) != len(cases):
        raise ValueError("duplicate discovered case ID")
    return cases


def validate_cases(cases, system):
    errors = []
    required = (*harness.REQUIRED_CASES, *(MACOS_CASES if system == "Darwin" else ()))
    counts = Counter(cases)
    if len(set(required)) != len(required):
        errors.append("duplicate required case ID")
    errors.extend(f"{case}: required exactly once, discovered {counts[case]}" for case in required if counts[case] != 1)
    errors.extend(f"{case}: duplicate discovered case ID" for case, count in counts.items() if count > 1)
    if system != "Darwin" and any(counts[case] for case in MACOS_CASES):
        errors.append("macOS-only cases unexpectedly compiled on this host")
    return errors


def validate_nls_cases(cases):
    """Require the feature-gated NLS target to contain exactly its own case."""
    counts = Counter(cases)
    errors = []
    if counts[NLS_CASE] != 1:
        errors.append(f"{NLS_CASE}: required exactly once in the nls target, discovered {counts[NLS_CASE]}")
    errors.extend(f"{case}: unexpected case in the nls target" for case in counts if case != NLS_CASE)
    return errors


def check():
    report = {"stage": "preflight_only", "support_certified": False, "database_tests_executed": 0,
              "commands": [], "errors": [], "lanes": validate_pins({})[0], "host": platform.system(),
              "host_architecture": platform.machine(),
              "dispositions": {
                  "nls": "separate pg16-nls pin and feature; locale/runtime proof required",
            "macos": "discover native fault cases" if platform.system() == "Darwin" else "cfg-excluded; separate native macOS fault proof required; no Linux equivalence claimed",
            "versions": "exact case inventory is checked; pending MySQL 5.7 dispositions block that lane",
            "architecture": "pins are selected only for a matching native Linux platform; no emulation evidence"}}
    if report["host"] not in ("Darwin", "Linux"):
        report["errors"].append("unreviewed host platform; native dispositions unavailable")
    try:
        pins = json.loads(harness.PINS.read_text(), object_pairs_hook=unique_object)
        report["declared_pins"] = pins
        report["lanes"], errors = validate_pins(pins)
        report["errors"].extend(errors)
    except (OSError, ValueError) as error:
        report["errors"].append(f"image registry: {error}")
    try:
        args = [str(harness.ROOT / "bin/cargo"), "test", "--offline", "--locked", "--features", FEATURES,
                "--tests", "--no-run", "--message-format=json"]
        report["commands"].append(args)
        build = harness.command(args)
        report["production_executable"], report["test_artifacts"] = test_artifacts(build.stdout)
        cases = []
        nls_cases = []
        for artifact in report["test_artifacts"]:
            args = [artifact["executable"], "--ignored", "--list"]
            report["commands"].append(args)
            discovered = listed_cases(harness.command(args).stdout)
            artifact["ignored_case_ids"] = discovered
            if artifact["name"] == "nls":
                nls_cases.extend(discovered)
            else:
                cases.extend(discovered)
        report["errors"].extend(validate_cases(cases, report["host"]))
        report["errors"].extend(validate_nls_cases(nls_cases))
        required = (*harness.REQUIRED_CASES, *(MACOS_CASES if report["host"] == "Darwin" else ()))
        report["required_case_counts"] = {case: cases.count(case) for case in required}
        report["nls_case_counts"] = {NLS_CASE: nls_cases.count(NLS_CASE)}
        report["discovered_ignored_case_count"] = len(cases) + len(nls_cases)
        report["mysql_version_applicability"] = {}
        for lane in MYSQL:
            case_plan = harness.mysql_case_plan(lane, cases)
            counts = Counter(status for status, _ in case_plan.values())
            pending = sorted(case for case, (status, _) in case_plan.items() if status == "pending")
            report["mysql_version_applicability"][lane] = {
                "discovered": len(case_plan),
                "run": counts["run"],
                "not_applicable": counts["not_applicable"],
                "pending": len(pending),
                "pending_sample": pending[:8],
            }
            if pending:
                report["errors"].append(
                    f"{lane}: {len(pending)} discovered cases have pending version applicability decisions"
                )
    except (OSError, ValueError, RuntimeError) as error:
        report["errors"].append(f"test discovery: {error}")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", required=True)
    parser.add_argument("--all", action="store_true", required=True)
    parser.parse_args()
    report = check()
    print(json.dumps(report, indent=2, sort_keys=True))
    return int(bool(report["errors"]))


if __name__ == "__main__":
    raise SystemExit(main())
