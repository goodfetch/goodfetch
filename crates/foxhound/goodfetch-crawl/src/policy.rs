//! A crawl's policy for its attempts (ADRs 0003, 0080 and 0093): how long a
//! handler has, the lease that bounds it, how many work items the program
//! handles at once, and which failures are retried and how often. Its
//! values are the settings' defaults until the settings resolver (#225)
//! gives each Run its own.

use std::time::Duration;

/// How a crawl's attempts are bounded and retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrawlPolicy {
    /// `timeout.handler`: how long a handler has for one attempt.
    pub handler_timeout: Duration,
    /// `crawl.retry.max`: how many times a work item is tried again.
    pub retry_max: u32,
    /// `crawl.retry.handler_failed`: whether a handler's own failure is
    /// retried.
    pub retry_handler_failed: bool,
    /// `crawl.handler.concurrency`: how many work items the program handles
    /// at once.
    pub handler_concurrency: usize,
}

impl Default for CrawlPolicy {
    /// The settings' defaults: 60 s per attempt (Crawlee's
    /// `requestHandlerTimeoutSecs`), two retries (Scrapy's `RETRY_TIMES`),
    /// a handler's own failure not retried, 16 at once (Scrapy's
    /// `CONCURRENT_REQUESTS`).
    fn default() -> Self {
        Self {
            handler_timeout: Duration::from_mins(1),
            retry_max: 2,
            retry_handler_failed: false,
            handler_concurrency: 16,
        }
    }
}

impl CrawlPolicy {
    /// A work item's lease: the handler's timeout plus a margin of the
    /// larger of 10 s and a tenth of it (spec #213).
    pub fn lease_length(&self) -> Duration {
        self.handler_timeout + (self.handler_timeout / 10).max(Duration::from_secs(10))
    }

    /// Whether a work item tried `attempts` times may be tried again.
    pub fn may_retry(&self, attempts: u32) -> bool {
        attempts <= self.retry_max
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rstest::rstest;

    use super::CrawlPolicy;

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
            ..CrawlPolicy::default()
        };

        assert_eq!(policy.lease_length(), lease);
    }

    #[rstest]
    #[case::the_first_attempt(1, true)]
    #[case::the_last_retry(2, true)]
    #[case::after_two_retries(3, false)]
    fn two_retries_give_three_attempts(#[case] attempts: u32, #[case] retried: bool) {
        assert_eq!(CrawlPolicy::default().may_retry(attempts), retried);
    }
}
