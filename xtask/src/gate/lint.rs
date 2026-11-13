//! What `just lint` reads besides the tree and the host (ADR 0135; #2090's
//! F7): the day, in its register check, and GitHub, in its quarantine check
//! and, with a GitHub token in the run's environment, in zizmor's online
//! audits. `xtask gate plan` reads the first two again before it reuses a
//! green lint on the same tree, so the reused result is the one a lint run
//! now would give, and reuses none while zizmor audits online.

use std::path::Path;

use crate::{github, quarantine, register};

/// Where the plan reads again what lint reads besides the tree and the host.
pub(super) struct Reads<'a> {
    pub(super) root: &'a Path,
    /// Now, in seconds since the Unix epoch.
    pub(super) now: u64,
    /// Whether lint's zizmor audits online, with a GitHub token of the run's
    /// environment: what those audits read cannot be read again here.
    pub(super) zizmor_online: bool,
    /// Where the quarantine check reads its tests' issues, and its token.
    pub(super) github: &'a github::Arguments,
    pub(super) token: Option<&'a str>,
}

impl Reads<'_> {
    /// `Ok` when a lint run now would read what a green one did: no Exception
    /// of the Quality register past its expiry today (ADR 0140), and each
    /// quarantined test's issue an open `flaky-test` issue (ADR 0064); or
    /// why not. What cannot be read does not hold, and neither does a lint
    /// whose zizmor audits online.
    pub(super) fn hold(&self) -> Result<(), String> {
        if self.zizmor_online {
            return Err(
                "lint's zizmor audits online with the run's GitHub token, so lint reads more than the tree"
                    .to_owned(),
            );
        }
        let today = i64::try_from(self.now)
            .ok()
            .and_then(|now| jiff::Timestamp::from_second(now).ok())
            .ok_or_else(|| format!("{} is no time to read the day from", self.now))?
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date();
        register::exceptions_hold(self.root, today)?;
        quarantine::issues_hold(self.root, self.github, self.token)
    }
}
