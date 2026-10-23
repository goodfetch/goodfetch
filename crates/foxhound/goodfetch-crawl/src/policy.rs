//! A crawl's policy for its attempts (ADRs 0003, 0080 and 0093): how long a
//! handler has, the lease that bounds it, how many work items the program
//! handles at once, and which failures are retried and how often, each read
//! from the Run's resolved settings (`timeout.handler`, `crawl.retry.max`,
//! `crawl.retry.handler_failed`, `crawl.handler.concurrency`) at each
//! admission point, so a tightened Operator policy cap reaches a crawl
//! already going (ADR 0028).

use std::time::Duration;

use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_HANDLER_CONCURRENCY, CRAWL_RETRY_HANDLER_FAILED, CRAWL_RETRY_MAX, TIMEOUT_HANDLER,
};

/// How a crawl's attempts are bounded and retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CrawlPolicy {
    /// `timeout.handler`: how long a handler has for one attempt.
    pub(crate) handler_timeout: Duration,
    /// `crawl.retry.max`: how many times a work item is tried again.
    pub(crate) retry_max: u32,
    /// `crawl.retry.handler_failed`: whether a handler's own failure is
    /// retried.
    pub(crate) retry_handler_failed: bool,
    /// `crawl.handler.concurrency`: how many work items the program handles
    /// at once.
    pub(crate) handler_concurrency: usize,
}

impl CrawlPolicy {
    /// The policy `resolved` gives a Foxhound Run, whose keys always hold
    /// a value of their kind.
    pub(crate) fn of(resolved: &Resolved) -> Self {
        let count = |key: &str| {
            resolved
                .count(key)
                .expect("invariant: a crawl's count settings resolve to a count")
        };
        Self {
            handler_timeout: resolved
                .duration(TIMEOUT_HANDLER.name)
                .expect("invariant: `timeout.handler` resolves to a duration"),
            retry_max: u32::try_from(count(CRAWL_RETRY_MAX.name)).unwrap_or(u32::MAX),
            retry_handler_failed: resolved
                .boolean(CRAWL_RETRY_HANDLER_FAILED.name)
                .expect("invariant: `crawl.retry.handler_failed` resolves to a boolean"),
            handler_concurrency: usize::try_from(count(CRAWL_HANDLER_CONCURRENCY.name))
                .unwrap_or(usize::MAX)
                .max(1),
        }
    }

    /// A work item's lease: the handler's timeout plus a margin of the
    /// larger of 10 s and a tenth of it (spec #213).
    pub(crate) fn lease_length(&self) -> Duration {
        self.handler_timeout + (self.handler_timeout / 10).max(Duration::from_secs(10))
    }

    /// Whether a work item tried `attempts` times may be tried again.
    pub(crate) fn may_retry(&self, attempts: u32) -> bool {
        attempts <= self.retry_max
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rstest::rstest;

    use super::CrawlPolicy;

    const POLICY: CrawlPolicy = CrawlPolicy {
        handler_timeout: Duration::from_mins(1),
        retry_max: 2,
        retry_handler_failed: false,
        handler_concurrency: 16,
    };

    #[rstest]
    #[case::the_default_minute(Duration::from_mins(1), Duration::from_secs(70))]
    #[case::a_long_handler(Duration::from_mins(10), Duration::from_mins(11))]
    #[case::a_short_handler(Duration::from_secs(1), Duration::from_secs(11))]
    fn a_lease_lasts_the_handler_timeout_and_the_larger_of_ten_seconds_and_a_tenth(
        #[case] handler_timeout: Duration,
        #[case] lease: Duration,
    ) {
        let policy = CrawlPolicy {
            handler_timeout,
            ..POLICY
        };

        assert_eq!(policy.lease_length(), lease);
    }

    #[rstest]
    #[case::the_first_attempt(1, true)]
    #[case::the_last_retry(2, true)]
    #[case::after_two_retries(3, false)]
    fn two_retries_give_three_attempts(#[case] attempts: u32, #[case] retried: bool) {
        assert_eq!(POLICY.may_retry(attempts), retried);
    }
}
