//! A wait a case takes on something outside the Connection, such as the
//! Proving ground's observations or oracle, or a file the engine writes: read
//! it again, on the harness's own thread, until what it read holds or the wait
//! ends. `service_workers` keeps two loops of its own, which serve the engine
//! between reads.

use std::convert::Infallible;
use std::time::{Duration, Instant};

/// What `read` reads, once `done` holds of it or `wait` passed, read again
/// every `every`: the last read either way, so the case's check judges it.
pub(super) fn until<T>(
    wait: Duration,
    every: Duration,
    mut read: impl FnMut() -> T,
    done: impl Fn(&T) -> bool,
) -> T {
    let read = try_until(wait, every, || Ok::<_, Infallible>(read()), done);
    match read {
        Ok(found) => found,
        Err(never) => match never {},
    }
}

/// [`until`] for a read that can fail: a failure ends the wait with it,
/// which the case reports as what it expected and could not read.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness polls on its own thread, outside any async worker (ADR 0060's note for the harness)"
)]
pub(super) fn try_until<T, E>(
    wait: Duration,
    every: Duration,
    mut read: impl FnMut() -> Result<T, E>,
    done: impl Fn(&T) -> bool,
) -> Result<T, E> {
    let started = Instant::now();
    loop {
        let found = read()?;
        if done(&found) || started.elapsed() > wait {
            return Ok(found);
        }
        std::thread::sleep(every);
    }
}

#[cfg(test)]
mod tests;
