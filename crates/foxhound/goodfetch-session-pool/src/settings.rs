//! The bounds and rules of a crawl's Session pool, read from the Run's
//! resolved `session.pool.*` settings, resolved once as the Run opens (ADR
//! 0138).

use std::time::Duration;

use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::session_pool::{
    SESSION_POOL_AFFINITY, SESSION_POOL_GROWTH, SESSION_POOL_MAX_AGE, SESSION_POOL_MAX_ERROR_SCORE,
    SESSION_POOL_MAX_SIZE, SESSION_POOL_MAX_USES, SESSION_POOL_SESSION_CONCURRENCY,
};

/// How many Sessions a pool keeps, how long each serves, and when it opens
/// a new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolSettings {
    /// `session.pool.max_size`: Sessions open at once, at least 1.
    pub max_size: usize,
    /// `session.pool.max_uses`: requests one Session serves.
    pub max_uses: u64,
    /// `session.pool.max_age`: how long after it opened a Session serves.
    pub max_age: Duration,
    /// `session.pool.max_error_score`: the error score that retires a
    /// Session.
    pub max_error_score: u64,
    /// `session.pool.growth`: when a new Session opens.
    pub growth: Growth,
    /// `session.pool.session_concurrency`: the requests one Session serves
    /// at once on demand, at least 1; `None`, unset, for any number.
    pub session_concurrency: Option<u64>,
    /// `session.pool.affinity`: which open Session with room a request
    /// prefers on demand.
    pub affinity: Affinity,
}

/// When a pool opens a new Session (`session.pool.growth`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Growth {
    /// Only when no open Session of the client a request needs, at its
    /// domain's proxy tier, has room, or when its domain owes one.
    #[default]
    OnDemand,
    /// For every request while the pool has room, as Crawlee does (#920).
    Eager,
}

/// Which open Session with room a request prefers on demand
/// (`session.pool.affinity`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    /// One whose last request went to the request's host.
    #[default]
    Host,
    /// None: they are drawn among at random.
    None,
}

impl PoolSettings {
    /// The bounds `resolved` gives a Foxhound Run.
    pub fn of(resolved: &Resolved) -> Self {
        Self {
            max_size: usize::try_from(resolved.get::<u64>(&SESSION_POOL_MAX_SIZE))
                .unwrap_or(usize::MAX)
                .max(1),
            max_uses: resolved.get(&SESSION_POOL_MAX_USES),
            max_age: resolved.get(&SESSION_POOL_MAX_AGE),
            max_error_score: resolved.get(&SESSION_POOL_MAX_ERROR_SCORE),
            growth: match resolved.get(&SESSION_POOL_GROWTH) {
                "eager" => Growth::Eager,
                _ => Growth::OnDemand,
            },
            session_concurrency: resolved
                .get::<Option<u64>>(&SESSION_POOL_SESSION_CONCURRENCY)
                .map(|concurrency| concurrency.max(1)),
            affinity: match resolved.get(&SESSION_POOL_AFFINITY) {
                "none" => Affinity::None,
                _ => Affinity::Host,
            },
        }
    }
}
