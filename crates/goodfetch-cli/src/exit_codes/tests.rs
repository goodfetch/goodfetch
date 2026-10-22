//! ADR 0069's exit-code table, in process (Quick suite): each Error code
//! with a row of its own exits with it, every other one with its group's,
//! the table maps every Error code the catalog declares, and no exit code
//! has two rows.
//!
//! Covers: story:11-205, story:11-206

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::cli::exit_code_table;
use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::run::{
    Completeness, Lifecycle, PointInTime, RunId, RunReport, RunUsage, Verdict,
};
use rstest::rstest;

use super::{of_error, of_report};

#[rstest]
#[case::not_in_product("not_in_product", ErrorGroup::Unavailable, 10)]
#[case::protocol_version_unsupported("protocol_version_unsupported", ErrorGroup::Unavailable, 11)]
#[case::browser_host_outdated("browser_host_outdated", ErrorGroup::Unavailable, 12)]
#[case::path_not_writable("path_not_writable", ErrorGroup::Program, 13)]
#[case::file_write_failed("file_write_failed", ErrorGroup::Unavailable, 14)]
#[case::resources_exhausted("resources_exhausted", ErrorGroup::Unavailable, 15)]
#[case::limit_reached("limit_reached", ErrorGroup::Refused, 16)]
#[case::person_required("person_required", ErrorGroup::Refused, 4)]
#[case::run_not_found("run_not_found", ErrorGroup::Program, 3)]
#[case::deadline_exceeded("deadline_exceeded", ErrorGroup::DeadlineExceeded, 7)]
#[case::call_cancelled("call_cancelled", ErrorGroup::Unavailable, 8)]
#[case::console_port_in_use("console_port_in_use", ErrorGroup::Unavailable, 8)]
#[case::engine_output_invalid("engine_output_invalid", ErrorGroup::Internal, 1)]
#[case::a_code_no_entry_declares("made_up_code", ErrorGroup::Page, 5)]
#[case::a_model_code_no_entry_declares("made_up_code", ErrorGroup::Model, 6)]
fn a_code_exits_with_its_own_row_else_its_groups(
    #[case] code: &str,
    #[case] group: ErrorGroup,
    #[case] exit: u8,
) {
    assert_eq!(of_error(code, group), exit);
}

#[test]
fn person_required_is_a_refusal_that_is_not_retryable() {
    let declared = MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .find(|definition| definition.name == "person_required")
        .map(|definition| (definition.group, definition.retryable));

    assert_eq!(declared, Some((ErrorGroup::Refused, false)));
}

#[test]
fn the_table_holds_one_row_per_exit_code() {
    let mut rows: BTreeMap<u8, Vec<&str>> = BTreeMap::new();
    for row in exit_code_table() {
        rows.entry(row.code).or_default().push(row.name);
    }

    let shared: Vec<(u8, Vec<&str>)> = rows
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .collect();

    assert_eq!(shared, []);
}

#[test]
fn the_table_maps_every_error_code_the_catalog_declares() {
    let table: Vec<u8> = exit_code_table().iter().map(|row| row.code).collect();

    let unmapped: Vec<(&str, u8)> = MODULES
        .iter()
        .flat_map(|entries| entries.error_codes)
        .map(|definition| (definition.name, of_error(definition.name, definition.group)))
        .filter(|(_, exit)| !table.contains(exit))
        .collect();

    assert_eq!(unmapped, []);
}

#[test]
fn the_table_has_the_rows_adr_0069_gives() {
    let rows: Vec<(u8, &str)> = exit_code_table()
        .iter()
        .map(|row| (row.code, row.name))
        .collect();

    assert_eq!(
        rows,
        [
            (0, "success"),
            (1, "internal"),
            (2, "usage"),
            (3, "program"),
            (4, "refused"),
            (5, "page"),
            (6, "model"),
            (7, "deadline_exceeded"),
            (8, "unavailable"),
            (10, "not_in_product"),
            (11, "protocol_version_unsupported"),
            (12, "browser_host_outdated"),
            (13, "path_not_writable"),
            (14, "file_write_failed"),
            (15, "resources_exhausted"),
            (16, "limit_reached"),
            (20, "run_stopped"),
            (21, "verdict_not_verified"),
            (130, "interrupted"),
            (143, "terminated"),
        ]
    );
}

fn report(lifecycle: Lifecycle, verdict: Verdict) -> RunReport {
    RunReport {
        run_id: RunId::parse("first").unwrap(),
        lifecycle,
        stop_reason: None,
        verdict,
        verdict_reasons: Vec::new(),
        completeness: Completeness::Complete,
        completeness_reasons: Vec::new(),
        started_at: PointInTime::from_rfc3339("2026-10-22T06:00:00Z".to_owned()),
        ended_at: PointInTime::from_rfc3339("2026-10-22T06:00:01Z".to_owned()),
        usage: RunUsage::default(),
        weakened: Vec::new(),
        plan: None,
        policy_version: None,
        settings: None,
        plugins: None,
        warnings: Vec::new(),
        recording: None,
        checkpoint: None,
        nothing_done: None,
        console_url: None,
        uncertain_effects: None,
        error: None,
        kind_parts: serde_json::Map::new(),
    }
}

#[rstest]
#[case::finished_and_verified(Lifecycle::Finished, Verdict::Verified, false, 0)]
#[case::finished_and_rejected(Lifecycle::Finished, Verdict::Rejected, false, 0)]
#[case::stopped(Lifecycle::Stopped, Verdict::Verified, false, 20)]
#[case::finished_and_verified_when_required(Lifecycle::Finished, Verdict::Verified, true, 0)]
#[case::rejected_when_required(Lifecycle::Finished, Verdict::Rejected, true, 21)]
#[case::to_review_when_required(Lifecycle::Finished, Verdict::NeedsReview, true, 21)]
#[case::stopped_to_review_when_required(Lifecycle::Stopped, Verdict::NeedsReview, true, 20)]
fn a_run_report_exits_by_how_its_run_ended(
    #[case] lifecycle: Lifecycle,
    #[case] verdict: Verdict,
    #[case] require_verified: bool,
    #[case] exit: u8,
) {
    assert_eq!(
        of_report(&report(lifecycle, verdict), require_verified),
        exit
    );
}
