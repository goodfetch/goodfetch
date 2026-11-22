//! The crate's locks and atomics (ADR 0060), the one place
//! `disallowed-types` lets them be named: a Connection's list of its Runs,
//! its streams of the engine's requests, the log level it told the engine
//! and its Subscription with its readers, a Run's state, the names
//! `markers` has warned of and the callsites `logging` keeps, the call an
//! event stream shares with the values it opens and the wakers of its
//! readers, each held only to read or write a few words, or to queue a
//! message, never across an `.await`, and whether a value a Run holds is
//! closed. Locks are taken inside another only in the Subscription, in one
//! order: the Connection's, then its readers', then a reader's queue; and
//! in an opened call, in one order: the call's, then its readers' wakers.
//! Under `--cfg goodfetch_loom` the locks and atomics map to loom's, so the
//! opened call's loom tests explore every interleaving of its readers; the
//! locks in a `static` stay std's, since loom's cannot be built in one.
#![expect(
    clippy::disallowed_types,
    reason = "the crate's sync module, the one place its locks and atomics are named (ADR 0060)"
)]

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

/// A lock in a `static`, std's under `goodfetch_loom` too.
pub(crate) type StaticMutex<T> = std::sync::Mutex<T>;

/// A flag shared between threads, loom's under `goodfetch_loom`.
#[cfg(all(test, goodfetch_loom))]
pub(crate) type AtomicBool = loom::sync::atomic::AtomicBool;
/// A flag shared between threads, loom's under `goodfetch_loom`.
#[cfg(not(all(test, goodfetch_loom)))]
pub(crate) type AtomicBool = std::sync::atomic::AtomicBool;
