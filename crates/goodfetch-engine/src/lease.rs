//! The lease check (ADRs 0003 and 0087): the interface every outgoing
//! browser or network command the engine sends for a work item passes
//! first, so a handler past its lease sends nothing more through the
//! engine. It is Collie's: area 5 builds it into the page, where a page a
//! program opens leaves it empty, and Foxhound fills it with the work
//! item's lease when a handler gets its page. The run epoch and the commit
//! check under it are `run_state`'s.

use std::fmt::Debug;

/// Why a lease lets no more commands leave.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LeaseEnded {
    /// It ran out before its attempt ended: `lease_lost`.
    #[error("the work item's lease ran out")]
    Lost,
    /// Its attempt ended and gave it back: `lease_released`.
    #[error("the work item's attempt ended")]
    Released,
}

/// What a command checks before it leaves the engine.
pub trait LeaseCheck: Debug + Send + Sync {
    /// Whether the lease still holds now, on the engine's monotonic clock.
    ///
    /// # Errors
    ///
    /// Why it ended, once it did.
    fn check(&self) -> Result<(), LeaseEnded>;
}
