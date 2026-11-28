//! A crawl's record of its live Run (ADR 0145): the Run's identity and the
//! powers the crawl uses, taken once from the Run the registry hands the
//! crawl as it starts or resumes it, with the gate in admission the
//! composition passes in. Each of the crawl's functions takes the record,
//! or only the identity or the one power it uses. The record also carries
//! the page crate's record of the same Run, which the fetch tier's
//! interfaces take.

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
use goodfetch_engine::runs::{Ending, Progress, ReportWarnings, RunEnd, RunKeys, RunParts};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::{CallContext, CallError};
use goodfetch_fetch_tier::BrowserRun;
use goodfetch_protocol::run::EndingError;
use goodfetch_sink::{Delivery, Sinks};

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
    /// Its deliveries to the Sinks its settings name, which its end
    /// finishes first.
    sinks: Delivery,
}

impl CrawlRun {
    /// The record of the Run whose `parts` the registry handed the crawl
    /// in `context`, delivering to the Sinks of `sinks` its `settings`
    /// name, once those Sinks were probed against its
    /// `crawl.min_sink_guarantee` and started delivering (ADR 0026),
    /// before the Run does any other work; a refusal ends the Run `failed`
    /// with it, having done nothing.
    ///
    /// # Errors
    ///
    /// As [`Delivery::start`].
    ///
    /// # Cancel safety
    ///
    /// As [`Delivery::start`]'s; a refused Run's ending is
    /// [`CrawlRun::end`]'s.
    pub(crate) async fn started(
        parts: &RunParts,
        context: &CallContext,
        sinks: (Sinks, &Resolved),
    ) -> Result<Arc<Self>, CallError> {
        let run = Self::of(parts, context, sinks);
        let Err(error) = run.sinks.start().await else {
            return Ok(run);
        };
        run.end(Ending::Failed(EndingError {
            code: error.code().to_owned(),
            message: error.message().to_owned(),
            details: error.details().cloned().unwrap_or_default(),
        }))
        .await;
        Err(error)
    }

    /// The record of the Run whose `parts` the registry handed the crawl
    /// in `context`, delivering to the Sinks of `sinks` its `settings`
    /// name, which [`CrawlRun::started`] starts.
    fn of(
        parts: &RunParts,
        context: &CallContext,
        (sinks, settings): (Sinks, &Resolved),
    ) -> Arc<Self> {
        let daemon = context
            .is_daemon()
            .then(|| Arc::clone(context.engine().owner()));
        Arc::new(Self {
            sinks: sinks.deliver(parts, settings, context.runs(), daemon),
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
        })
    }

    /// Returns a Sink failure reason if `crawl.on_sink_failure` is `stop` and
    /// a Sink has already failed.
    pub(crate) fn sink_failure_reason(&self) -> Option<String> {
        self.sinks.failure_reason()
    }

    /// Resolves once any Sink fails under `crawl.on_sink_failure: stop`
    /// (pends forever under the default `continue`).
    pub(crate) async fn sink_failed(&self) -> String {
        self.sinks.failed().await
    }

    /// Ends the Run as `ending` says, once its Sinks hold what its run
    /// file committed (ADR 0026): [`RunEnd::end`] after
    /// [`Delivery::finish`], which swaps a `replace` Sink's Records in
    /// when the Run finished.
    ///
    /// # Cancel safety
    ///
    /// As [`Delivery::finish`]'s, then as [`RunEnd::end`]'s.
    pub(crate) async fn end(&self, ending: Ending) {
        self.sinks
            .finish(matches!(ending, Ending::Finished(_)))
            .await;
        self.end.end(ending).await;
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
        let replaying = self
            .browser
            .engine
            .recordings()
            .is_replaying(self.identity.id());
        admission::of_run(
            self.gates.as_ref(),
            (&self.state, self.identity.clock()),
            &self.entropy,
            &self.politeness,
            settings,
            replaying,
        )
        .await
    }
}
