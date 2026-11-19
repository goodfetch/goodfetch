//! The crate's locks and atomics (ADR 0060), the one place
//! `disallowed-types` lets them be named: a Connection's list of its Runs,
//! its streams of the engine's requests and a Run's state, each held only
//! to read or write a few words, never across an `.await`, and whether a
//! value a Run holds is closed.
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
