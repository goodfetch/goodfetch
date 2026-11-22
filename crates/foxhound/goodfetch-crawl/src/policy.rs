//! A crawl's policy for its attempts (ADRs 0003, 0080 and 0093): how long a
//! handler has, the lease that bounds it, how many work items the program
//! handles at once, and which failures are retried and how often, each read
//! from the Run's resolved settings (`timeout.handler`, `crawl.retry.max`,
//! `crawl.retry.handler_failed`, `crawl.retry.engine_panicked`,
//! `crawl.handler.concurrency`), resolved once
//! as the Run opens (ADR 0138), a Label's own `crawl.retry.handler_failed`
//! replacing the Run's for its requests (ADR 0104); and the Run's domain policy, which its
//! Frontier judges each seed and enqueue by.

use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::crawl::{
    CRAWL_HANDLER_CONCURRENCY, CRAWL_RETRY_ENGINE_PANICKED, CRAWL_RETRY_HANDLER_FAILED,
    CRAWL_RETRY_MAX, TIMEOUT_HANDLER,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};

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
    /// `crawl.retry.engine_panicked`: whether an attempt the engine's panic
    /// ended is retried (ADR 0148).
    pub(crate) retry_engine_panicked: bool,
    /// `crawl.handler.concurrency`: how many work items the program handles
    /// at once.
    pub(crate) handler_concurrency: usize,
}

impl CrawlPolicy {
    /// The policy `resolved` gives a Foxhound Run.
    pub(crate) fn of(resolved: &Resolved) -> Self {
        Self {
            handler_timeout: resolved.get(&TIMEOUT_HANDLER),
            retry_max: u32::try_from(resolved.get::<u64>(&CRAWL_RETRY_MAX)).unwrap_or(u32::MAX),
            retry_handler_failed: resolved.get(&CRAWL_RETRY_HANDLER_FAILED),
            retry_engine_panicked: resolved.get(&CRAWL_RETRY_ENGINE_PANICKED),
            handler_concurrency: usize::try_from(resolved.get::<u64>(&CRAWL_HANDLER_CONCURRENCY))
                .unwrap_or(usize::MAX)
                .max(1),
        }
    }

    /// A work item's lease: the handler's timeout plus a margin of the
    /// larger of 10 s and a tenth of it (spec #213).
    pub(crate) fn lease_length(&self) -> Duration {
        lease_for(self.handler_timeout)
    }

    /// The policy of a request whose Label sets `crawl.retry.handler_failed`
    /// to `retries_handler_failed`, or leaves it to the Run, `None` (ADR
    /// 0104).
    pub(crate) fn under_label(self, retries_handler_failed: Option<bool>) -> Self {
        Self {
            retry_handler_failed: retries_handler_failed.unwrap_or(self.retry_handler_failed),
            ..self
        }
    }

    /// Whether a work item tried `attempts` times may be tried again.
    pub(crate) fn may_retry(&self, attempts: u32) -> bool {
        attempts <= self.retry_max
    }
}

/// The Run's domain policy, `egress.allowed_domains` and
/// `egress.blocked_domains` as `resolved` holds them, which the Frontier
/// judges every seed and enqueue by (ADRs 0017 and 0093).
///
/// # Errors
///
/// `setting_invalid` for an entry that is no host.
pub(crate) fn domain_policy(resolved: &Resolved) -> Result<Policy, CallError> {
    Policy::of(resolved).map_err(|invalid| {
        CallError::new(
            &SETTING_INVALID,
            format!(
                "`{}` holds `{}`, which is no host",
                invalid.setting, invalid.entry
            ),
            &SettingInvalidDetails {
                key: invalid.setting.to_owned(),
                layer: None,
                reason: format!("`{}` is no host", invalid.entry),
                limit: None,
                fix: None,
            },
        )
    })
}

/// The domain policy of the crawl `run_id`, resumed under `resolved`; one
/// that cannot be read, which its start would have refused, admits no seed
/// or enqueue, and is logged.
pub(crate) fn resumed_domain_policy(resolved: &Resolved, run_id: &RunId) -> Policy {
    domain_policy(resolved).unwrap_or_else(|error| {
        tracing::error!(error = error.message(), %run_id, "the resumed crawl's domain policy could not be read: it admits no seed or enqueue");
        Policy::new(&[] as &[&str], &[] as &[&str])
            .expect("invariant: an empty domain policy reads")
    })
}

/// A lease that holds through a wait of `wait`: the wait plus a margin of
/// the larger of 10 s and a tenth of it (spec #213).
pub(crate) fn lease_for(wait: Duration) -> Duration {
    wait + (wait / 10).max(Duration::from_secs(10))
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
        retry_engine_panicked: true,
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
    #[case::the_labels_own(Some(true), true)]
    #[case::the_runs(None, false)]
    fn a_labels_handler_retry_replaces_the_runs(
        #[case] label: Option<bool>,
        #[case] retried: bool,
    ) {
        assert_eq!(POLICY.under_label(label).retry_handler_failed, retried);
    }

    #[rstest]
    #[case::the_first_attempt(1, true)]
    #[case::the_last_retry(2, true)]
    #[case::after_two_retries(3, false)]
    fn two_retries_give_three_attempts(#[case] attempts: u32, #[case] retried: bool) {
        assert_eq!(POLICY.may_retry(attempts), retried);
    }
}
