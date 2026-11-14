//! The crawl's tier race (spec #582's `goodfetch-crawl`; ADRs 0040 and
//! 0093; CO-16): under `fetch.tier_race`, an attempt whose tier is not
//! forced first requests its document on the document tier, from where its
//! Label learned to leave from (#2421): a Session's own transport for a
//! Label learned onto it, else browser-fetch from the hop page of the
//! Session the race navigates in. A delay later, a full navigation starts
//! in a Session whose client is its browser; the first result that passes
//! the Page check and agrees with its Label's comparison wins, and the
//! other is cancelled.
//!
//! The delay is the median latency of the first requests of the request's
//! Label on its domain key ([`Latencies`]), [`FIRST_DELAY`] before any is
//! known. A first request's result agrees with its Label's comparison
//! unless the sample comparison learned that Label, or the request's path
//! segment of it, onto `browser`, where a document never answers as the
//! browser does ([`Raced::agrees`]).
//!
//! The race's runs are the delivery's (`attempt`).

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

/// The delay before a race's navigation while its Label on its domain has
/// no first request's latency yet.
pub(crate) const FIRST_DELAY: Duration = Duration::from_millis(500);

/// How many of the latest latencies of its races' first requests a Label
/// on a domain keeps.
pub(crate) const KEPT: usize = 128;

/// How one attempt races.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Raced {
    pub(crate) label: String,
    pub(crate) domain_key: String,
    /// How long after its first request starts the navigation starts.
    pub(crate) delay: Duration,
    /// Whether its first request's result agrees with its Label's
    /// comparison.
    pub(crate) agrees: bool,
}

/// The latencies of the races' first requests of every Label on every
/// domain of a Run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Latencies {
    by_unit: BTreeMap<(String, String), VecDeque<Duration>>,
}

impl Latencies {
    /// The delay before the navigation of a race of `label` on
    /// `domain_key`: the median of its first requests' latencies, else
    /// [`FIRST_DELAY`].
    pub(crate) fn delay(&self, label: &str, domain_key: &str) -> Duration {
        self.by_unit
            .get(&(label.to_owned(), domain_key.to_owned()))
            .and_then(|kept| median(kept.iter().copied()))
            .unwrap_or(FIRST_DELAY)
    }

    /// Takes the latency of a race's first request of `label` on `domain_key`,
    /// keeping its latest [`KEPT`].
    pub(crate) fn record(&mut self, label: &str, domain_key: &str, latency: Duration) {
        let kept = self
            .by_unit
            .entry((label.to_owned(), domain_key.to_owned()))
            .or_default();
        if kept.len() >= KEPT {
            kept.pop_front();
        }
        kept.push_back(latency);
    }
}

/// The median of `samples` by nearest rank, the `⌈n/2⌉`th smallest, so it
/// is always one of them; `None` for none.
pub(crate) fn median(samples: impl IntoIterator<Item = Duration>) -> Option<Duration> {
    let mut sorted: Vec<Duration> = samples.into_iter().collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_unstable();
    Some(sorted[sorted.len().div_ceil(2) - 1])
}

#[cfg(test)]
mod tests;
