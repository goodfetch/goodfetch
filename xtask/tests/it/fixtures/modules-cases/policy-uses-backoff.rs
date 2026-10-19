//! How many attempts a call gets, reaching a sibling it may not import.

pub(super) fn attempts() -> u64 {
    crate::retry::backoff::delay() + 3
}
