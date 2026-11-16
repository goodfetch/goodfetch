use serde_json::json;

use super::{Side, peak, row};

#[test]
fn a_side_that_kept_or_lost_a_url_fails_the_run() {
    let figures = json!({
        "urls": 1000, "added": 1000, "duplicates": 999,
        "added_ns_per_url": 10.0, "duplicate_ns_per_url": 5.0, "links_in_memory": 1001,
    });
    let error = row(Side::Spider, 1000, figures.to_string().as_bytes(), 1)
        .expect_err("a duplicate admitted again is no exact visited set");
    assert!(error.to_string().contains("999 duplicates"), "{error}");
}

#[test]
fn a_side_that_wrote_an_error_fails_the_run_with_it() {
    let figures = json!({ "error": "link 7 was not a duplicate" });
    let error = row(Side::Foxhound, 1000, figures.to_string().as_bytes(), 1)
        .expect_err("a failed side has no row");
    assert!(
        error.to_string().contains("link 7 was not a duplicate"),
        "{error}"
    );
}

#[test]
fn foxhound_s_row_keeps_its_budget_memory_and_spilled_lookups() {
    let figures = json!({
        "urls": 1000, "added": 1000, "duplicates": 1000,
        "added_ns_per_url": 8345.0, "duplicate_ns_per_url": 1588.0,
        "budget_bytes": 67_108_864, "memory_held_bytes": 8_169_796, "spilled_lookups": 0.0,
    });
    let row = row(Side::Foxhound, 1000, figures.to_string().as_bytes(), 42).expect("an exact run");
    assert_eq!(
        (
            row.peak_bytes,
            row.budget_bytes,
            row.memory_held_bytes,
            row.spilled_lookups
        ),
        (42, Some(67_108_864), Some(8_169_796), Some(0.0))
    );
}

#[test]
fn the_peak_is_read_from_the_probe_s_lines() {
    assert_eq!(
        peak("peak-bytes=123456\nexecutable-bytes=789\n").expect("a peak"),
        123_456
    );
    assert!(peak("executable-bytes=789\n").is_err());
}
