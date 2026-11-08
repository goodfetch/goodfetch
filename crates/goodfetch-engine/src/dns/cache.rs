//! The engine's resolver cache (ADR 0118): the answers of its own resolver
//! as the records say, by name and IP family. An answer is kept for its
//! TTL, never a fixed time; an `NXDOMAIN` or an empty answer for its
//! negative TTL (RFC 2308), and not at all without an SOA to give one; a
//! server failure or a timeout not at all, so a retry asks again. A name
//! is forgotten whole once every address it gave failed to connect (RFC
//! 8305 section 9). A pure function of the outcomes it is given and the
//! moments it is asked at.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use goodfetch_protocol::dns::IpFamily;
use tokio::time::Instant;

use crate::host_name;

/// The most names kept (STYLE.md: every buffer is bounded). A full cache
/// drops what expires soonest to keep a new answer.
const CAPACITY: usize = 10_000;

/// What one query of one family gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// These addresses, never empty, which the records let a resolver keep
    /// for `ttl`, their smallest TTL.
    Addresses {
        addresses: Vec<IpAddr>,
        ttl: Duration,
    },
    /// No address: the name does not exist or has none of the family. Its
    /// SOA's negative TTL, when the answer carried one.
    Absent { negative_ttl: Option<Duration> },
    /// A server failure, a refusal or a timeout: no answer.
    Failure,
}

/// What the cache holds for one name and family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kept {
    /// The name's addresses of the family.
    Addresses(Vec<IpAddr>),
    /// The name has no address of the family.
    Absent,
}

struct Entry {
    kept: Kept,
    until: Instant,
}

/// The kept answers, each until its moment.
pub(crate) struct AnswerCache {
    entries: BTreeMap<(String, IpFamily), Entry>,
    capacity: usize,
}

impl Default for AnswerCache {
    fn default() -> Self {
        Self::with_capacity(CAPACITY)
    }
}

impl AnswerCache {
    /// A cache that keeps at most `capacity` answers.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            capacity,
        }
    }

    /// What is kept for `name`'s addresses of `family` at `now`.
    pub(crate) fn kept(&self, name: &str, family: IpFamily, now: Instant) -> Option<Kept> {
        self.entries
            .get(&(host_name::lookup_key(name), family))
            .filter(|entry| now < entry.until)
            .map(|entry| entry.kept.clone())
    }

    /// Keeps what a query of `name`'s `family` gave at `now`, for as long
    /// as its records allow.
    pub(crate) fn keep(&mut self, name: &str, family: IpFamily, outcome: Outcome, now: Instant) {
        let (kept, lifetime) = match outcome {
            Outcome::Addresses { addresses, ttl } => (Kept::Addresses(addresses), ttl),
            Outcome::Absent {
                negative_ttl: Some(negative_ttl),
            } => (Kept::Absent, negative_ttl),
            Outcome::Absent { negative_ttl: None } | Outcome::Failure => return,
        };
        let key = (host_name::lookup_key(name), family);
        if lifetime.is_zero() {
            self.entries.remove(&key);
            return;
        }
        if !self.entries.contains_key(&key) {
            self.make_room(now);
        }
        self.entries.insert(
            key,
            Entry {
                kept,
                until: now + lifetime,
            },
        );
    }

    /// Forgets every answer of `name`.
    pub(crate) fn forget(&mut self, name: &str) {
        let name = host_name::lookup_key(name);
        for family in [IpFamily::Ipv4, IpFamily::Ipv6] {
            self.entries.remove(&(name.clone(), family));
        }
    }

    /// Drops expired answers, then those that expire soonest, until one
    /// more fits.
    fn make_room(&mut self, now: Instant) {
        if self.entries.len() < self.capacity {
            return;
        }
        self.entries.retain(|_, entry| now < entry.until);
        while self.entries.len() >= self.capacity.max(1) {
            let Some(soonest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.until)
                .map(|(key, _)| key.clone())
            else {
                return;
            };
            self.entries.remove(&soonest);
        }
    }
}

#[cfg(test)]
mod tests;
