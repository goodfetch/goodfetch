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
//! its count after `goodfetch-test-client`'s `robots_txt_read`. `xtask
//! received-requests` refuses a read of the observations' requests anywhere
//! else.

use serde_json::Value;

use crate::proving_ground_format::ReceivedRequests;

impl ReceivedRequests {
    /// The requests of a Proving ground's `observations`.
    ///
    /// # Panics
    ///
    /// When the observations hold no list of requests, or a request the
    /// Proving ground's format does not describe.
    pub fn of(observations: &Value) -> Self {
        Self::read(observations).unwrap_or_else(|error| {
            panic!("the observations' requests cannot be read ({error}): {observations}")
        })
    }
}

#[cfg(test)]
mod tests;
