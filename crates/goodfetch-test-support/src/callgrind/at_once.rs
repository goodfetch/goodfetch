//! How many Callgrind children one process runs at once: half the
//! machine's cores, at least one and at most [`MOST_AT_ONCE`], or fewer
//! when a cap says so; and the bound that holds the others back.

use std::num::NonZeroUsize;

#[expect(
    clippy::disallowed_types,
    reason = "a benchmark harness's bound on its own child processes, waited for on threads no runtime owns, which loom does not model"
)]
use std::sync::{Condvar, Mutex, PoisonError};

/// The most Callgrind children one process runs at once, whatever its cores.
const MOST_AT_ONCE: usize = 8;

/// How many Callgrind children run at once on a machine of `cores` cores:
/// half of them, at least one and at most [`MOST_AT_ONCE`], and never more
/// than `cap`, which only lowers it.
pub(super) fn most_at_once(cores: usize, cap: Option<NonZeroUsize>) -> usize {
    let most = (cores / 2).clamp(1, MOST_AT_ONCE);
    cap.map_or(most, |cap| most.min(cap.get()))
}

/// The Callgrind children running within one bound, and their release.
#[expect(
    clippy::disallowed_types,
    reason = "a benchmark harness's bound on its own child processes, waited for on threads no runtime owns, which loom does not model"
)]
pub(super) struct Bound {
    running: Mutex<usize>,
    released: Condvar,
}

impl Bound {
    #[expect(
        clippy::disallowed_types,
        reason = "a benchmark harness's bound on its own child processes, waited for on threads no runtime owns, which loom does not model"
    )]
    pub(super) const fn new() -> Self {
        Self {
            running: Mutex::new(0),
            released: Condvar::new(),
        }
    }

    /// Runs `run`, which runs one Callgrind child, once fewer than `most`
    /// run within this bound.
    pub(super) fn run<T>(&self, most: usize, run: impl FnOnce() -> T) -> T {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        while *running >= most {
            running = self
                .released
                .wait(running)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *running += 1;
        drop(running);
        let _release = Release(self);
        run()
    }
}

/// Gives back its child's place in its bound once the child ended, whether
/// `run` returned or panicked.
struct Release<'bound>(&'bound Bound);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        *self
            .0
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner) -= 1;
        self.0.released.notify_one();
    }
}

#[cfg(test)]
mod tests;
