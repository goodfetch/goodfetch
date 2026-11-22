//! A crawl's record of its live Run (ADR 0145): the Run's identity and the
//! powers the crawl uses, taken once from the Run the registry hands the
//! crawl as it starts or resumes it, with the gate in admission the
//! composition passes in. Each of the crawl's functions takes the record,
//! or only the identity or the one power it uses. The record also carries
//! that one `Run`, which it hands only to the fetch tier's and the page
//! crate's interfaces that still take one (#2278 removes them).

use std::sync::Arc;

use goodfetch_engine::admission::{self, Admission, AdmissionError, Gates};
use goodfetch_engine::entropy::RunEntropy;
use goodfetch_engine::governor::RunGovernor;
use goodfetch_engine::ledger::RunCharging;
use goodfetch_engine::politeness::RunPoliteness;
use goodfetch_engine::robots_txt::OwnCopies;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::run_trace::RunTrace;
use goodfetch_engine::runs::{self, Progress, ReportWarnings, Run, RunEnd, RunKeys, Runs};
use goodfetch_engine::settings::Resolved;

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
    /// The Run itself, handed only to the interfaces that still take it.
    run: Run,
}

impl CrawlRun {
    /// The record of `run`, which the registry `runs` handed the crawl.
    pub(crate) fn of(run: Run, runs: &Runs) -> Arc<Self> {
        Arc::new(Self {
            identity: runs::identity(&run).clone(),
            state: run.state().clone(),
            charging: runs::charging(&run).clone(),
            keys: runs::keys(&run).clone(),
            warnings: runs::warnings(&run).clone(),
            end: runs::end(&run).clone(),
            governor: runs::governor(&run).clone(),
            entropy: runs::entropy(&run).clone(),
            politeness: runs::politeness(&run).clone(),
            progress: runs::progress(&run).clone(),
            trace: runs::run_trace(&run).clone(),
            own_robots_txt: runs::own_robots_txt(&run).clone(),
            gates: runs.admission_gates().cloned(),
            run,
        })
    }

    /// The Run, for the fetch tier's and the page crate's interfaces that
    /// still take one (#2278 removes them); nothing else reads it.
    pub(crate) fn handed_on(&self) -> &Run {
        &self.run
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
