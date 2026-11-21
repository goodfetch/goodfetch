//! The crate's locks and atomics (ADR 0060), the one place
//! `disallowed-types` lets them be named: a Connection's list of its Runs,
//! its streams of the engine's requests, the log level it told the engine
//! and its Subscription with its readers, a Run's state, the names
//! `markers` has warned of and the callsites `logging` keeps, each held
//! only to read or write a few words, or to queue a message, never across
//! an `.await`, and whether a value a Run holds is closed. Locks are taken
//! inside another only in the Subscription, in one order: the
//! Connection's, then its readers', then a reader's queue.
#![expect(
    clippy::disallowed_types,
    reason = "the crate's sync module, the one place its locks and atomics are named (ADR 0060)"
)]

/// A lock.
pub(crate) type Mutex<T> = std::sync::Mutex<T>;

/// A held [`Mutex`].
pub(crate) type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;

/// A flag shared between threads.
pub(crate) type AtomicBool = std::sync::atomic::AtomicBool;
