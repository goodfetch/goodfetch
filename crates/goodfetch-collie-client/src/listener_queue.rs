//! How many events each stream of the Connection's own Subscription holds
//! unread (ADR 0081), the bound a Product crate's `max_listener_queue`
//! sets: 1,024 by default, and refused with `setting_invalid` at 0.

use std::num::NonZeroUsize;

use goodfetch_protocol::settings::SettingInvalidDetails;

use crate::error::Error;

/// How many events each stream of the Connection's Subscription holds
/// unread, such as a Session's pages (ADR 0081): past it, the stream loses
/// each further event of its own, and once it handed those it held it ends
/// with `listener_events_lost`, while the Subscription and the other
/// streams go on. 1,024 by default, as many as the engine holds for a
/// page's own `page.on` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListenerQueue(NonZeroUsize);

impl ListenerQueue {
    /// The bound when the program sets none.
    pub const DEFAULT: Self = Self(NonZeroUsize::new(1_024).expect("invariant: 1,024 is not 0"));

    /// A queue of `bound` events, the bound a Product crate's
    /// `max_listener_queue` gives.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for 0, which would lose every event, its `key`
    /// `max_listener_queue` and its `fix` the default.
    pub fn new(bound: usize) -> Result<Self, Error> {
        NonZeroUsize::new(bound).map(Self).ok_or_else(|| {
            let reason = "a queue of 0 events would lose every event";
            let fix = format!(
                "give a whole number of 1 or more, or leave it out for its default of {}",
                Self::DEFAULT.get()
            );
            let message = format!("max_listener_queue is 0: {reason}; {fix}.");
            Error::setting_invalid(
                message,
                &SettingInvalidDetails {
                    key: "max_listener_queue".to_owned(),
                    layer: None,
                    reason: reason.to_owned(),
                    limit: None,
                    fix: Some(fix),
                },
            )
        })
    }

    /// How many events it holds.
    #[must_use]
    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl Default for ListenerQueue {
    fn default() -> Self {
        Self::DEFAULT
    }
}
