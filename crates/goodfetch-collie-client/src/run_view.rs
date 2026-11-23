//! A view of a Run another program started (ADR 0102), such as a detached
//! one. A Connection's `open_run` opens it by the Run's ID with no round
//! trip. It takes no writer role, and closing it frees only the view: the
//! Run goes on. Python's `run_view.py` and TypeScript's `run-view.ts` are
//! its twins.

use goodfetch_protocol::run::RunId;

use crate::connection::ClientConnection;
use crate::error::Error;

/// A Run named by its Run ID, opened as a view: one another program
/// started, such as a detached Run. It takes no writer role.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a view is the Run's ID; closing it frees only the view"]
pub struct RunView {
    run_id: RunId,
}

impl RunView {
    /// The Run's ID, which names it for good. Read without a round trip.
    pub fn id(&self) -> &RunId {
        &self.run_id
    }

    /// Closes the view. Frees only the view: the Run goes on.
    pub fn close(self) {}
}

impl ClientConnection {
    /// Opens a view of the Run `run_id` names, one another program started
    /// such as a detached Run, with no round trip (ADR 0102). The view takes
    /// no writer role, and closing it frees only the view.
    ///
    /// # Errors
    ///
    /// `closed` once the Connection is closed, and `connection_forked` in
    /// another process than the one that opened it.
    pub fn open_run(&self, run_id: &RunId) -> Result<RunView, Error> {
        self.ensure_opener()?;
        if self.is_closed() {
            return Err(Error::closed());
        }
        Ok(RunView {
            run_id: run_id.clone(),
        })
    }
}

/// C-SEND-SYNC: a view moves between threads, is shared between them and
/// outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<RunView>();
};
