//! How long to wait before the next attempt.

pub(super) fn delay() -> u64 {
    super::policy::attempts()
}
