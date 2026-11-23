//! How a Daemon closes one of its Connections under the calls in flight on
//! it (ADR 0113).

use std::sync::{Arc, OnceLock};

use tokio_util::sync::CancellationToken;

use crate::call_error::CallError;

/// How a Daemon closes one of its Connections under the calls in flight
/// on it (ADR 0113): each call the close cancels raises the error it was
/// closed with, `call_cancelled` naming the Connection.
#[derive(Clone, Default)]
pub(crate) struct Closing {
    closed: CancellationToken,
    error: Arc<OnceLock<CallError>>,
}

impl Closing {
    /// Closes the Connection with `error`; a later close changes nothing.
    pub(crate) fn close(&self, error: CallError) {
        let _ = self.error.set(error);
        self.closed.cancel();
    }

    /// Ends once the Connection is closed.
    pub(crate) async fn closed(&self) {
        self.closed.cancelled().await;
    }

    /// What a call the close cancelled raises, once it is closed.
    pub(crate) fn error(&self) -> Option<CallError> {
        self.error.get().cloned()
    }
}
