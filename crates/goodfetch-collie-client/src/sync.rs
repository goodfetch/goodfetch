//! The crate's locks (ADR 0060), the one place `disallowed-types` lets
//! them be named: a Connection's list of its Runs, its streams of the
//! engine's requests, a Run's state and the names `markers` has warned of,
//! each held only to read or write a few words, never across an `.await`.
#![expect(
    clippy::disallowed_types,
    reason = "the crate's sync module, the one place its locks are named (ADR 0060)"
)]

/// A lock.
pub(crate) type Mutex<T> = std::sync::Mutex<T>;

/// A held [`Mutex`].
pub(crate) type MutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
