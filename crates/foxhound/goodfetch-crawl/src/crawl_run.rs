//! A crawl's record of its live Run (ADR 0145): the Run's identity and the
//! powers the crawl uses, taken once from the Run the registry hands the
//! crawl as it starts or resumes it, with the gate in admission the
//! composition passes in. Each of the crawl's functions takes the record,
//! or only the identity or the one power it uses. The record also carries
//! the page crate's record of the same Run, which the fetch tier's
//! interfaces take.

use std::sync::Arc;

use goodfetch_engine::CallContext;
use goodfetch_engine::admission::{self, Admission, AdmissionError, Gates};
use goodfetch_engine::entropy::RunEntropy;
use goodfetch_engine::governor::RunGovernor;
use goodfetch_engine::ledger::RunCharging;
use goodfetch_engine::politeness::RunPoliteness;
use goodfetch_engine::robots_txt::OwnCopies;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::run_trace::RunTrace;
use goodfetch_engine::runs::{
    CleanUpRefresher, Progress, ReportWarnings, RunEnd, RunKeys, RunParts, Switched,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::BrowserRun;
use goodfetch_protocol::run::RunKind;

/// A crawl's record of its live Run, which its delivery, its attempts and
/// the calls that reach the live crawl share.
pub(crate) struct CrawlRun {
    /// Its identity.
    pub(crate) identity: RunIdentity,
    /// Its run file's writer.
    pub(crate) state: RunState,
    /// Its charging.
    pub(crate) charging: RunCharging,
    /// Its Secrets and data key.
    pub(crate) keys: RunKeys,
    /// Its Run report's warnings.
    pub(crate) warnings: ReportWarnings,
    /// Its end, with whether `run.finish` let it finish.
    pub(crate) end: RunEnd,
    /// Its part of the Resource governor.
    pub(crate) governor: RunGovernor,
    /// Its entropy.
    pub(crate) entropy: RunEntropy,
    /// The Politeness it shares.
    pub(crate) politeness: RunPoliteness,
    /// Its progress.
    pub(crate) progress: Progress,
    /// Its Run trace.
    pub(crate) trace: RunTrace,
    /// Its own robots.txt copies.
    pub(crate) own_robots_txt: OwnCopies,
    /// The gate in admission the composition passes in, if any.
    gates: Option<Arc<dyn Gates>>,
    /// The page crate's record of the Run, which the fetch tier's
    /// interfaces take.
    pub(crate) browser: Arc<BrowserRun>,
    /// Handle to adopt newer Clean-up rules versions at Safe points under
    /// `clean_up.follow_latest` (ADRs 0047 and 0139).
    clean_up_refresher: Option<CleanUpRefresher>,
}

impl CrawlRun {
    /// The record of the Run whose `parts` the registry handed the crawl
    /// in `context`.
    pub(crate) fn of(parts: &RunParts, context: &CallContext) -> Arc<Self> {
        Arc::new(Self {
            identity: parts.identity.clone(),
            state: parts.state.clone(),
            charging: parts.charging.clone(),
            keys: parts.keys.clone(),
            warnings: parts.warnings.clone(),
            end: parts.end.clone(),
            governor: parts.governor.clone(),
            entropy: parts.entropy.clone(),
            politeness: parts.politeness.clone(),
            progress: parts.progress.clone(),
            trace: parts.trace.clone(),
            own_robots_txt: parts.own_robots_txt.clone(),
            gates: context.runs().admission_gates().cloned(),
            browser: BrowserRun::of(parts, context.engine()),
            clean_up_refresher: context.runs().clean_up_refresher(parts, RunKind::Crawl),
        })
    }

    /// Checks for and adopts a newly published Clean-up rules version at a
    /// Safe point when `clean_up.follow_latest` is enabled (ADRs 0047 and
    /// 0139).
    pub(crate) async fn refresh_clean_up_rules(&self) -> Option<Switched> {
        let refresher = self.clean_up_refresher.as_ref()?;
        refresher.refresh().await
    }

    /// The Run's admission under `settings`: [`admission::of_run`] over its
    /// run file's writer, its clock, its entropy and the Politeness it
    /// shares, through the composition's gate.
    ///
    /// # Errors
    ///
    /// As [`admission::of_run`].
    ///
    /// # Cancel safety
    ///
    /// As [`admission::of_run`]'s.
    pub(crate) async fn admission(
        &self,
        settings: &Resolved,
    ) -> Result<Option<Admission>, AdmissionError> {
        admission::of_run(
            self.gates.as_ref(),
            (&self.state, self.identity.clock()),
            &self.entropy,
            &self.politeness,
            settings,
        )
        .await
    }
}
