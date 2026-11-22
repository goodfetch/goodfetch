//! The end of a live Run (ADR 0145): the command that asks the Runs'
//! registry's loop to end it as its driver says, with the signal that
//! `run.finish` let it finish. The driver that ends a Run is the one that
//! waits to be let finish, and the registry alone sets the signal.

use goodfetch_protocol::run::RunId;
use tokio::sync::{mpsc, watch};

use super::{Command, End, Ending};

/// The end of a live Run, which a function takes in place of the Run's
/// handle to end it or to wait until it is let finish (ADR 0145).
#[derive(Clone)]
pub struct RunEnd {
    run_id: RunId,
    commands: mpsc::Sender<Command>,
    /// Whether `run.finish` let it finish, which only the registry sets.
    finish: watch::Receiver<bool>,
}

impl std::fmt::Debug for RunEnd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunEnd")
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

impl RunEnd {
    /// The end of the Run `run_id`, sent to the registry through
    /// `commands`, let finish once `finish` says so.
    pub(super) fn new(
        run_id: RunId,
        commands: mpsc::Sender<Command>,
        finish: watch::Receiver<bool>,
    ) -> Self {
        Self {
            run_id,
            commands,
            finish,
        }
    }

    /// Whether `run.finish` let the Run finish: a driver that waits for new
    /// work while it has none, as a keep-alive crawl does, then ends it once
    /// its work is done.
    pub fn is_let_finish(&self) -> bool {
        *self.finish.borrow()
    }

    /// Answers once `run.finish` lets the Run finish.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing; a later call answers the same.
    pub async fn let_finish(&self) {
        let mut finish = self.finish.clone();
        if finish.wait_for(|finish| *finish).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Ends the Run as its driver says, once: its tasks stop, and its Run
    /// report is written and given to whoever waits for it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the end was sent still ends the Run.
    pub async fn end(&self, ending: Ending) {
        let end = match ending {
            Ending::Finished(reasons) => End::Finished(reasons),
            Ending::Failed(error) => End::Failed(error),
            Ending::Stopped(reason) => End::Stopped(reason),
        };
        let _ = self
            .commands
            .send(Command::End(self.run_id.clone(), end))
            .await;
    }
}
