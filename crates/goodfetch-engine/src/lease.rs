//! The lease check (ADRs 0003 and 0087): the interface every outgoing
//! browser or network command the engine sends for a work item passes
//! first, so a handler past its lease sends nothing more through the
//! engine. It is Collie's: area 5 builds it into the page, where a page a
//! program opens leaves it empty, and Foxhound fills it with the work
//! item's lease when a handler gets its page. The run epoch and the commit
//! check under it are `run_state`'s.

use std::fmt::Debug;

use crate::CallError;

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

    /// The error a call raises once its page withheld the command `method`
    /// because the lease ended as `ended`: the Product's own Error code,
    /// since the page that withholds it is Collie's and names none of
    /// Foxhound's (ADR 0087).
    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError;

    /// Holds the lease while its page waits for a navigation gate's permit
    /// for `url`, a wait that counts against no timeout (ADR 0093): the
    /// lease is renewed and the Run's live status names the wait until the
    /// answer is dropped. A lease with nothing to renew answers
    /// [`PermitWait::none`].
    fn permit_wait(&self, url: &str) -> PermitWait {
        let _ = url;
        PermitWait::none()
    }
}

/// A wait for a navigation gate's permit, under way until dropped.
#[must_use = "the wait ends when this is dropped"]
pub struct PermitWait(Option<Box<dyn Send + Sync>>);

impl PermitWait {
    /// A wait that holds `held` until it ends, which ends the hold once
    /// dropped.
    pub fn holding(held: impl Send + Sync + 'static) -> Self {
        Self(Some(Box::new(held)))
    }

    /// A wait with nothing to hold.
    pub fn none() -> Self {
        Self(None)
    }
}

impl Debug for PermitWait {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("PermitWait")
            .field(&self.0.is_some())
            .finish()
    }
}
