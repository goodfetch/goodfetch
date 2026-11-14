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
