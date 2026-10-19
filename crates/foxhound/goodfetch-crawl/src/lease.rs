//! A work item's lease (ADR 0003): the right of one attempt to commit its
//! outputs, numbered by an epoch that only goes up, granted for a length
//! and renewed only by lengthening. Whether it still holds is a pure
//! function of the `Clock`'s monotonic reading (ADR 0059).

use std::time::Duration;

use tokio::time::Instant;

/// One attempt's lease of a work item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    epoch: u64,
    expires_at: Instant,
}

impl Lease {
    /// The lease of epoch `epoch`, granted at `now` for `length`.
    pub fn granted(epoch: u64, now: Instant, length: Duration) -> Self {
        Self {
            epoch,
            expires_at: now + length,
        }
    }

    /// Its epoch.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// When it ends.
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }

    /// Whether it still holds at `now`.
    pub fn holds_at(&self, now: Instant) -> bool {
        now < self.expires_at
    }

    /// Renews it at `now` for `length`; a renewal never shortens it.
    #[must_use]
    pub fn renewed(self, now: Instant, length: Duration) -> Self {
        Self {
            expires_at: self.expires_at.max(now + length),
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;
    use tokio::time::Instant;

    use super::Lease;

    /// Some monotonic moment, from the test `Clock`; the rules take every
    /// reading as an argument.
    #[expect(
        clippy::disallowed_methods,
        reason = "tests read the test Clock (ADR 0059)"
    )]
    fn origin() -> Instant {
        goodfetch_engine::clock::Clock::paused(std::time::UNIX_EPOCH).now()
    }

    proptest! {
        #![proptest_config(goodfetch_test_support::proptest_config())]

        #[test]
        fn a_lease_holds_until_its_length_has_passed(length in 1_u64..1_000_000, after in 0_u64..2_000_000) {
            let start = origin();
            let lease = Lease::granted(1, start, Duration::from_millis(length));

            prop_assert_eq!(lease.holds_at(start + Duration::from_millis(after)), after < length);
        }

        #[test]
        fn a_renewal_never_shortens_a_lease(length in 1_u64..1_000_000, at in 0_u64..1_000_000, renewal in 0_u64..1_000_000) {
            let start = origin();
            let lease = Lease::granted(1, start, Duration::from_millis(length));

            let renewed = lease.renewed(start + Duration::from_millis(at), Duration::from_millis(renewal));

            prop_assert!(renewed.expires_at() >= lease.expires_at());
            prop_assert_eq!(renewed.epoch(), lease.epoch());
        }
    }
}
