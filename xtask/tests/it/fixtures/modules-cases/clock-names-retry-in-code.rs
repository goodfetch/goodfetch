//! Time, naming a module it may not in a qualified path.

pub(crate) fn now() -> u64 {
    crate::retry::retry()
}
