//! Retries.

use crate::clock;

pub(crate) fn retry() -> u64 {
    clock::now()
}

#[cfg(test)]
mod tests {
    // A test may build its fixtures from any module (ADR 0126).
    use crate::dispatch::dispatch;

    #[test]
    fn retries() {
        assert_eq!(dispatch(), super::retry());
    }
}
