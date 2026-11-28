//! Per-Run Wall allowance state and compiled rules (ADR 0052; ticket #757).
//!
//! Kept in a leaf module so [`crate::browser_run::BrowserRun`] can own
//! [`RunWalls`] without introducing a module dependency cycle with
//! [`crate::walls`].
#![expect(
    clippy::disallowed_types,
    reason = "RunWalls locks a small in-memory table for single set lookups/inserts, never held across `.await` or another lock (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use goodfetch_clean_up_rules::walls::WallRules;
use goodfetch_protocol::page::SessionId;

/// Reason a paywall refetch without site data was skipped (ADR 0052).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefetchSkipReason {
    LoggedIn,
    Identified,
    ExitNotKept,
    AlreadyRefetched,
    StoppedForDomain,
}

impl RefetchSkipReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LoggedIn => "logged_in",
            Self::Identified => "identified",
            Self::ExitNotKept => "exit_not_kept",
            Self::AlreadyRefetched => "already_refetched",
            Self::StoppedForDomain => "stopped_for_domain",
        }
    }
}

/// Context needed to decide whether a Session may refetch a page without site
/// data.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RefetchEligibility {
    pub(crate) logged_in: bool,
    pub(crate) identified: bool,
    pub(crate) exit_kept: bool,
}

#[derive(Default)]
struct RunWallsState {
    refetched_pages: BTreeSet<String>,
    stopped_domains: BTreeSet<String>,
    cleared_sessions: BTreeMap<SessionId, BTreeSet<String>>,
    audited_domains: BTreeSet<String>,
}

/// Per-Run Wall allowance state and compiled rules, held on [`crate::BrowserRun`].
#[derive(Clone)]
pub struct RunWalls {
    rules: Arc<WallRules>,
    state: Arc<Mutex<RunWallsState>>,
}

impl Default for RunWalls {
    fn default() -> Self {
        Self {
            rules: Arc::new(WallRules::embedded()),
            state: Arc::new(Mutex::new(RunWallsState::default())),
        }
    }
}

impl RunWalls {
    /// Compiled generic Wall detection rules, built once when the Run starts.
    #[must_use]
    pub fn rules(&self) -> &WallRules {
        &self.rules
    }

    /// Decides whether `url` on registrable `domain` in `session` may perform
    /// its one refetch without site data; on `Ok(())`, reserves the page and
    /// records `session` as having cleared `domain`.
    pub(crate) fn try_begin_refetch(
        &self,
        url: &str,
        domain: &str,
        session: &SessionId,
        eligibility: RefetchEligibility,
    ) -> Result<(), RefetchSkipReason> {
        if eligibility.logged_in {
            return Err(RefetchSkipReason::LoggedIn);
        }
        if eligibility.identified {
            return Err(RefetchSkipReason::Identified);
        }
        if !eligibility.exit_kept {
            return Err(RefetchSkipReason::ExitNotKept);
        }
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.stopped_domains.contains(domain) {
            return Err(RefetchSkipReason::StoppedForDomain);
        }
        if !guard.refetched_pages.insert(url.to_owned()) {
            return Err(RefetchSkipReason::AlreadyRefetched);
        }
        guard
            .cleared_sessions
            .entry(session.clone())
            .or_default()
            .insert(domain.to_owned());
        Ok(())
    }

    /// Marks `domain` as stopped for refetching for the rest of this Run.
    pub(crate) fn mark_stopped(&self, domain: &str) {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.stopped_domains.insert(domain.to_owned());
    }

    /// Whether `domain` is already stopped for refetching in this Run.
    #[cfg(test)]
    pub(crate) fn is_stopped(&self, domain: &str) -> bool {
        let guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.stopped_domains.contains(domain)
    }

    /// Whether `session` previously cleared site data for `domain` in this Run
    /// (ADR 0052: a later Challenge on that site also stops refetching).
    #[cfg(test)]
    pub(crate) fn session_cleared(&self, session: &SessionId, domain: &str) -> bool {
        let guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .cleared_sessions
            .get(session)
            .is_some_and(|domains| domains.contains(domain))
    }

    /// Marks `domain` as audited for this Run; returns `true` only on the
    /// first use so the Audit log gets one entry per `(run, domain)` (ADR 0052).
    pub(crate) fn mark_audited(&self, domain: &str) -> bool {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.audited_domains.insert(domain.to_owned())
    }
}
