//! A Run's writer lock (ADRs 0031 and 0037's consequence): the exclusive OS
//! lock on the lock file beside the Run's state, which the engine that
//! writes the Run holds, so a dead engine holds none. A reader tests it with
//! a shared lock it releases at once, and a writer that finds it held,
//! perhaps only by such a test, retries it with backoff before it gives up.
//! Every writer that takes it retries it the same way, through
//! [`retried`]. Its calls block, apart from [`try_lock`] and the retries:
//! the registry runs them under its owner's blocking policy.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::clock::Clock;
use crate::durable;
use crate::entropy::Entropy;
use crate::tasks::TaskOwner;

/// The writer's lock file, beside the Run's state (ADR 0031).
pub(super) const LOCK_FILE: &str = "run.lock";

/// How many times a writer retries a held lock after its owner did not
/// take the action.
pub(super) const RETRIES: u32 = 5;

/// Each retry's wait before jitter, in microseconds: doubling, 1 s in all.
const WAITS: [u64; RETRIES as usize] = [32_258, 64_516, 129_032, 258_065, 516_129];

/// Whether a Run has a live writer, as a reader's test finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    /// An engine holds the Run's writer lock.
    Live,
    /// No engine holds it: the Run's engine ended or died.
    None,
}

/// Tests whether an engine writes the Run in `folder`, without becoming its
/// writer: it takes a shared lock on the lock file and releases it at once,
/// and creates nothing. It blocks: run it under a blocking policy.
///
/// # Errors
///
/// When the OS refuses the lock file.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub fn test_writer(folder: &Path) -> io::Result<Writer> {
    let lock = match OpenOptions::new().read(true).open(folder.join(LOCK_FILE)) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Writer::None),
        Err(error) => return Err(error),
    };
    match lock.try_lock_shared() {
        Ok(()) => {
            lock.unlock()?;
            Ok(Writer::None)
        }
        Err(TryLockError::WouldBlock) => Ok(Writer::Live),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Why the writer's lock was not taken.
pub(super) enum LockError {
    /// Another holds it: an engine, or a reader's test.
    Held,
    /// The OS refused the folder or the file.
    Io(String),
}

/// Creates the Run's folder, synced into the folder above it so that the
/// run file it holds outlasts a power cut (ADR 0002), and takes its
/// writer's lock (ADR 0031).
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(super) fn take_lock(folder: &Path) -> Result<File, LockError> {
    durable::create_folder(folder).map_err(|error| LockError::Io(error.to_string()))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(LOCK_FILE))
        .map_err(|error| LockError::Io(error.to_string()))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(LockError::Held),
        Err(TryLockError::Error(error)) => Err(LockError::Io(error.to_string())),
    }
}

/// Takes the writer's lock in `folder` as [`take_lock`] does, under
/// `owner`'s blocking policy: `None` when another holds it.
///
/// # Errors
///
/// When the OS refuses the folder or the file.
pub(super) async fn try_lock(owner: &TaskOwner, folder: &Path) -> Result<Option<File>, String> {
    let folder = folder.to_owned();
    match owner.blocking(move || take_lock(&folder)).await {
        Ok(lock) => Ok(Some(lock)),
        Err(LockError::Held) => Ok(None),
        Err(LockError::Io(error)) => Err(error),
    }
}

/// Runs `attempt` again up to [`RETRIES`] times, each after its
/// [`backoff`] on `clock` with jitter from `entropy`, until it answers
/// something: how a writer retries what it found held, perhaps only for a
/// reader's moment (ADR 0031). `None` when every retry answered none.
///
/// # Errors
///
/// The first error of `attempt`.
pub(super) async fn retried<T, E, F>(
    clock: &Clock,
    entropy: &mut Entropy,
    mut attempt: impl FnMut() -> F,
) -> Result<Option<T>, E>
where
    F: Future<Output = Result<Option<T>, E>>,
{
    for retry in 1..=RETRIES {
        clock.sleep(backoff(retry, entropy.next_u64())).await;
        if let Some(found) = attempt().await? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// How long a writer waits before its `attempt`-th retry of a held lock,
/// from 1 to [`RETRIES`]: the wait doubles with each attempt, 1 s over all
/// of them before jitter, and `jitter` (any number, from the engine's
/// `Entropy`) scales it between three quarters and five quarters of that,
/// so two writers that met once seldom meet again.
pub(super) fn backoff(attempt: u32, jitter: u64) -> Duration {
    let index = usize::try_from(attempt.clamp(1, RETRIES) - 1)
        .expect("invariant: an attempt below RETRIES fits a usize");
    let scale = 1_500 + jitter % 1_001;
    Duration::from_micros(WAITS[index] * scale / 2_000)
}

#[cfg(test)]
mod tests;
