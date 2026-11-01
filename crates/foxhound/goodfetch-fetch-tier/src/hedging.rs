//! The rules of the HTTP tier's redundant requests (ADRs 0040, 0045, 0086
//! and 0106; RT-20), each a pure function of what it is given: which
//! requests may be sent twice or answered by an identical one in flight
//! ([`repeatable`]); when a hedge goes, once its route's 95th-percentile
//! latency passed ([`Latencies`]); and how many hedges a Run may send, a
//! share of its requests ([`Budget`]).

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

/// How many latencies of a route its threshold is read from: the latest.
const WINDOW: usize = 128;

/// How many latencies a route needs before it has a threshold: the fewest
/// whose 95th percentile by nearest rank is not their largest.
const FEWEST: usize = 20;

/// Whether a request may be sent twice, as a hedge, or answered by an
/// identical request in flight: only a GET or a HEAD (ADR 0040), from a
/// work item that declares no Effect (ADR 0106), and never a Paid fetch
/// (ADR 0045).
pub(crate) fn repeatable(method: &http::Method, declares_effect: bool, paid: bool) -> bool {
    (method == http::Method::GET || method == http::Method::HEAD) && !declares_effect && !paid
}

/// The latencies of a Run's routes, each a Label on one domain (ADR
/// 0093), the latest [`WINDOW`] of each.
#[derive(Debug, Default)]
pub(crate) struct Latencies {
    routes: BTreeMap<String, VecDeque<Duration>>,
}

impl Latencies {
    /// A request of `route` answered after `latency`.
    pub(crate) fn record(&mut self, route: &str, latency: Duration) {
        let latest = self.routes.entry(route.to_owned()).or_default();
        if latest.len() == WINDOW {
            latest.pop_front();
        }
        latest.push_back(latency);
    }

    /// How long a request of `route` goes unanswered before it is hedged:
    /// the 95th percentile, by nearest rank, of its latest latencies;
    /// `None` until it has [`FEWEST`].
    pub(crate) fn threshold(&self, route: &str) -> Option<Duration> {
        let latest = self
            .routes
            .get(route)
            .filter(|latest| latest.len() >= FEWEST)?;
        let mut sorted: Vec<Duration> = latest.iter().copied().collect();
        sorted.sort_unstable();
        // The nearest rank: the smallest at or above 95 % of them.
        let rank = (sorted.len() * 95).div_ceil(100);
        sorted.get(rank - 1).copied()
    }
}

/// How many hedges a Run may still send: at most `max_percent` of the
/// requests it sent, whole percent (`fetch.hedge.max_percent`).
#[derive(Debug)]
pub(crate) struct Budget {
    max_percent: u64,
    requests: u64,
    hedges: u64,
}

impl Budget {
    /// A Run's budget under `fetch.hedge.max_percent`, before any request.
    pub(crate) fn new(max_percent: u64) -> Self {
        Self {
            max_percent,
            requests: 0,
            hedges: 0,
        }
    }

    /// The Run sent one more request, not a hedge.
    pub(crate) fn sent(&mut self) {
        self.requests = self.requests.saturating_add(1);
    }

    /// Spends one hedge, if the budget has one left: `false` when one more
    /// would pass `max_percent` of the requests sent.
    pub(crate) fn spend(&mut self) -> bool {
        let after = self.hedges.saturating_add(1);
        let fits =
            u128::from(after) * 100 <= u128::from(self.max_percent) * u128::from(self.requests);
        if fits {
            self.hedges = after;
        }
        fits
    }
}

#[cfg(test)]
mod tests;
