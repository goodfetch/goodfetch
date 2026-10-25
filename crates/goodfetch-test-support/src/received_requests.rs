//! The requests a Proving ground's page hosts received, as a test counts
//! them (ADR 0064: a count waits on a named state, never on luck).
//!
//! A site sees requests a case never asked for: the robots.txt the engine
//! reads of each document's origin, as a task of the Run that nothing the
//! case calls waits for (ADR 0086), and the icon Chromium asks a site for on
//! its own. Either may land before or after any count a test takes, so a
//! count over every request is a race under load (fix-queue 53, 57, 64 and
//! 71, and #1608). [`ReceivedRequests::of_the_case`] leaves them out; a test
//! whose claim covers them too, such as "nothing leaves" or a robots.txt
//! test, reads [`ReceivedRequests::every`], and where order matters takes
//! its count after [`robots_txt_read`]. `xtask received-requests` refuses a
//! read of the observations' requests anywhere else.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads a Run's Provenance file and waits on it outside any async worker (ADR 0062)"
)]

use std::time::Duration;

use serde_json::{Value, json};

use crate::env::TestEnv;

/// The targets the engine and the browser ask a site for on their own: the
/// robots.txt read of a document's origin (ADR 0086) and Chromium's icon.
pub const OWN_TARGETS: [&str; 2] = ["/robots.txt", "/favicon.ico"];

/// How long [`robots_txt_read`] waits for the Run's reading.
const ROBOTS_TXT_DEADLINE: Duration = Duration::from_secs(60);

/// How often [`robots_txt_read`] reads the Run's Provenance again.
const ROBOTS_TXT_POLL: Duration = Duration::from_millis(100);

/// Each request the Proving ground's page hosts received, in arrival order,
/// as `GET /observations` lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReceivedRequests(Vec<Value>);

impl ReceivedRequests {
    /// The requests of a Proving ground's `observations`.
    ///
    /// # Panics
    ///
    /// When the observations hold no list of requests.
    pub fn of(observations: &Value) -> Self {
        Self(
            observations["requests"]
                .as_array()
                .unwrap_or_else(|| panic!("the observations hold no requests: {observations}"))
                .clone(),
        )
    }

    /// The requests the case is about: every one but those for
    /// [`OWN_TARGETS`], which the engine and the browser send on their own,
    /// whenever they like.
    pub fn of_the_case(&self) -> impl Iterator<Item = &Value> {
        self.0.iter().filter(|request| {
            !OWN_TARGETS
                .iter()
                .any(|target| request["target"] == json!(target))
        })
    }

    /// Every request, the robots.txt reads and icons included, for a claim
    /// about them or about all a site saw. A count over them is taken at a
    /// named state, such as after [`robots_txt_read`].
    pub fn every(&self) -> impl Iterator<Item = &Value> {
        self.0.iter()
    }
}

/// The Run trace's Provenance entries of the Run `run_id` in the test's home
/// for a robots.txt reading.
pub fn robots_txt_readings(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let file = env
        .home()
        .join("0.0")
        .join("runs")
        .join(run_id)
        .join("trace")
        .join("provenance.jsonl");
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
