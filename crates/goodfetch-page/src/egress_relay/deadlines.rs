//! The relays' own deadlines, on the Run's clock (ADR 0059): how long a
//! client may take to send its request head, and the wait a connection the
//! relay opens is held to.

use std::io;
use std::time::Duration;

use goodfetch_engine::clock::Clock;

/// The longest a relay waits for a client's request head once it accepted
/// the connection: a browser sends its head at once, so a client that sends
/// none holds no task past it (deep review finding 577-8).
pub const HEAD_DEADLINE: Duration = Duration::from_secs(10);

/// What `work` gave, unless `deadline` passed on `clock` first.
///
/// # Cancel safety
///
/// As cancel-safe as `work`, which is dropped at the deadline.
pub async fn within_or_none<T>(
    clock: &Clock,
    deadline: Duration,
    work: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        done = work => Some(done),
        () = clock.sleep(deadline) => None,
    }
}

/// What `work` gave, or `TimedOut` once `deadline` passed on `clock`.
///
/// # Errors
///
/// `work`'s, and `TimedOut` at the deadline.
///
/// # Cancel safety
///
/// As [`within_or_none`].
pub async fn within<T>(
    clock: &Clock,
    deadline: Duration,
    work: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    within_or_none(clock, deadline, work)
        .await
        .unwrap_or_else(|| Err(io::ErrorKind::TimedOut.into()))
}
