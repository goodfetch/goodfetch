//! The guard's check script and the guard name each step alike.

use std::time::Duration;

use goodfetch_counted_work::Row;
use serde_json::json;

use super::Action;
use super::check::{CHECK, Seen};

/// Every step `check.js` waits on reads as the guard's `Step` of that
/// name, so a step renamed in the script fails here rather than leave the
/// guard's own reading of it, such as the hiding lift at `visible`, unrun.
#[test]
fn every_step_the_check_script_waits_on_reads_as_the_guards() {
    let named: Vec<&str> = CHECK
        .split("waiting(\"")
        .skip(1)
        .filter_map(|rest| rest.split_once('"').map(|(step, _)| step))
        .collect();

    let unread: Vec<&str> = named
        .iter()
        .copied()
        .filter(|step| {
            !matches!(
                Seen::read(&json!({ "state": "waiting", "step": step, "observed": "" })),
                Ok(Seen::Waiting { step: read, .. }) if read.as_str() == *step
            )
        })
        .collect();

    assert_eq!((named.is_empty(), unread), (false, Vec::<&str>::new()));
}

/// The guard reads a step the script does not name as no step at all.
#[test]
fn a_step_the_guard_does_not_know_does_not_read() {
    let seen = Seen::read(&json!({ "state": "waiting", "step": "shown", "observed": "" }));

    assert!(seen.is_err());
}

/// Press-and-hold checks pointer readiness and routes its command counts
/// to `C3.commands.human_input` when human-like input is on.
#[test]
fn press_and_hold_checks_pointer_readiness_and_routes_human_input_counts() {
    let hold = Action::PressAndHold(Duration::from_millis(600));

    assert_eq!(hold.kind(), "pointer");
    assert_eq!(hold.commands(false), Row::ActionClickCommands);
    assert_eq!(hold.commands(true), Row::ActionHumanInputCommands);
}
