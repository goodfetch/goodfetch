//! The version check of a launched build (ADR 0035): the pinned Chrome for
//! Testing, launched on a day many Chrome majors after its own, is graded
//! against the shipped release table, outdated, and the Run goes on with
//! `browser_build_outdated` in its report.

use std::time::SystemTime;

use goodfetch_recording_gate::RecordingGate;
use goodfetch_test_support::CHROME_FOR_TESTING_BUILD;
use serde_json::{Map, Value, json};

use crate::handler_page::{RUN, with_handler_page_on};

/// A day the shipped table schedules Chrome 170 for, months after the
/// pinned build's major.
fn far_ahead() -> SystemTime {
    let day: jiff::Timestamp = "2027-06-01T00:00:00Z".parse().expect("a literal timestamp");
    SystemTime::from(day)
}

/// Covers: story:8-47, story:8-49
#[test]
fn a_pinned_build_many_majors_behind_warns_and_the_run_goes_on() {
    let gate = RecordingGate::new();
    let major = CHROME_FOR_TESTING_BUILD
        .split('.')
        .next()
        .expect("a build has a major");
    with_handler_page_on(&gate, far_ahead(), Map::new(), async |program| {
        let evaluated = program
            .on_page("page.evaluate", json!({ "function": "() => 1 + 1" }))
            .await;
        let finished = program.call("run.finish", &json!({ "run_id": RUN })).await;

        let outdated: Vec<&Value> = finished["result"]["warnings"]
            .as_array()
            .unwrap_or_else(|| panic!("no Run report: {finished}"))
            .iter()
            .filter(|warning| warning["code"] == json!("browser_build_outdated"))
            .collect();
        let message = outdated
            .first()
            .and_then(|warning| warning["message"].as_str())
            .unwrap_or_default();
        assert_eq!(
            (
                evaluated["result"]["value"].clone(),
                outdated.len(),
                message.starts_with(&format!("The browser build, Chrome {major}, is outdated: ")),
                message.contains(" versions and ")
                    && message.contains(" weeks behind Chrome's Stable, "),
            ),
            (json!(2), 1, true, true),
            "{finished}"
        );
    });
}
