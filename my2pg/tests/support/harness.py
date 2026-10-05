#!/usr/bin/env python3
"""Owned disposable DBs. An explicitly requested test lane never silently skips."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import signal
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
COMPOSE = ROOT / "tests/compose/compose.yml"
PINS = ROOT / "tests/compose/images.json"
FIXTURE = ROOT / "tests/fixtures/core"
NATIVE_FEATURES = "artifact-worker-tests,native-import-tests"
REQUIRED_CASES = (
    "t10_empty_string_default::native_empty_text_defaults_remain_distinct_without_source_rows_or_writes",
    "t10_enum_metadata::supplementary_enum_set_labels_are_indistinguishable_from_question_marks_in_catalogs",
    "t16_import_native::imported_cli_casts_filters_renames_and_defaults_have_native_fidelity",
    "t16_import_native::imported_existing_modes_preserve_native_oids_and_refusals_publish_nothing",
    "t15_durable_run::production_cli_persists_conversion_and_copy_rejects_before_final_accounting",
    "t15_durable_run::artifact_admission_and_creation_fail_before_target_mutation",
    "pipeline::worker_fault_tests::native_runner_panic_after_ack_retains_prefix_and_closes_sibling",
    "pipeline::worker_fault_tests::native_runner_panic_preserves_registered_unread_source_backend",
    "t11_identity_edges::native_runner_round_trips_unsigned_bigint_max_as_numeric",
    "t11_identity_edges::native_runner_refuses_nondefault_source_auto_increment_policy_before_ddl",
    "t11_identity_exhaustion::native_runner_refuses_source_auto_increment_beyond_smallint_sequence_max",
    "t13_snapshot_consistency::actual_run_keeps_the_original_source_snapshot_for_later_tables",
    "t13_external_mdl::cancellation_kills_reader_waiting_on_external_mysql_metadata_lock",
    "t13_snapshot_mdl::cancellation_terminates_single_snapshot_select_waiting_on_external_mdl",
    "t13_snapshot_mdl::cancellation_terminates_single_snapshot_reader_blocked_during_preflight",
    "t11_schema_only_recreate::native_schema_only_recreate_replaces_selected_table_without_copying_or_touching_sentinel",
    "t11_truncate_sequence_reset::native_data_only_truncate_resets_identity_after_copy_and_preserves_source_and_sentinels",
    "t12_lost_commit_ack::completed_commit_with_dropped_ack_is_indeterminate_and_never_replayed",
    "t17_on_update_override::native_on_update_requires_explicit_omission_and_omission_does_not_create_trigger",
    "t17_generated_policy::native_generated_columns_require_expression_or_materialization_and_match_existing_targets",
    "t17_check_override::native_check_requires_explicit_postgres_expression_and_verifies_it_exactly",
    "t20_ranges::integer_ranges_preserve_exact_signed_unsigned_and_fallback_multisets",
)
MACOS_REQUIRED_CASES = (
    "t12_uncertain_storage::sent_commit_uncertainty_survives_physical_failure_to_publish_new_report",
)
MACOS_ONLY_CASES = (
    "t12_durable_faults::actual_cli_physical_enospc_stops_without_replaying_and_preserves_ack_report",
    "t12_durable_faults::metadata_descriptor_sync_fault_never_advances_second_durable_reject",
    "t12_durable_faults::physical_enospc_poisoned_reject_and_atomic_report_preserve_prior_durable_evidence",
    *MACOS_REQUIRED_CASES,
)


# Exact list-only inventory: new ignored tests must receive reviewed version applicability.
MYSQL_CASE_INVENTORY = (
    "deterministic_fixture_is_accessible_over_verified_tls",
    "mysql_owned_stream_backpressure_and_cancellation",
    "mysql_raw_values_and_copy_transactions",
    "pipeline::worker_fault_tests::native_runner_panic_after_ack_retains_prefix_and_closes_sibling",
    "pipeline::worker_fault_tests::native_runner_panic_preserves_registered_unread_source_backend",
    "postgres::tests::native_catalog_union_uses_one_read_only_repeatable_snapshot",
    "t05_mysql::abandoned_reader_guard_closes_without_draining",
    "t05_mysql::catalog_preserves_order_and_original_metadata",
    "t05_mysql::catalog_query_failure_never_returns_empty_success",
    "t05_mysql::empty_database_and_denied_source_are_distinct",
    "t05_mysql::production_mysql_tls_mutual_auth_and_policy",
    "t05_mysql::projection_preserves_raw_charset_set_mask_and_snapshot",
    "t05_mysql::reader_drop_during_panic_and_outside_runtime_closes_promptly",
    "t05_mysql::real_packet_over_limit_fails_before_conversion",
    "t06_plan::pure_plan_and_text_encoder_execute_real_relational_fixture",
    "t07_postgres::cancellation_before_and_during_commit_preserves_stage",
    "t07_postgres::copy_acknowledgement_rollback_metadata_and_qualification",
    "t07_postgres::restricted_role_privileges_and_external_dependency_are_visible",
    "t07_postgres::startup_failure_closes_connection_without_claiming_rollback",
    "t07_postgres::tls_password_file_and_dropped_guard_are_real",
    "t08_pipeline::actual_copy_cancellation_rolls_back_pending_batch",
    "t08_pipeline::advisory_lock_and_artifact_failure_precede_writes",
    "t08_pipeline::cancelled_ddl_is_not_a_failed_execution_step",
    "t08_pipeline::closed_json_output_fails_and_persists_the_run_report",
    "t08_pipeline::conversion_failure_preserves_acknowledged_batches_and_artifacts",
    "t08_pipeline::explicit_transform_counts_changed_rows_and_null_stays_unchanged",
    "t08_pipeline::interrupted_commit_marks_only_submitted_rows_indeterminate",
    "t08_pipeline::pipeline_verification_keeps_the_original_snapshot",
    "t08_pipeline::readonly_plan_success_schema_only_and_preflight_gates",
    "t09_cli::actual_cli_conversion_failure_persists_accurate_partial_report",
    "t09_cli::actual_cli_plan_is_read_only_and_run_preserves_relational_values",
    "t09_cli::actual_cli_sigterm_rolls_back_active_copy_and_persists_cancellation",
    "t10_binary_default::native_binary_defaults_recover_actual_bytes_without_source_rows_or_writes",
    "t10_empty_string_default::native_empty_text_defaults_remain_distinct_without_source_rows_or_writes",
    "t10_enum_metadata::supplementary_enum_set_labels_are_indistinguishable_from_question_marks_in_catalogs",
    "t10_label_recovery::native_recursive_cte_widens_enum_set_and_cannot_recover_labels",
    "t10_label_recovery::native_show_create_and_binary_metadata_lose_unused_labels_in_both_sql_modes",
    "t10_mysql_projection::enum_projection_distinguishes_valid_empty_invalid_and_null_even_for_text_override",
    "t10_set_verification::typed_set_membership_matches_catalog_and_detects_changed_or_unvalidated_checks",
    "t10_values::native_invalid_enum_zero_dates_and_byte_sequences_are_explicit_errors",
    "t10_values::native_numeric_temporal_enum_set_charsets_and_defaults_roundtrip",
    "t11_catalog::native_failed_concurrent_index_retains_real_invalid_state",
    "t11_catalog::native_observed_columns_indexes_constraints_are_ordered_typed_and_read_only",
    "t11_constraint_triggers::native_deferred_constraint_error_retains_ack_and_custom_retry_is_not_replayed",
    "t11_constraint_triggers::native_linked_constraint_trigger_preservation_firing_and_metadata_fail_closed",
    "t11_cross_schema_run::native_run_migrates_mapped_schemas_and_refuses_partial_lock_set_before_writes",
    "t11_deferred_keys::native_deferred_unique_extra_preserves_oid_aborts_duplicate_and_rejects_damaged_key_links",
    "t11_dependency_graph::native_dependency_closure_types_headers_and_errors_are_distinct",
    "t11_existing_comments::native_existing_append_rejects_comment_drift_before_copy_or_mutation",
    "t11_existing_policies::native_composite_order_extra_states_and_unknown_graph_fail_before_mutation",
    "t11_existing_policies::native_existing_append_truncate_recreate_bind_actual_structures_and_preserve_sentinels",
    "t11_existing_policies::native_existing_enum_reuses_exact_identity_order_and_preserves_outside_users",
    "t11_existing_policies::native_mapped_schema_facts_scope_collisions_and_privileges",
    "t11_graph_invariants::graph_edges_require_incident_raw_identity_and_coherent_owner_extension_fields",
    "t11_graph_invariants::namespace_only_type_headers_refuse_dangling_permission_owner_and_extension_facts",
    "t11_graph_invariants::native_system_column_dependencies_preserve_exact_negative_attribute_ids",
    "t11_identity_edges::native_runner_refuses_nondefault_source_auto_increment_policy_before_ddl",
    "t11_identity_edges::native_runner_round_trips_unsigned_bigint_max_as_numeric",
    "t11_identity_exhaustion::native_runner_refuses_source_auto_increment_beyond_smallint_sequence_max",
    "t11_identity_types::native_runner_preserves_supported_auto_increment_integer_mappings",
    "t11_identity_types::native_runner_refuses_unsigned_bigint_auto_increment_before_target_ddl",
    "t11_monotonic_sequences::actual_adjustment_never_rewinds_and_next_insert_uses_exact_bound",
    "t11_monotonic_sequences::fresh_sequence_drift_exhaustion_and_external_users_fail_before_setval",
    "t11_monotonic_sequences::real_runner_adjusts_existing_identity_and_preserves_disabled_state",
    "t11_monotonic_sequences::sequence_privileges_table_lock_and_nontransactional_effect_are_observable",
    "t11_multischema::namespace_union_preserves_distinct_names_privileges_defaults_and_graph_roots",
    "t11_observed_decoder::malformed_binary_rows_and_failed_metadata_query_are_errors_not_empty_success",
    "t11_observed_decoder::public_inspector_preserves_sequence_state_denial_and_global_event_headers",
    "t11_observed_decoder::public_inspector_retains_typed_observations_and_known_empty_without_mutation",
    "t11_schema_only_recreate::native_schema_only_recreate_replaces_selected_table_without_copying_or_touching_sentinel",
    "t11_sequence_catalog::native_sequence_ownership_dependencies_privileges_and_state_are_independent",
    "t11_sequence_catalog::native_trigger_headers_function_provenance_and_global_event_inventory_survive",
    "t11_structures::native_cli_full_and_schema_only_preserve_renamed_composite_structures",
    "t11_truncate_sequence_reset::native_data_only_truncate_resets_identity_after_copy_and_preserves_source_and_sentinels",
    "t12_durable_faults::actual_cli_physical_enospc_stops_without_replaying_and_preserves_ack_report",
    "t12_durable_faults::metadata_descriptor_sync_fault_never_advances_second_durable_reject",
    "t12_durable_faults::physical_enospc_poisoned_reject_and_atomic_report_preserve_prior_durable_evidence",
    "t12_lost_commit_ack::completed_commit_with_dropped_ack_is_indeterminate_and_never_replayed",
    "t12_recovery::bad_row_positions_exact_bytes_and_left_first_duplicates",
    "t12_recovery::cancellation_after_left_ack_and_unknown_right_commit_preserves_progress",
    "t12_recovery::forged_retry_code_custom_operators_and_classes_are_not_replayed",
    "t12_recovery::genuine_repeated_deadlocks_exhaust_three_attempts_without_committing",
    "t12_recovery::inheritance_parent_child_and_partition_members_are_blocked_without_mutation",
    "t12_recovery::init_permission_missing_relation_and_columns_are_never_bisected",
    "t12_recovery::malformed_batch_observer_stop_and_fatal_constraint_have_no_replay",
    "t12_recovery::ordinary_error_codes_limits_stop_and_artifact_failure",
    "t12_recovery::real_connection_loss_after_partial_progress_and_foreign_keys_never_replay",
    "t12_recovery::real_deadlock_rolls_back_retries_identical_slice_and_commits_once",
    "t12_recovery::real_partial_reject_write_failure_never_claims_durable_reject",
    "t12_recovery::real_serialization_conflict_is_observed_then_retried_with_one_ack",
    "t12_recovery::safety_guard_blocks_custom_behavior_and_preserves_safe_enums_arrays",
    "t12_recovery::typed_set_membership_check_is_safe_but_changed_or_custom_casts_block",
    "t12_required_ddl::native_required_check_ddl_failure_persists_failed_report_after_copy",
    "t12_runner::actual_cli_broken_final_output_preserves_indeterminate_commit_report",
    "t12_runner::actual_cli_conversion_and_copy_rejects_share_exact_global_limit",
    "t12_runner::actual_cli_existing_hierarchies_fail_before_any_policy_mutates_sentinels",
    "t12_runner::actual_cli_named_transform_counts_only_changed_nonnull_values",
    "t12_runner::actual_cli_next_reject_after_cap_fails_without_losing_earlier_commits",
    "t12_runner::actual_cli_reject_file_collision_preserves_final_failed_report_and_prior_commits",
    "t12_runner::actual_cli_sigterm_at_recovery_commit_marks_only_active_tail_indeterminate",
    "t12_runner::actual_cli_sigterm_preserves_committed_recovery_before_blocked_copy_tail",
    "t12_runner::actual_cli_stop_conversion_never_creates_rejects",
    "t12_runner::actual_cli_unsafe_recovery_target_is_rejected_before_truncate",
    "t12_uncertain_storage::sent_commit_uncertainty_survives_physical_failure_to_publish_new_report",
    "t13_concurrency::actual_cli_fixed_budget_rss_does_not_scale_with_tenfold_source_rows",
    "t13_concurrency::actual_cli_sigterm_stops_two_blocked_full_queues",
    "t13_concurrency::actual_writer_socket_failure_stops_full_queue_sibling_without_replay",
    "t13_concurrency::concurrent_commit_cancellation_retains_sibling_ack_prefix_without_replay",
    "t13_concurrency::control_ddl_signal_before_commit_is_cancelled_not_transport_failure",
    "t13_concurrency::empty_queues_cancel_blocked_source_and_close_reader_writer_pairs",
    "t13_concurrency::exact_one_pipeline_budget_reduces_admission_and_finishes_all_tables",
    "t13_concurrency::full_queues_cancel_two_copy_workers_and_release_all_connections",
    "t13_concurrency::mixed_conversion_copy_rejects_share_one_global_durable_limit",
    "t13_concurrency::schema_only_index_workers_are_effective_and_separately_bounded",
    "t13_concurrency::source_socket_fault_wakes_empty_consumers_and_stops_sibling",
    "t13_external_mdl::cancellation_kills_reader_waiting_on_external_mysql_metadata_lock",
    "t13_shutdown_phases::actual_control_verification_cancellation_retains_all_acknowledged_tables",
    "t13_shutdown_phases::concurrent_index_cancellation_before_commit_clears_active_ddl_ledger",
    "t13_snapshot_consistency::actual_run_keeps_the_original_source_snapshot_for_later_tables",
    "t13_snapshot_mdl::cancellation_terminates_single_snapshot_reader_blocked_during_preflight",
    "t13_snapshot_mdl::cancellation_terminates_single_snapshot_select_waiting_on_external_mdl",
    "t14_auth::percent_encoded_passwords_private_passfiles_and_secret_redaction",
    "t14_auth::production_hostname_and_expired_server_validation_are_tls_failures",
    "t14_auth::production_mysql_mutual_tls_rejects_invalid_certificate_chains",
    "t14_auth::production_postgres_mutual_tls_rejects_invalid_client_identities",
    "t14_auth::sessions_apply_on_every_connection_and_bad_values_fail_without_leaking",
    "t15_console::actual_failure_diagnostics_honor_color_no_color_and_verbose",
    "t15_console::actual_json_table_metrics_are_scoped_and_acknowledged_copy_text",
    "t15_console::real_pty_detects_stderr_and_respects_actual_narrow_width",
    "t15_console::redirected_plain_quiet_verbose_and_json_outputs_match_reports",
    "t15_default_complete::complete_supported_defaults_survive_native_styles_identity_and_mutations",
    "t15_defaults::planner_pipeline_core_defaults_and_mutations_use_actual_catalog",
    "t15_defaults::unknown_identical_default_does_not_execute_its_side_effect",
    "t15_durable_run::artifact_admission_and_creation_fail_before_target_mutation",
    "t15_durable_run::production_cli_persists_conversion_and_copy_rejects_before_final_accounting",
    "t15_enum_catalog::live_enum_label_addition_preserves_oid_count_default_but_fails_schema_verification",
    "t15_verify::actual_counts_columns_keys_validity_and_honest_scope",
    "t15_verify::explicit_identity_generation_state_and_exhaustion_are_verified_without_consumption",
    "t15_verify::safe_typed_default_literals_match_real_catalog_and_detect_changes",
    "t15_verify::verification_uses_the_supplied_single_snapshot_instead_of_a_new_connection",
    "t16_import_native::imported_cli_casts_filters_renames_and_defaults_have_native_fidelity",
    "t16_import_native::imported_existing_modes_preserve_native_oids_and_refusals_publish_nothing",
    "t17_check_override::native_check_requires_explicit_postgres_expression_and_verifies_it_exactly",
    "t17_generated_policy::native_generated_columns_require_expression_or_materialization_and_match_existing_targets",
    "t17_index_expression::native_functional_index_requires_override_and_preserves_unique_behavior",
    "t17_on_update_override::native_on_update_requires_explicit_omission_and_omission_does_not_create_trigger",
    "t17_spatial::native_spatial_requires_preinstalled_postgis_and_preserves_nothing_without_it",
    "t17_spatial::native_spatial_values_preserve_shape_empty_null_and_srid_with_postgis",
    "t18_content::append_content_baseline_unites_existing_rows_and_migrated_rows_exactly",
    "t18_content::live_content_streams_transformed_rows_and_reports_equal_count_corruption",
    "t18_pipeline::content_verification_is_reported_by_the_real_pipeline_and_append_includes_baseline",
    "t19_views_hooks::after_hook_sql_failure_fails_an_otherwise_completed_load",
    "t19_views_hooks::after_hooks_are_suppressed_when_target_preparation_fails",
    "t19_views_hooks::selected_view_is_snapshotted_and_hooks_bracket_only_the_run",
    "t20_ranges::integer_ranges_preserve_exact_signed_unsigned_and_fallback_multisets",
    "verified_tls_rejects_wrong_ca_and_hostname",
)

MYSQL57_NOT_APPLICABLE = {
    "mysql_owned_stream_backpressure_and_cancellation":
        "fixture uses a nonrecursive CTE, which MySQL 5.7 does not support",
    "pipeline::worker_fault_tests::native_runner_panic_preserves_registered_unread_source_backend":
        "unread-source fixture generates rows with a recursive CTE, which MySQL 5.7 does not support",
    "t05_mysql::catalog_preserves_order_and_original_metadata":
        "fixture requires MySQL 8 invisible columns and enforced CHECK metadata",
    "t05_mysql::abandoned_reader_guard_closes_without_draining":
        "fixture uses recursive CTEs, which MySQL 5.7 does not support",
    "t05_mysql::reader_drop_during_panic_and_outside_runtime_closes_promptly":
        "fixture uses recursive CTEs, which MySQL 5.7 does not support",
    "t10_label_recovery::native_recursive_cte_widens_enum_set_and_cannot_recover_labels":
        "fixture uses recursive CTEs, which MySQL 5.7 does not support",
    "t10_binary_default::native_binary_defaults_recover_actual_bytes_without_source_rows_or_writes":
        "fixture guard uses an expression default, which requires MySQL 8.0.13 or later",
    "t12_required_ddl::native_required_check_ddl_failure_persists_failed_report_after_copy":
        "MySQL 5.7 parses but does not enforce CHECK constraints; the case requires an enforced source CHECK",
    "t11_structures::native_cli_full_and_schema_only_preserve_renamed_composite_structures":
        "MySQL 5.7 accepts but ignores descending index key parts, which this case asserts",
    "t11_existing_policies::native_composite_order_extra_states_and_unknown_graph_fail_before_mutation":
        "MySQL 5.7 accepts but ignores descending index key parts, which this case asserts",
    "t13_concurrency::actual_cli_fixed_budget_rss_does_not_scale_with_tenfold_source_rows":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::actual_cli_sigterm_stops_two_blocked_full_queues":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::actual_writer_socket_failure_stops_full_queue_sibling_without_replay":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::concurrent_commit_cancellation_retains_sibling_ack_prefix_without_replay":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::empty_queues_cancel_blocked_source_and_close_reader_writer_pairs":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::exact_one_pipeline_budget_reduces_admission_and_finishes_all_tables":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::full_queues_cancel_two_copy_workers_and_release_all_connections":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::mixed_conversion_copy_rejects_share_one_global_durable_limit":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t13_concurrency::source_socket_fault_wakes_empty_consumers_and_stops_sibling":
        "shared fixture uses SET cte_max_recursion_depth and INSERT WITH RECURSIVE, unavailable in MySQL 5.7",
    "t17_check_override::native_check_requires_explicit_postgres_expression_and_verifies_it_exactly":
        "MySQL 5.7 parses but does not enforce CHECK constraints; the case requires an enforced source CHECK",
    "t17_index_expression::native_functional_index_requires_override_and_preserves_unique_behavior":
        "functional indexes require MySQL 8.0.13 or later",
    "t17_spatial::native_spatial_requires_preinstalled_postgis_and_preserves_nothing_without_it":
        "shared fixture uses GEOMETRYCOLLECTION EMPTY syntax unsupported by MySQL 5.7",
    "t17_spatial::native_spatial_values_preserve_shape_empty_null_and_srid_with_postgis":
        "shared fixture uses GEOMETRYCOLLECTION EMPTY syntax unsupported by MySQL 5.7",
}

MYSQL57_NEEDS_RUNTIME = {}

MYSQL_CASE_MANIFEST = {
    "mysql57": {
        case: ("not_applicable", MYSQL57_NOT_APPLICABLE[case])
        if case in MYSQL57_NOT_APPLICABLE
        else ("pending", MYSQL57_NEEDS_RUNTIME[case])
        if case in MYSQL57_NEEDS_RUNTIME
        else ("run", None)
        for case in MYSQL_CASE_INVENTORY
    },
    "mysql80": {case: ("run", None) for case in MYSQL_CASE_INVENTORY},
    "mysql84": {case: ("run", None) for case in MYSQL_CASE_INVENTORY},
}


def mysql_case_plan(mysql_lane, discovered_cases):
    """Resolve every discovered test ID or fail; pending cases block that lane."""
    if mysql_lane not in MYSQL_CASE_MANIFEST:
        raise ValueError(f"unknown MySQL version lane: {mysql_lane}")
    if len(discovered_cases) != len(set(discovered_cases)):
        raise ValueError("duplicate discovered MySQL case ID")
    dispositions = MYSQL_CASE_MANIFEST[mysql_lane]
    unknown = sorted(set(discovered_cases) - set(MYSQL_CASE_INVENTORY))
    missing = sorted(set(discovered_cases) - set(dispositions))
    if unknown:
        raise ValueError("unknown discovered MySQL case IDs: " + ", ".join(unknown))
    if missing:
        raise ValueError("missing MySQL case dispositions: " + ", ".join(missing))
    plan = {case: dispositions[case] for case in discovered_cases}
    for case, disposition in plan.items():
        if not isinstance(disposition, tuple) or len(disposition) != 2:
            raise ValueError(f"malformed MySQL case disposition: {case}")
        status, reason = disposition
        if status not in ("run", "not_applicable", "pending"):
            raise ValueError(f"unknown MySQL case disposition: {case}: {status}")
        if status == "not_applicable" and (not isinstance(reason, str) or not reason.strip()):
            raise ValueError(f"not_applicable requires a concrete source feature/version reason: {case}")
        if status == "pending" and (not isinstance(reason, str) or not reason.strip()):
            raise ValueError(f"pending requires concrete unresolved version evidence: {case}")
    return plan


def mysql_case_skip_selectors(plan):
    """Return unique libtest substring selectors for reviewed non-applicable cases."""
    excluded = sorted(case for case, (status, _) in plan.items() if status == "not_applicable")
    for case in excluded:
        matches = [candidate for candidate in MYSQL_CASE_INVENTORY if case in candidate]
        if matches != [case]:
            raise ValueError(f"non-applicable selector is not unique: {case}")
    return excluded

MYSQL_80_REQUIRED_CASES = (
    "t17_index_expression::native_functional_index_requires_override_and_preserves_unique_behavior",
    "t12_required_ddl::native_required_check_ddl_failure_persists_failed_report_after_copy",
)
MYSQL80_NO_POSTGIS_REQUIRED_CASES = (
    "t17_spatial::native_spatial_requires_preinstalled_postgis_and_preserves_nothing_without_it",
)
MYSQL80_POSTGIS_REQUIRED_CASES = (
    "t17_spatial::native_spatial_values_preserve_shape_empty_null_and_srid_with_postgis",
)


def required_cases(system=None, mysql_lane=None, postgres_lane=None):
    system = platform.system() if system is None else system
    mysql_lane = mysql_lane or "mysql84"
    if mysql_lane in ("mysql80", "mysql84") and postgres_lane is None:
        postgres_lane = "pg16"
    cases = REQUIRED_CASES + (MACOS_REQUIRED_CASES if system == "Darwin" else ())
    if mysql_lane in ("mysql80", "mysql84"):
        cases += MYSQL_80_REQUIRED_CASES
        if postgres_lane == "pg16":
            cases += MYSQL80_NO_POSTGIS_REQUIRED_CASES
        elif postgres_lane == "pg16-postgis":
            cases += MYSQL80_POSTGIS_REQUIRED_CASES
    plan = mysql_case_plan(mysql_lane, cases)
    return tuple(case for case in cases if plan[case][0] == "run")


def unresolved_mysql_cases(plan):
    return sorted(case for case, (status, _) in plan.items() if status == "pending")


def platform_unavailable_case_reasons(system=None):
    system = platform.system() if system is None else system
    if system == "Darwin":
        return {}
    return {case: "test source is gated by target_os=macos" for case in MACOS_ONLY_CASES}


def runnable_mysql_case_ids(plan, system=None):
    unavailable = platform_unavailable_case_reasons(system)
    return tuple(case for case, (status, _) in plan.items()
                 if status == "run" and case not in unavailable)


def native_platform(machine=None):
    machine = (platform.machine() if machine is None else machine).lower()
    if machine in ("arm64", "aarch64"):
        arch = "arm64"
    elif machine in ("amd64", "x86_64"):
        arch = "amd64"
    else:
        raise ValueError(f"unsupported native runner architecture: {machine}")
    if platform.system() not in ("Darwin", "Linux"):
        raise ValueError(f"unsupported native runner operating system: {platform.system()}")
    return f"linux/{arch}"


def pin_variants(entry):
    """Read a legacy one-platform pin or explicit platform-keyed variants."""
    if not isinstance(entry, dict):
        return {}
    variants = entry.get("platforms")
    if variants is None:
        platform_name = entry.get("platform")
        return {platform_name: entry} if isinstance(platform_name, str) else {}
    if not isinstance(variants, dict):
        return {}
    return variants


def select_pin(pins, lane, kind, platform_name):
    entry = pins.get(lane) if isinstance(pins, dict) else None
    variants = pin_variants(entry)
    pin = variants.get(platform_name)
    if not isinstance(pin, dict):
        available = ", ".join(sorted(variants)) or "none"
        raise ValueError(f"{lane}: no reviewed {platform_name} image pin (available: {available})")
    if pin.get("platform") != platform_name or pin.get("kind") != kind:
        raise ValueError(f"{lane}: pin kind/platform does not match {kind} on {platform_name}")
    return pin


def command(args, *, env=None, data=None, check=True):
    result = subprocess.run(args, cwd=ROOT, env=env, input=data, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if check and result.returncode:
        raise RuntimeError(f"{shlex.join(args)} failed ({result.returncode}):\n{result.stderr}")
    return result


def certificates(path):
    path.mkdir()
    common = ["openssl", "req", "-newkey", "rsa:2048", "-nodes"]
    command(common + ["-x509", "-days", "2", "-subj", "/CN=my2pg integration CA",
                      "-keyout", str(path / "ca-key.pem"), "-out", str(path / "ca.pem")])
    for name, subject, extension in [
        ("server", "localhost", "subjectAltName=DNS:localhost,DNS:mysql,DNS:postgres,IP:127.0.0.1\nextendedKeyUsage=serverAuth"),
        ("client", "my2pg client", "extendedKeyUsage=clientAuth"),
        ("wrong-client", "different client", "extendedKeyUsage=clientAuth"),
        ("wrong-host", "wrong.invalid", "subjectAltName=DNS:wrong.invalid\nextendedKeyUsage=serverAuth"),
    ]:
        command(common + ["-subj", f"/CN={subject}", "-keyout", str(path / f"{name}-key.pem"),
                          "-out", str(path / f"{name}.csr")])
        ext = path / f"{name}.ext"
        ext.write_text(extension + "\n")
        command(["openssl", "x509", "-req", "-days", "2", "-in", str(path / f"{name}.csr"),
                 "-CA", str(path / "ca.pem"), "-CAkey", str(path / "ca-key.pem"),
                 "-CAcreateserial", "-extfile", str(ext), "-out", str(path / f"{name}.pem")])
    command(common + ["-x509", "-days", "2", "-subj", "/CN=untrusted integration CA",
                      "-keyout", str(path / "untrusted-ca-key.pem"), "-out", str(path / "untrusted-ca.pem")])
    command(common + ["-subj", "/CN=my2pg client", "-keyout", str(path / "untrusted-client-key.pem"),
                      "-out", str(path / "untrusted-client.csr")])
    command(["openssl", "x509", "-req", "-days", "2", "-in", str(path / "untrusted-client.csr"),
             "-CA", str(path / "untrusted-ca.pem"), "-CAkey", str(path / "untrusted-ca-key.pem"),
             "-CAcreateserial", "-extfile", str(path / "client.ext"), "-out", str(path / "untrusted-client.pem")])
    (path / "ca-index").write_text("")
    (path / "ca-serial").write_text("01\n")
    ca_config = path / "ca-signing.conf"
    ca_config.write_text(f"[ca]\ndefault_ca=local\n[local]\ndatabase={path}/ca-index\nserial={path}/ca-serial\nnew_certs_dir={path}\ncertificate={path}/ca.pem\nprivate_key={path}/ca-key.pem\ndefault_md=sha256\npolicy=cn_only\n[cn_only]\ncommonName=supplied\n")
    for name, subject, extension in [
        ("expired-client", "my2pg client", "extendedKeyUsage=clientAuth"),
        ("expired-server", "localhost", "subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth"),
    ]:
        command(common + ["-subj", f"/CN={subject}", "-keyout", str(path / f"{name}-key.pem"),
                          "-out", str(path / f"{name}.csr")])
        ext = path / f"{name}.ext"
        ext.write_text("[usage]\n" + extension + "\n")
        signed = path / f"{name}-signed.pem"
        command(["openssl", "ca", "-batch", "-config", str(ca_config),
                 "-startdate", "200101000000Z", "-enddate", "200102000000Z", "-extensions", "usage",
                 "-extfile", str(ext), "-in", str(path / f"{name}.csr"), "-out", str(signed)])
        command(["openssl", "x509", "-in", str(signed), "-out", str(path / f"{name}.pem")])
    (path / "pg_hba.conf").write_text(
        "local all all trust\n"
        'hostssl all "my2pg client" 0.0.0.0/0 scram-sha-256 clientcert=verify-full\n'
        'hostssl all "my2pg client" ::/0 scram-sha-256 clientcert=verify-full\n'
        'hostnossl all "my2pg client" 0.0.0.0/0 reject\n'
        'hostnossl all "my2pg client" ::/0 reject\n'
        "host all all 0.0.0.0/0 scram-sha-256\n"
        "host all all ::/0 scram-sha-256\n"
    )
    # Server containers must read mounted certificates; these disposable test keys
    # confer no access beyond uniquely owned integration databases.
    for file in path.glob("*.pem"):
        file.chmod(0o644)
    for file in path.glob("*client-key.pem"):
        file.chmod(0o600)


def compose(metadata, *args, check=True):
    env = os.environ.copy()
    env.update(metadata["compose_env"])
    return command(["docker", "compose", "-f", str(COMPOSE), "-p", metadata["project"], *args],
                   env=env, check=check)


def sql(metadata, service, statement):
    if service == "mysql":
        args = ["exec", "-T", "-e", "MYSQL_PWD=integration-root", "mysql", "mysql",
                "-uroot", "--batch", "--skip-column-names"]
    else:
        args = ["exec", "-T", "postgres", "psql", "-U", "my2pg", "-d", "target", "-v", "ON_ERROR_STOP=1", "-At"]
    env = os.environ.copy()
    env.update(metadata["compose_env"])
    return command(["docker", "compose", "-f", str(COMPOSE), "-p", metadata["project"], *args],
                   env=env, data=statement).stdout.strip()


def validate_owned(metadata):
    if not re.fullmatch(r"my2pg-[a-z0-9-]+", metadata.get("project", "")):
        raise ValueError("refusing cleanup of a non-my2pg project")
    ids = command(["docker", "ps", "-aq", "--filter", "label=com.docker.compose.project=" + metadata["project"]]).stdout.split()
    for container in ids:
        item = json.loads(command(["docker", "inspect", container]).stdout)[0]
        labels = item["Config"].get("Labels", {})
        if labels.get("org.my2pg.integration") != "true" or labels.get("com.docker.compose.project") != metadata["project"]:
            raise ValueError("refusing cleanup: container ownership label mismatch")
    return ids


def stop(metadata):
    validate_owned(metadata)
    logs = compose(metadata, "logs", "--no-color", check=False)
    (Path(metadata["artifact_dir"]) / "containers.log").write_text(logs.stdout + logs.stderr)
    compose(metadata, "down", "--volumes", "--remove-orphans")
    metadata["state"] = "stopped"
    save(metadata)


def save(metadata):
    path = Path(metadata["artifact_dir"])
    (path / "connections.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (path / "env.sh").write_text("\n".join(f"export {key}={shlex.quote(value)}" for key, value in metadata["env"].items()) + "\n")


def start(mysql_lane, pg_lane):
    pins = json.loads(PINS.read_text())
    platform_name = native_platform()
    selected = {
        mysql_lane: select_pin(pins, mysql_lane, "mysql", platform_name),
        pg_lane: select_pin(pins, pg_lane, "postgres", platform_name),
    }
    architecture = platform_name.split("/", 1)[1]
    project = f"my2pg-{mysql_lane}-{pg_lane}-{uuid.uuid4().hex[:12]}"
    artifact = ROOT / "target/integration" / project
    artifact.mkdir(parents=True)
    certificates(artifact / "tls")
    metadata = {
        "project": project, "artifact_dir": str(artifact), "state": "starting",
        "architecture": architecture, "platform": platform_name, "lanes": [mysql_lane, pg_lane],
        "pin_keys": {mysql_lane: f"{mysql_lane}@{platform_name}", pg_lane: f"{pg_lane}@{platform_name}"},
        "images": selected,
        "compose_env": {"MY2PG_MYSQL_IMAGE": selected[mysql_lane]["image"], "MY2PG_POSTGRES_IMAGE": selected[pg_lane]["image"],
                        "MY2PG_PLATFORM": platform_name, "MY2PG_TLS_DIR": str(artifact / "tls")},
        "env": {"MY2PG_INTEGRATION": "1", "MY2PG_ARTIFACT_DIR": str(artifact)},
        "fixture_sha256": {file.name: hashlib.sha256(file.read_bytes()).hexdigest() for file in sorted(FIXTURE.glob("*.sql"))},
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    }
    save(metadata)
    try:
        compose(metadata, "up", "-d", "--wait", "--wait-timeout", "180")
        for service, port, user, database in [("mysql", "3306", "my2pg", "source"), ("postgres", "5432", "my2pg", "target")]:
            address = compose(metadata, "port", service, port).stdout.strip()
            if not re.fullmatch(r"127\.0\.0\.1:\d+", address):
                raise ValueError(f"unexpected non-loopback address: {address}")
            scheme = "mysql" if service == "mysql" else "postgresql"
            metadata["env"][f"MY2PG_{service.upper()}_URL"] = f"{scheme}://{user}:integration-only@{address}/{database}"
            if service == "mysql":
                metadata["env"]["MY2PG_MYSQL_ROOT_URL"] = f"mysql://root:integration-root@{address}/source"
            metadata[service + "_port"] = int(address.rsplit(":", 1)[1])
        for name in ["ca", "server", "client", "client-key", "untrusted-ca", "wrong-host",
                     "wrong-client", "wrong-client-key", "untrusted-client", "untrusted-client-key",
                     "expired-client", "expired-client-key", "expired-server", "expired-server-key"]:
            metadata["env"]["MY2PG_TLS_" + name.upper().replace("-", "_")] = str(artifact / "tls" / (name + ".pem"))
        metadata["env"]["MY2PG_TLS_BAD_CA"] = metadata["env"]["MY2PG_TLS_UNTRUSTED_CA"]
        metadata["versions"] = {
            "mysql": sql(metadata, "mysql", "SELECT VERSION();"),
            "postgres": sql(metadata, "postgres", "SHOW server_version;"),
        }
        for lane, service in [(mysql_lane, "mysql"), (pg_lane, "postgres")]:
            if not metadata["versions"][service].startswith(selected[lane]["version_prefix"]):
                raise ValueError(f"unexpected {service} version: {metadata['versions'][service]}")
        sql(metadata, "mysql", (FIXTURE / "mysql.sql").read_text())
        sql(metadata, "mysql", "INSERT INTO source.all_bytes VALUES (1, X'" + bytes(range(256)).hex() + "');")
        sql(metadata, "mysql", "CREATE USER 'tls_client'@'%' IDENTIFIED BY 'integration-only' REQUIRE X509; GRANT SELECT ON source.* TO 'tls_client'@'%';")
        sql(metadata, "postgres", (FIXTURE / "postgres.sql").read_text())
        if pg_lane == "pg16-postgis":
            sql(metadata, "postgres", "CREATE EXTENSION IF NOT EXISTS postgis")
        sql(metadata, "postgres", 'CREATE ROLE "my2pg client" LOGIN PASSWORD \'integration-only\';')
        smoke(metadata)
        metadata["state"] = "ready"
        save(metadata)
        return metadata
    except BaseException:
        stop(metadata)
        raise


def smoke(metadata):
    cases = [
        ("mysql", "SELECT COUNT(*) FROM source.users", "3"),
        ("mysql", "SELECT COUNT(*) FROM source.orders", "2"),
        ("mysql", "SELECT COUNT(*) FROM source.empty_table", "0"),
        ("mysql", "SELECT amount FROM source.users WHERE id=1", "1234567890123456789012.12345678"),
        ("mysql", "SELECT HEX(payload) FROM source.users WHERE id=1", "00015C09FF"),
        ("mysql", "SELECT unsigned_big FROM source.numeric_edges WHERE id=1", "18446744073709551615"),
        ("mysql", "SELECT duration FROM source.numeric_edges WHERE id=1", "-838:59:58.999999"),
        ("mysql", "SELECT HEX(value) FROM source.all_bytes", bytes(range(256)).hex().upper()),
        ("mysql", "SELECT plugin FROM mysql.user WHERE user='my2pg'",
         "mysql_native_password" if metadata["lanes"][0] == "mysql57" else "caching_sha2_password"),
        ("mysql", "SELECT COUNT(*) FROM source.existing_view", "3"),
        ("postgres", "SELECT sentinel FROM public.users WHERE id=999", "must-survive"),
        ("postgres", "SHOW ssl", "on"),
        ("postgres", "SELECT rolcanlogin FROM pg_roles WHERE rolname='my2pg client'", "t"),
        ("postgres", "SELECT COUNT(*) FROM pg_hba_file_rules WHERE auth_method='scram-sha-256' AND user_name=ARRAY['my2pg client'] AND 'clientcert=verify-full'=ANY(options) AND error IS NULL", "2"),
    ]
    results = []
    for service, statement, expected in cases:
        actual = sql(metadata, service, statement)
        if actual != expected:
            raise AssertionError(f"{service}: {statement}: expected {expected!r}, got {actual!r}")
        results.append({"service": service, "statement": statement, "passed": True})
    (Path(metadata["artifact_dir"]) / "seed-checks.json").write_text(json.dumps(results, indent=2) + "\n")


def failed_cases(output):
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED$", output, re.M))
    # --nocapture can interrupt the status line; the final list remains complete.
    for block in re.findall(r"^failures:\n((?:\n|    [A-Za-z_][\w:]*\n)+)", output, re.M):
        failed.update(line.strip() for line in block.splitlines() if line.strip())
    return sorted(failed)


def case_results(output):
    r"""Read libtest status blocks, including interrupted --nocapture lines.

    >>> case_results("test a::case ... ok\ntest result: ok. 1 passed;\n")
    {'a::case': ['ok']}
    >>> case_results("test a::case ... source probe\n(0,0)\nok\ntest result: ok. 1 passed;\n")
    {'a::case': ['ok']}
    >>> case_results("test a::case ... ignored, requires fixture\n")
    {'a::case': ['ignored']}
    >>> case_results("test a::case ... diagnostic\nFAILED\ntest result: FAILED. 0 passed;\n")
    {'a::case': ['FAILED']}
    >>> case_results("test result: ok. 100 passed; 0 failed;\n")
    {}
    >>> case_results("test a::case ... ok\ntest a::case ... ok\n")
    {'a::case': ['ok', 'ok']}
    >>> case_results("test a::case ... no completed status\n")
    {'a::case': ['unknown']}
    """
    results = {}
    blocks = re.finditer(r"^test (\S+) \.\.\. (.*?)(?=^test \S+ \.\.\. |^test result:|\Z)",
                         output, re.M | re.S)
    for block in blocks:
        statuses = re.findall(r"^(ok|FAILED|ignored)(?:[ \t]*|, [^\n]*)\r?$", block[2], re.M)
        results.setdefault(block[1], []).append(statuses[-1] if statuses else "unknown")
    return results


def artifact_executable(output):
    r"""Select the real package binary, never its libtest executable.

    >>> sample = {"reason": "compiler-artifact", "manifest_path": str(ROOT / "Cargo.toml"), "target": {"name": "my2pg", "kind": ["bin"]}, "profile": {"test": False}, "executable": sys.executable}
    >>> artifact_executable(json.dumps(sample)) == Path(sys.executable)
    True
    >>> artifact_executable('')
    Traceback (most recent call last):
    ...
    ValueError: Cargo must report exactly one normal my2pg executable
    >>> artifact_executable(json.dumps(sample) + '\n' + json.dumps(sample))
    Traceback (most recent call last):
    ...
    ValueError: Cargo must report exactly one normal my2pg executable
    >>> sample['profile']['test'] = True
    >>> artifact_executable(json.dumps(sample))
    Traceback (most recent call last):
    ...
    ValueError: Cargo must report exactly one normal my2pg executable
    >>> sample['profile']['test'] = False; sample['executable'] = None
    >>> artifact_executable(json.dumps(sample))
    Traceback (most recent call last):
    ...
    ValueError: Cargo must report exactly one normal my2pg executable
    """
    candidates = []
    for line in output.splitlines():
        if not line.lstrip().startswith("{"):
            continue
        item = json.loads(line)
        if item.get("reason") != "compiler-artifact":
            continue
        target, profile = item.get("target"), item.get("profile")
        if not isinstance(target, dict) or not isinstance(profile, dict):
            raise ValueError("malformed Cargo artifact target or profile")
        if (item.get("manifest_path") == str(ROOT / "Cargo.toml")
                and target.get("name") == "my2pg"
                and target.get("kind") == ["bin"]
                and profile.get("test") is False
                and isinstance(item.get("executable"), str) and item["executable"]):
            candidates.append(Path(item["executable"]))
    if len(candidates) != 1:
        raise ValueError("Cargo must report exactly one normal my2pg executable")
    executable = candidates[0]
    if not executable.is_absolute() or not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError("Cargo reported an unavailable or non-absolute my2pg executable")
    return executable


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mysql_lane", nargs="?", default="mysql84")
    parser.add_argument("postgres_lane", nargs="?", default="pg16")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--start", action="store_true", help="leave databases running; print connection JSON path")
    modes.add_argument("--smoke", action="store_true", help="assert seed/database setup, then stop; does not claim Rust integration")
    modes.add_argument("--stop", type=Path, help="stop exactly the owned project in this connection JSON")
    args = parser.parse_args()
    if args.stop:
        stop(json.loads(args.stop.read_text()))
        return
    if not args.smoke and not args.start:
        plan = mysql_case_plan(args.mysql_lane, MYSQL_CASE_INVENTORY)
        pending = unresolved_mysql_cases(plan)
        if pending:
            raise RuntimeError(
                f"{args.mysql_lane} has {len(pending)} pending test applicability decisions; "
                "review every case before starting databases: " + ", ".join(pending[:8])
            )
        skip_selectors = mysql_case_skip_selectors(plan)
    else:
        plan, skip_selectors = {}, []
    metadata = start(args.mysql_lane, args.postgres_lane)
    print(Path(metadata["artifact_dir"]) / "connections.json", flush=True)
    if args.start:
        return
    try:
        if not args.smoke:
            env = os.environ.copy()
            env.update(metadata["env"])
            cargo = ROOT / "bin/cargo"
            build_args = ["test", "--offline", "--locked", "--features", NATIVE_FEATURES,
                          "--tests", "--no-run", "--message-format=json"]
            build = command([str(cargo), *build_args], check=False)
            artifact_dir = Path(metadata["artifact_dir"])
            (artifact_dir / "rust-build.jsonl").write_text(build.stdout)
            (artifact_dir / "rust-build.stderr.log").write_text(build.stderr)
            build_evidence = {"command": ["bin/cargo", *build_args], "exit_code": build.returncode,
                              "artifact_executable": None,
                              "log_sha256": hashlib.sha256((build.stdout + build.stderr).encode()).hexdigest()}
            try:
                if build.returncode:
                    raise RuntimeError(f"native test preparation failed ({build.returncode}); inspect rust-build logs")
                executable = artifact_executable(build.stdout)
                build_evidence["artifact_executable"] = str(executable)
                build_evidence["artifact_executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
            finally:
                (artifact_dir / "rust-build.json").write_text(json.dumps(build_evidence, indent=2) + "\n")
            env["MY2PG_TEST_ARTIFACT_EXE"] = str(executable)
            # Global event-trigger DDL must not overlap another case inspecting it.
            test_args = ["test", "--locked", "--features", NATIVE_FEATURES, "--tests", "--", "--ignored", "--nocapture", "--test-threads=1"]
            for selector in skip_selectors:
                test_args.extend(["--skip", selector])
            result = command([str(cargo), *test_args], env=env, check=False)
            (Path(metadata["artifact_dir"]) / "rust-tests.log").write_text(result.stdout + result.stderr)
            passed = sum(int(count) for count in re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed;", result.stdout))
            observed = case_results(result.stdout)
            mysql_lane = metadata["lanes"][0]
            postgres_lane = metadata["lanes"][1]
            required = {case: observed.get(case, []) for case in required_cases(
                mysql_lane=mysql_lane, postgres_lane=postgres_lane)}
            unmet = [case for case, statuses in required.items() if statuses != ["ok"]]
            runnable_ids = runnable_mysql_case_ids(plan, system=platform.system())
            runnable = {case: observed.get(case, []) for case in runnable_ids}
            unmet_runnable = [case for case, statuses in runnable.items() if statuses != ["ok"]]
            platform_unavailable = platform_unavailable_case_reasons(platform.system())
            unclassified = sorted(set(observed) - set(MYSQL_CASE_INVENTORY))
            unexpectedly_run = sorted(case for case, (status, _) in plan.items()
                                      if status == "not_applicable" and case in observed)
            evidence = {"command": ["bin/cargo", *test_args],
                        "exit_code": result.returncode, "passed_cases": passed,
                        "preparation": build_evidence,
                        "failed_case_ids": failed_cases(result.stdout),
                        "required_case_results": required, "unmet_required_case_ids": unmet,
                        "runnable_case_results": runnable, "unmet_runnable_case_ids": unmet_runnable,
                        "platform_unavailable_case_reasons": platform_unavailable,
                        "case_applicability": {case: {"status": status, "reason": reason}
                                               for case, (status, reason) in plan.items()},
                        "not_applicable_case_ids": [case for case, (status, _) in plan.items()
                                                     if status == "not_applicable"],
                        "unclassified_case_ids": unclassified,
                        "unexpectedly_run_not_applicable_case_ids": unexpectedly_run,
                        "host": platform.system(),
                        "binary_revision": command(["git", "rev-parse", "HEAD"]).stdout.strip(),
                        "working_tree_dirty": bool(command(["git", "status", "--porcelain"]).stdout.strip()),
                        "lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
                        "fixture_sha256": metadata["fixture_sha256"], "harness_sha256": metadata["harness_sha256"],
                        "log_sha256": hashlib.sha256((result.stdout + result.stderr).encode()).hexdigest()}
            (Path(metadata["artifact_dir"]) / "rust-tests.json").write_text(json.dumps(evidence, indent=2) + "\n")
            print(result.stdout, end="")
            print(result.stderr, file=sys.stderr, end="")
            if result.returncode:
                raise RuntimeError(f"requested integration tests failed ({result.returncode})")
            if unmet:
                raise RuntimeError("required native cases did not each execute and pass exactly once: " + ", ".join(unmet))
            if unmet_runnable:
                raise RuntimeError("runnable native cases did not each execute and pass exactly once: " + ", ".join(unmet_runnable))
            if unclassified:
                raise RuntimeError("discovered tests lack MySQL version dispositions: " + ", ".join(unclassified))
            if unexpectedly_run:
                raise RuntimeError("non-applicable tests unexpectedly executed: " + ", ".join(unexpectedly_run))
            if passed == 0:
                raise RuntimeError("requested integration discovered no passing cases")
    finally:
        stop(metadata)


if __name__ == "__main__":
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        main()
    except (RuntimeError, ValueError, AssertionError, OSError) as error:
        print(f"integration error: {error}", file=sys.stderr)
        sys.exit(1)
