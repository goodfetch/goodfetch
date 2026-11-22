//! The crate's locks and atomics (ADR 0060), the one place
//! `disallowed-types` lets them be named: the Crawl requests a connection's
//! Runs share, held only to move a few values in or out, never across an
//! `.await`, and the count that gives each crawl its number.
#![expect(
    clippy::disallowed_types,
    reason = "the crate's sync module, the one place its locks and atomics are named (ADR 0060)"
)]

/// A lock.
pub(crate) type Mutex<T> = std::sync::Mutex<T>;

/// A held [`Mutex`].
pub(crate) type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;

/// A count shared between threads.
pub(crate) type AtomicU64 = std::sync::atomic::AtomicU64;
