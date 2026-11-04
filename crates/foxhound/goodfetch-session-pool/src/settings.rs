//! The bounds of a crawl's Session pool, read from the Run's resolved
//! `session.pool.*` settings, resolved once as the Run opens (ADR 0138).

use std::time::Duration;

use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::session_pool::{
    SESSION_POOL_MAX_AGE, SESSION_POOL_MAX_ERROR_SCORE, SESSION_POOL_MAX_SIZE,
    SESSION_POOL_MAX_USES,
};

/// How many Sessions a pool keeps and how long each serves.
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
}

impl PoolSettings {
    /// The bounds `resolved` gives a Foxhound Run, whose keys always hold
    /// a value of their kind.
    pub fn of(resolved: &Resolved) -> Self {
        let count = |key: &str| {
            resolved
                .count(key)
                .expect("invariant: the pool's count settings resolve to a count")
        };
        Self {
            max_size: usize::try_from(count(SESSION_POOL_MAX_SIZE.name))
                .unwrap_or(usize::MAX)
                .max(1),
            max_uses: count(SESSION_POOL_MAX_USES.name),
            max_age: resolved
                .duration(SESSION_POOL_MAX_AGE.name)
                .expect("invariant: `session.pool.max_age` resolves to a duration"),
            max_error_score: count(SESSION_POOL_MAX_ERROR_SCORE.name),
        }
    }
}
