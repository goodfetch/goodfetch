//! What a test reads of a Run's trace in its home: the robots.txt readings
//! its Provenance records, which a count of what a site received waits on
//! (ADRs 0064 and 0086; `goodfetch-test-support`'s `ReceivedRequests`).
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads a Run's Provenance file and waits on it outside any async worker (ADR 0062)"
)]

use std::time::Duration;

use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::home::HomeLayout;

/// How long [`robots_txt_read`] waits for the Run's reading.
const ROBOTS_TXT_DEADLINE: Duration = Duration::from_secs(60);

/// How often [`robots_txt_read`] reads the Run's Provenance again.
const ROBOTS_TXT_POLL: Duration = Duration::from_millis(100);

/// The Run trace's Provenance entries of the Run `run_id` in the test's home
/// for a robots.txt reading.
pub fn robots_txt_readings(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let file = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_PROVENANCE);
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["action"] == json!("robots_txt"))
        .collect()
}

/// Waits until the Run `run_id` recorded a robots.txt reading, a minute at
/// most: the entry is written after the read's answer, so the read's
/// request, if it sent one, has reached the site, as it came before its
/// answer.
///
/// # Panics
///
/// When the Run recorded none within a minute.
pub fn robots_txt_read(env: &TestEnv, run_id: &str) {
    let polls = ROBOTS_TXT_DEADLINE.as_millis() / ROBOTS_TXT_POLL.as_millis();
    for _ in 0..polls {
        if !robots_txt_readings(env, run_id).is_empty() {
            return;
        }
        std::thread::sleep(ROBOTS_TXT_POLL);
    }
    panic!("the Run {run_id} recorded no robots.txt reading within a minute");
}

#[cfg(test)]
mod tests;
