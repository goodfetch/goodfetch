//! The crate's locks (ADR 0060), the one place `disallowed-types` lets
//! them be named: the lock over a Sink's delivery driver, held only on the
//! blocking pool while one delivery runs, never across an `.await`.
#![expect(
    clippy::disallowed_types,
    reason = "the crate's sync module, the one place its locks are named (ADR 0060)"
)]

/// A lock.
pub(crate) type Mutex<T> = std::sync::Mutex<T>;
