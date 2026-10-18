//! Time, naming a module it may not inside a macro invocation.

pub(crate) fn now() -> u64 {
    tracing::info!(attempt = crate::retry::retry(), "reading the clock");
    0
}
