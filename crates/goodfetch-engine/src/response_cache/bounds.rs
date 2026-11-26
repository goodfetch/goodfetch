//! The Response cache's bounds (ADR 0046): `cache.max_size`, the most
//! bytes it keeps, its index and its bodies together, beyond which its
//! least recently used entries leave, and `cache.max_age_ceiling`, the
//! longest it keeps an entry, whatever its freshness and in every mode.
//! While its filesystem is at the `low` or `critical` storage level it
//! stores nothing new, so it evicts nothing beyond its own cap (ADR 0122).
//!
//! Which entries leave is a pure function of what they hold
//! ([`evictions`]): a body several entries share is counted once, and
//! freed only with the last of them.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::response_cache::{CACHE_MAX_AGE_CEILING, CACHE_MAX_SIZE};

use crate::settings::{Resolved, catalog_default};

/// The bounds a Run's settings give the cache, read where the Run starts,
/// in user configuration or an Operator policy only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    /// The most bytes kept, the index's and the bodies' together.
    pub max_size: u64,
    /// The longest an entry is kept from its fetch, or its last
    /// revalidation.
    pub max_age_ceiling: Duration,
}

impl Bounds {
    /// The bounds `settings` set.
    pub fn of(settings: &Resolved) -> Self {
        Self {
            max_size: settings.get(&CACHE_MAX_SIZE),
            max_age_ceiling: settings.get(&CACHE_MAX_AGE_CEILING),
        }
    }

    /// Whether an entry fetched, or last revalidated, at `fetched_at` has
    /// outlived the ceiling at `now`, which then serves it in no mode.
    pub(crate) fn outlived(&self, fetched_at: SystemTime, now: SystemTime) -> bool {
        now.duration_since(fetched_at)
            .is_ok_and(|age| age > self.max_age_ceiling)
    }
}

impl Default for Bounds {
    /// The catalog's: 10 GB and 30 days.
    fn default() -> Self {
        Self {
            max_size: catalog_default(&CACHE_MAX_SIZE),
            max_age_ceiling: catalog_default(&CACHE_MAX_AGE_CEILING),
        }
    }
}

/// An entry as eviction weighs it: its body's size, and how many entries
/// hold that body, this one included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Weighed {
    /// Its body's SHA-256.
    pub(crate) body: String,
    /// Its body's size in bytes.
    pub(crate) size: u64,
    /// The entries holding its body, as the index counts them.
    pub(crate) holders: u64,
}

/// How many of `oldest`, entries in least recently used order, leave so
/// that `held` bytes fall to `cap` at most: the fewest from the front, a
/// body counted once and freed with the last entry holding it. All of
/// them when that is not enough, the caller then reading the next ones;
/// none when `held` is within `cap`.
pub(crate) fn evictions(held: u64, cap: u64, oldest: &[Weighed]) -> usize {
    let mut held = held;
    let mut left: HashMap<&str, u64> = HashMap::new();
    for (taken, entry) in oldest.iter().enumerate() {
        if held <= cap {
            return taken;
        }
        let holders = left.entry(entry.body.as_str()).or_insert(entry.holders);
        *holders = holders.saturating_sub(1);
        if *holders == 0 {
            held = held.saturating_sub(entry.size);
        }
    }
    oldest.len()
}

/// `at` as the index writes its times: RFC 3339 in UTC to the millisecond,
/// always of one width, so that their text sorts as they do, which the
/// least recently used order and the ceiling's deletion read.
pub(crate) fn index_time(at: SystemTime) -> String {
    let at = jiff::Timestamp::try_from(at)
        .expect("invariant: the engine holds no point in time RFC 3339 cannot write");
    format!("{at:.3}")
}

#[cfg(test)]
mod tests;
