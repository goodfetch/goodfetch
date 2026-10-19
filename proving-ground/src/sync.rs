//! The Proving ground's lock, the one module that names std's (ADR 0060).
#![expect(
    clippy::disallowed_types,
    reason = "the sync module is the one place that names std's locks (ADR 0060)"
)]

use std::sync::MutexGuard;

/// A mutual-exclusion lock whose holder never panics while holding it.
#[derive(Default)]
pub(crate) struct Mutex<T>(std::sync::Mutex<T>);

impl<T> Mutex<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }

    /// Takes the lock.
    pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
        self.0
            .lock()
            .expect("invariant: no holder of a Proving ground lock panics")
    }
}
