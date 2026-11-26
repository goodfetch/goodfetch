//! The crate's locks and atomics (ADR 0060), the one place
//! `disallowed-types` lets them be named: the lock over a Sink's delivery
//! driver, held only on the blocking pool while one delivery runs, never
//! across an `.await`; and the flag that says a Sink's delivery stopped.
#![expect(
    clippy::disallowed_types,
    reason = "the crate's sync module, the one place its locks and atomics are named (ADR 0060)"
)]

/// A lock.
pub(crate) type Mutex<T> = std::sync::Mutex<T>;

/// A flag shared between threads.
pub(crate) type AtomicBool = std::sync::atomic::AtomicBool;
