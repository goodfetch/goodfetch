//! Retries.

mod backoff;
mod policy;

use crate::Tick;
use crate::clock;

pub(crate) fn retry() -> u64 {
    let _beat = Tick;
    clock::now() + backoff::delay()
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
