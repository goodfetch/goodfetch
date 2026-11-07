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
    /// The bounds `resolved` gives a Foxhound Run.
    pub fn of(resolved: &Resolved) -> Self {
        Self {
            max_size: usize::try_from(resolved.get::<u64>(&SESSION_POOL_MAX_SIZE))
                .unwrap_or(usize::MAX)
                .max(1),
            max_uses: resolved.get(&SESSION_POOL_MAX_USES),
            max_age: resolved.get(&SESSION_POOL_MAX_AGE),
            max_error_score: resolved.get(&SESSION_POOL_MAX_ERROR_SCORE),
        }
    }
}
