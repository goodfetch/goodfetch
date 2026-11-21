//! A raw call's line in its Run's trace (ADR 0099, G8): a call a program
//! made through its SDK's raw `call`, whose `_meta` marks it raw, is
//! written to the trace of the Run it names, by its Run ID, its Session or
//! its page, once it ends, with how it ended. Its handler runs under the
//! raw mark, so a request it sends on to the Run's owner is marked raw
//! too. A generated call never comes here: its answer is not wrapped, and
//! it pays nothing for the mark.

use std::sync::Arc;

use goodfetch_protocol::run::RunId;
use jiff::Timestamp;
use tokio_util::sync::CancellationToken;

use super::cancelled_by_client;
use crate::call_error::CallError;
use crate::composition::{ConnectionContext, GuardAnswer, RunOfCall};
use crate::run_trace::RawCallLine;
use crate::runs;
use crate::trace::{self, RawMark};

/// What a raw call names its Run by, taken from its params before its
/// handler runs.
#[derive(Debug, Default)]
pub(super) struct Named {
    /// Its `run_id`.
    pub(super) run: Option<String>,
    /// Its `session_id`.
    pub(super) session: Option<String>,
    /// The page it names.
    pub(super) page: Option<String>,
}

/// A raw call on its way: what its line in the Run trace needs once its
/// handler ends.
pub(super) struct RawCall {
    /// Its operation.
    pub(super) operation: &'static str,
    /// When it arrived.
    pub(super) arrived_at: Timestamp,
    /// What it names its Run by.
    pub(super) named: Named,
    /// Its Connection's handles, whose Runs hold the Run.
    pub(super) connection: Arc<ConnectionContext>,
    /// What finds the Run a call names by a page or a Session.
    pub(super) runs_of_calls: Arc<[Arc<dyn RunOfCall>]>,
    /// Its cancellation, when its operation is cancellable: a cancelled
    /// raw call is written as one.
    pub(super) cancelled: Option<CancellationToken>,
}

impl RawCall {
    /// `answer`, its handler's, run under the raw mark and written to its
    /// Run's trace once it ends. The Run is found before the handler runs,
    /// so a call that closes its page is still written; a call that ends
    /// its own Run, or waits for its end, is written by that end, before
    /// the trace closes; a call that names no live Run is not written.
    pub(super) fn recorded(self, answer: GuardAnswer) -> GuardAnswer {
        Box::pin(async move {
            let line = match self.run().await {
                Some(run) => match runs::run_trace(&run).calls().await {
                    Ok(log) => Some(Arc::new(RawCallLine::new(
                        run.id().clone(),
                        log,
                        self.operation,
                        self.arrived_at,
                    ))),
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            operation = self.operation,
                            "a raw call is not in the Run trace"
                        );
                        None
                    }
                },
                None => None,
            };
            let handler = trace::raw_call(RawMark::new(line.clone()), answer);
            let answered = match &self.cancelled {
                Some(token) => tokio::select! {
                    biased;
                    answered = handler => answered,
                    () = token.cancelled() => Err(cancelled_by_client()),
                },
                None => handler.await,
            };
            if let Some(line) = line {
                line.write(answered.as_ref().map(drop).map_err(CallError::code));
            }
            answered
        })
    }

    /// The live Run it names: by its Run ID, else its Session's, else its
    /// page's.
    async fn run(&self) -> Option<runs::Run> {
        let run_id = match &self.named.run {
            Some(run) => RunId::parse(run).ok()?,
            None => self.run_of_call().await?,
        };
        self.connection.runs.live(run_id).await.ok()
    }

    /// The Run of the Session or the page it names, as the contribution
    /// that makes them finds it.
    async fn run_of_call(&self) -> Option<RunId> {
        if let Some(session) = &self.named.session {
            return self
                .runs_of_calls
                .iter()
                .find_map(|runs| runs.session_run(session));
        }
        let page = self.named.page.as_ref()?;
        for runs in self.runs_of_calls.iter() {
            if let Some(run_id) = runs.page_run(page).await {
                return Some(run_id);
            }
        }
        None
    }
}
