//! Real I/O on the paused clock (TESTING.md, Time): Tokio's current-thread
//! runtime never auto-advances its paused clock while a blocking task runs,
//! so a test that must wait on a real socket holds the clock still
//! meanwhile. No timer then fires early, a name server's query timeout and
//! retransmission included, and the test moves time only where it says.

use std::future::Future;
use std::sync::mpsc;

/// Runs `future` with the paused clock held still.
#[expect(
    clippy::disallowed_methods,
    reason = "holding the paused clock still is the blocking task's whole job (TESTING.md, Time)"
)]
pub(crate) async fn held<F: Future>(future: F) -> F::Output {
    let (release, released) = mpsc::channel::<()>();
    // The blocking task ends once `release` drops, after `future`.
    tokio::task::spawn_blocking(move || released.recv().is_err());
    let _release = release;
    future.await
}
