//! The process that opened a Connection, whose threads alone run its tasks
//! (ADR 0078): in another, such as the child of a `fork`, they never run,
//! so nothing handed to them is sent. Each use compares the process ID with
//! the opener's, so nothing is registered to run at a fork.

use crate::error::Error;

/// The process that opened a Connection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpeningProcess(u32);

impl OpeningProcess {
    /// The calling process.
    pub(crate) fn current() -> Self {
        Self(std::process::id())
    }

    /// Whether it is the calling process.
    pub(crate) fn is_current(self) -> bool {
        std::process::id() == self.0
    }

    /// `connection_forked` unless it is the calling process.
    pub(crate) fn ensure_current(self) -> Result<(), Error> {
        if self.is_current() {
            Ok(())
        } else {
            Err(Error::connection_forked())
        }
    }
}
