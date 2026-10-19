//! How many attempts a call gets, reaching a sibling through `super`.

pub(super) fn attempts() -> u64 {
    super::backoff::delay() + 3
}
