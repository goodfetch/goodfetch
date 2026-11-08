//! The guard's check script and the guard name each step alike.

use serde_json::json;

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
