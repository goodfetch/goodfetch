//! The crawl's tier race (spec #582's `goodfetch-crawl`; ADRs 0040 and
//! 0093; CO-16): under `fetch.tier_race`, an attempt whose tier is not
//! forced starts browser-fetch and, a delay later, a full navigation in the
//! same Session; the first result that passes the Page check and agrees
//! with its Label's comparison wins, and the other is cancelled.
//!
//! The delay is the median browser-fetch latency of the request's Label on
//! its domain key ([`Latencies`]), [`FIRST_DELAY`] before any sample. A
//! browser-fetch result agrees with its Label's comparison unless the
//! sample comparison learned that Label, or the request's path segment of
//! it, onto `browser`, where a document never answers as the browser does
//! ([`Raced::agrees`]).
//!
//! The race's runs are the delivery's (`attempt`).

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

/// The delay before a race's navigation while its Label on its domain has
/// no browser-fetch latency yet.
pub(crate) const FIRST_DELAY: Duration = Duration::from_millis(500);

/// How many of its latest browser-fetch latencies a Label on a domain
/// keeps.
pub(crate) const KEPT: usize = 128;

/// How one attempt races.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Raced {
    pub(crate) label: String,
    pub(crate) domain_key: String,
    /// How long after browser-fetch starts the navigation starts.
    pub(crate) delay: Duration,
    /// Whether a browser-fetch result agrees with its Label's comparison.
    pub(crate) agrees: bool,
}

/// The browser-fetch latencies of every Label on every domain of a Run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Latencies {
    by_unit: BTreeMap<(String, String), VecDeque<Duration>>,
}

impl Latencies {
    /// The delay before the navigation of a race of `label` on
    /// `domain_key`: the median of its browser-fetch latencies, else
    /// [`FIRST_DELAY`].
    pub(crate) fn delay(&self, label: &str, domain_key: &str) -> Duration {
        self.by_unit
            .get(&(label.to_owned(), domain_key.to_owned()))
            .and_then(|kept| median(kept.iter().copied()))
            .unwrap_or(FIRST_DELAY)
    }

    /// Takes the latency of a browser-fetch of `label` on `domain_key`,
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
