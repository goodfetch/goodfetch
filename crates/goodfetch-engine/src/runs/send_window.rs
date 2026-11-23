//! A live Run's send window (ADRs 0106 and 0145): the bodies of its
//! declared Effects that a command marks sent before it goes out, and the
//! safe point's wait for them, through the window the composition passes
//! in, if any. Nothing is marked or waited for when it passes in none.

use std::pin::Pin;
use std::sync::Arc;

use tokio::time::Instant;

use crate::call_error::CallError;
use crate::lease::LeaseCheck;
use crate::run_identity::RunIdentity;
use crate::run_state::RunState;

/// ADR 0106's send window: the engine's declared Effects, which mark the
/// bodies a Run's command counts for sent before it goes out, and which a
/// Run's safe point waits for.
pub(crate) trait SendWindow: Send + Sync {
    /// Waits for each body of `run` whose owner still holds to be judged,
    /// at most `run`'s `timeout.close` and never past `by`, or until `run`
    /// stops at once, then judges each body still running as cut short,
    /// through `state`, its run file's writer.
    fn settle<'a>(
        &'a self,
        run: &'a RunIdentity,
        state: &'a RunState,
        by: Option<Instant>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

    /// Marks sent, synced, through `state`, each body of `run` declared
    /// through the attempt `lease` names, or outside any attempt when it
    /// names none or is `None`, that nothing was sent for yet, before
    /// `command` goes out.
    ///
    /// # Errors
    ///
    /// `run_state_unavailable` when the mark could not be written:
    /// `command` must not go out.
    fn sending<'a>(
        &'a self,
        run: &'a RunIdentity,
        state: &'a RunState,
        lease: Option<&'a dyn LeaseCheck>,
        command: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), CallError>> + Send + 'a>>;
}

/// A live Run's send window, which a function takes in place of the Run's
/// handle to mark its declared Effects sent before a command goes out
/// (ADR 0145).
#[derive(Clone)]
pub struct RunSendWindow {
    window: Option<Arc<dyn SendWindow>>,
    identity: RunIdentity,
    state: RunState,
}

impl std::fmt::Debug for RunSendWindow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunSendWindow")
            .field("run_id", self.identity.id())
            .field("window", &self.window.is_some())
            .finish_non_exhaustive()
    }
}

impl RunSendWindow {
    /// The send window of the Run `identity` names, whose run file `state`
    /// writes, through `window`, the composition's, if any.
    pub(super) fn new(
        window: Option<Arc<dyn SendWindow>>,
        identity: RunIdentity,
        state: RunState,
    ) -> Self {
        Self {
            window,
            identity,
            state,
        }
    }

    /// Marks sent, synced, each body of the Run declared through the
    /// attempt `lease` names, or outside any attempt when it names none or
    /// is `None`, before `command`, which may change a page or a site, goes
    /// out (ADR 0106's window). The attempt is read only when the Run has a
    /// body running, and nothing is marked when the composition passes in
    /// no window.
    ///
    /// # Errors
    ///
    /// `run_state_unavailable` when the mark could not be written:
    /// `command` must not go out.
    ///
    /// # Cancel safety
    ///
    /// Dropping its future before the write began marks nothing, and
    /// `command` must then not go out. A drop once the write was queued
    /// may leave the bodies `sent` in the journal while the engine still
    /// holds them open: the next mark writes only those still `open`
    /// there, so the first command stays the one recorded.
    pub async fn sending(
        &self,
        lease: Option<&dyn LeaseCheck>,
        command: &str,
    ) -> Result<(), CallError> {
        match &self.window {
            Some(window) => {
                window
                    .sending(&self.identity, &self.state, lease, command)
                    .await
            }
            None => Ok(()),
        }
    }

    /// The safe point's wait for the Run's declared Effects (ADR 0106):
    /// waits for each body whose owner still holds to be judged, at most
    /// its `timeout.close` and never past `by`, then judges each body still
    /// running as cut short.
    pub(crate) async fn settle(&self, by: Option<Instant>) {
        if let Some(window) = &self.window {
            window.settle(&self.identity, &self.state, by).await;
        }
    }
}
