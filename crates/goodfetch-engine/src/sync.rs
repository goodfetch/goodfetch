//! The locks of the crate's CPU pool (ADR 0060), the one place
//! `disallowed-types` lets them be named: under `--cfg goodfetch_loom` they
//! map to loom's, so the pool's loom tests explore every interleaving of
//! its threads (#2200).
#![cfg_attr(
    not(all(test, goodfetch_loom)),
    expect(
        clippy::disallowed_types,
        reason = "the crate's sync module, the one place its locks are named (ADR 0060)"
    )
)]

#[cfg(all(test, goodfetch_loom))]
pub(crate) use loom::sync::Arc;
#[cfg(not(all(test, goodfetch_loom)))]
pub(crate) use std::sync::Arc;

/// An atomic `u64`, loom's under `goodfetch_loom`.
#[cfg(all(test, goodfetch_loom))]
pub(crate) type AtomicU64 = loom::sync::atomic::AtomicU64;
/// An atomic `u64`, loom's under `goodfetch_loom`.
#[cfg(not(all(test, goodfetch_loom)))]
pub(crate) type AtomicU64 = std::sync::atomic::AtomicU64;

#[cfg(all(test, goodfetch_loom))]
pub(crate) use loom::sync::atomic::Ordering;
#[cfg(not(all(test, goodfetch_loom)))]
pub(crate) use std::sync::atomic::Ordering;

/// A lock, loom's under `goodfetch_loom`.
#[cfg(all(test, goodfetch_loom))]
pub(crate) type Mutex<T> = loom::sync::Mutex<T>;
/// A lock, loom's under `goodfetch_loom`.
#[cfg(not(all(test, goodfetch_loom)))]
pub(crate) type Mutex<T> = std::sync::Mutex<T>;

/// A held [`Mutex`].
#[cfg(all(test, goodfetch_loom))]
pub(crate) type MutexGuard<'a, T> = loom::sync::MutexGuard<'a, T>;
/// A held [`Mutex`].
#[cfg(not(all(test, goodfetch_loom)))]
pub(crate) type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;

/// A condition variable, loom's under `goodfetch_loom`.
#[cfg(all(test, goodfetch_loom))]
pub(crate) type Condvar = loom::sync::Condvar;
/// A condition variable, loom's under `goodfetch_loom`.
#[cfg(not(all(test, goodfetch_loom)))]
pub(crate) type Condvar = std::sync::Condvar;

/// In-memory count of declared Effect bodies currently running for one Run,
/// read with a single atomic load and no I/O on the Response cache hot path
/// (ADRs 0046, 0106 and 0140).
#[derive(Clone, Debug, Default)]
pub struct ActiveEffects(std::sync::Arc<AtomicU64>);

impl ActiveEffects {
    /// Whether at least one declared Effect body is currently running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.0.load(Ordering::Acquire) > 0
    }

    /// Updates the running body count.
    pub(crate) fn set(&self, count: usize) {
        self.0
            .store(u64::try_from(count).unwrap_or(u64::MAX), Ordering::Release);
    }

    /// Downgrades to a weak handle so `Effects` drops entries once no Run or
    /// Shelf holds them.
    pub(crate) fn downgrade(&self) -> std::sync::Weak<AtomicU64> {
        std::sync::Arc::downgrade(&self.0)
    }

    /// Upgrades a weak handle back into an [`ActiveEffects`] handle.
    pub(crate) fn upgrade(weak: &std::sync::Weak<AtomicU64>) -> Option<Self> {
        weak.upgrade().map(Self)
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}
