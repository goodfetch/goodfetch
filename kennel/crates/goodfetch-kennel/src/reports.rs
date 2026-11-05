//! Where a task's reports go: the Client's and the settings' follower's.
//!
//! A closure takes each report at once, as the Tray's does, handing it to
//! the event loop's proxy. A sink of its own can make the task wait while
//! it is full instead: the details window's model reads its inputs from a
//! bounded channel, and a Client that waits there stops reading its
//! Connection, so the Daemon's queue for it falls back to `events.resync`
//! rather than growing (STYLE.md: every channel is bounded).

use std::future::Future;

/// Takes a task's reports of type `T`, in order.
pub trait Reports<T>: Send + Sync + 'static {
    /// Takes `value`; the task waits until the returned future ends.
    fn report(&self, value: T) -> impl Future<Output = ()> + Send;
}

/// A closure takes each report at once, and never makes the task wait.
impl<T, F> Reports<T> for F
where
    T: Send + 'static,
    F: Fn(T) + Send + Sync + 'static,
{
    fn report(&self, value: T) -> impl Future<Output = ()> + Send {
        self(value);
        std::future::ready(())
    }
}
