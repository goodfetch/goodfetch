//! What the Proving ground's observations (`GET /observations`) say one
//! trial did, never a tool's report (spec #152): the same-work check's work,
//! the network figures, and the times of its first request, of each page's
//! answer and of each page's discovery. A trial marks its tool's start with
//! one request of its own, [`MARKER`], sent just before it starts the tool,
//! so the Proving ground's clock gives each time from that start; the
//! marker is no request of the tool's.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::trials::{LatencyEnd, Percentiles, Work};

/// The target of a trial's start marker.
pub(crate) const MARKER: &str = "/compare/start";

/// What one trial did, as observed.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Observed {
    pub(crate) work: Work,
    pub(crate) requests_sent: u64,
    pub(crate) duplicates: u64,
    pub(crate) errors: u64,
    /// The bytes the tool sent to the site's host and received from it.
    pub(crate) sent_bytes: u64,
    pub(crate) received_bytes: u64,
    pub(crate) connections: u64,
    pub(crate) tls_connections: u64,
    pub(crate) startup_seconds: f64,
    pub(crate) first_page_seconds: f64,
    pub(crate) first_record_seconds: Option<f64>,
    pub(crate) curve: Vec<f64>,
    pub(crate) latency: Percentiles,
    pub(crate) latency_end: Option<LatencyEnd>,
    /// When the site's `k`-th distinct page was answered, for each `k`
    /// from 1, in seconds from the start.
    pub(crate) page_times: Vec<f64>,
}

/// The site a trial crawled.
pub(crate) struct Site<'a> {
    pub(crate) host: &'a str,
    pub(crate) pages: u64,
}

impl Site<'_> {
    fn prefix(&self) -> String {
        format!("/sites/{}/", self.pages)
    }

    /// The page number of `target`, when it is a page of the site.
    fn page(&self, target: &str) -> Option<u64> {
        let page: u64 = target.strip_prefix(&self.prefix())?.parse().ok()?;
        (1..=self.pages).contains(&page).then_some(page)
    }
}

/// The page that links page `page` first: its parent, `(page - 2) / 10 +
/// 1`, as the Proving ground's sites link each page `n` to `10(n-1)+2` to
/// `10n+1`; none for the first page.
fn parent(page: u64) -> Option<u64> {
    (page > 1).then(|| (page - 2) / 10 + 1)
}

/// Whether a request's `host` names `host`, with or without its port.
fn on_host(request: &Value, host: &str) -> bool {
    request["host"].as_str().is_some_and(|seen| {
        seen == host || seen.split_once(':').is_some_and(|(name, _)| name == host)
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "microseconds and counts here stay far below 2^52, where a float is exact"
)]
fn float(count: u64) -> f64 {
    count as f64
}

/// The nearest-rank percentile `share` of sorted `values`.
pub(crate) fn percentile(sorted: &[f64], share: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (share * float(sorted.len() as u64)).ceil();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a rank between 1 and the count, which fits a usize"
    )]
    let index = (rank as usize).clamp(1, sorted.len()) - 1;
    sorted[index]
}

/// What `observations` record of the trial on `site`, timed from its
/// marker. `records` holds the time of each page's Record commit in
/// seconds from the start, by path, for a tool that keeps Records; and
/// `seconds` is the trial's length, which the curve divides in tenths.
#[expect(
    clippy::too_many_lines,
    reason = "one pass over the observations gives every figure of the trial"
)]
pub(crate) fn observe(
    observations: &Value,
    site: &Site<'_>,
    seconds: f64,
    records: Option<&HashMap<String, f64>>,
) -> Result<Observed> {
    let requests = observations["requests"]
        .as_array()
        .context("the observations hold no requests")?;
    let unrecorded = observations["unrecorded_requests"].as_u64().unwrap_or(0);
    anyhow::ensure!(
        unrecorded == 0,
        "the Proving ground left {unrecorded} requests unrecorded"
    );
    let on_site: Vec<&Value> = requests
        .iter()
        .filter(|request| on_host(request, site.host))
        .collect();
    let marker = *on_site
        .iter()
        .find(|request| request["target"] == MARKER)
        .context("the observations hold no start marker")?;
    let origin = marker["timing"]["arrived_microseconds"]
        .as_u64()
        .context("the marker has no arrival time")?;
    let seconds_of = |microseconds: u64| float(microseconds.saturating_sub(origin)) / 1e6;
    let answered = |request: &Value| {
        request["timing"]["answered_microseconds"]
            .as_u64()
            .or_else(|| request["timing"]["arrived_microseconds"].as_u64())
            .map(seconds_of)
    };
    let mut observed = Observed {
        sent_bytes: observations["hosts"][site.host]["received"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(marker["bytes"]["received"].as_u64().unwrap_or(0)),
        received_bytes: observations["hosts"][site.host]["sent"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(marker["bytes"]["sent"].as_u64().unwrap_or(0)),
        startup_seconds: f64::NAN,
        first_page_seconds: f64::NAN,
        ..Observed::default()
    };
    let mut connections = BTreeSet::new();
    let mut tls = BTreeSet::new();
    // Each page's first answer, in seconds, and the bytes of that answer.
    let mut first: BTreeMap<u64, f64> = BTreeMap::new();
    let mut extra = BTreeSet::new();
    let mut first_request = f64::INFINITY;
    for request in &on_site {
        if request["target"] == MARKER && request["connection"] == marker["connection"] {
            continue;
        }
        observed.requests_sent += 1;
        connections.insert(request["connection"].as_u64());
        if !request["tls"].is_null() {
            tls.insert(request["connection"].as_u64());
        }
        if let Some(arrived) = request["timing"]["arrived_microseconds"].as_u64() {
            first_request = first_request.min(seconds_of(arrived));
        }
        let target = request["target"].as_str().unwrap_or_default();
        if request["status"] != 200 {
            observed.errors += 1;
            continue;
        }
        if request["method"] != "GET" || !target.starts_with(&site.prefix()) {
            continue;
        }
        let bytes = request["bytes"]["sent"]
            .as_u64()
            .with_context(|| format!("the request for {target} counts no bytes sent"))?;
        observed.work.requests += 1;
        observed.work.bytes += bytes;
        let Some(page) = site.page(target) else {
            extra.insert(target.to_owned());
            continue;
        };
        let at = answered(request).unwrap_or(f64::NAN);
        match first.entry(page) {
            std::collections::btree_map::Entry::Occupied(_) => observed.duplicates += 1,
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(at);
                observed.work.first_bytes += bytes;
            }
        }
    }
    observed.connections = connections.len() as u64;
    observed.tls_connections = tls.len() as u64;
    observed.startup_seconds = first_request;
    observed.work.pages = first.len() as u64;
    observed.work.missing = (1..=site.pages)
        .filter(|page| !first.contains_key(page))
        .map(|page| format!("{}{page}", site.prefix()))
        .collect();
    observed.work.extra = extra.into_iter().collect();
    let mut times: Vec<f64> = first.values().copied().collect();
    times.sort_by(f64::total_cmp);
    observed.first_page_seconds = times.first().copied().unwrap_or(f64::NAN);
    let tenth = seconds / 10.0;
    let mut counts = [0u64; 10];
    for &at in &times {
        // A page answered after the tool's exit was timed would be past
        // the last tenth; it counts in it.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the tenth's index, clamped to 0 to 9"
        )]
        let part = ((at / tenth).floor().max(0.0) as usize).min(9);
        counts[part] += 1;
    }
    observed.curve = counts.iter().map(|&count| float(count) / tenth).collect();
    let end = |page: u64| -> Option<f64> {
        match records {
            Some(records) => records.get(&format!("{}{page}", site.prefix())).copied(),
            None => first.get(&page).copied(),
        }
    };
    let mut latencies: Vec<f64> = first
        .keys()
        .filter_map(|&page| {
            let discovered = match parent(page) {
                Some(parent) => *first.get(&parent)?,
                None => 0.0,
            };
            Some((end(page)? - discovered).max(0.0))
        })
        .collect();
    latencies.sort_by(f64::total_cmp);
    observed.latency = Percentiles {
        p50: percentile(&latencies, 0.50),
        p95: percentile(&latencies, 0.95),
        p99: percentile(&latencies, 0.99),
    };
    observed.latency_end = Some(match records {
        Some(_) => LatencyEnd::Record,
        None => LatencyEnd::Fetch,
    });
    observed.first_record_seconds =
        records.and_then(|records| records.values().copied().min_by(f64::total_cmp));
    observed.page_times = times;
    Ok(observed)
}

#[cfg(test)]
mod tests;
