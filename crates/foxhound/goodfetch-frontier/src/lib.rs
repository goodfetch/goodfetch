//! The Frontier of a Foxhound crawl (ADRs 0004, 0053 and 0087): the domain
//! key of each crawl request, the bounded per-domain queues that fill the
//! engine's gate in admission, served round-robin, one request per domain
//! per turn, priority ordering requests within a domain only, and the
//! per-domain Politeness their keys follow in the per-user Politeness file,
//! shared by every run of the user (the per-domain cap, delay, backoff,
//! `Retry-After` and 429 halving). Its own crate
//! under `crates/foxhound/`, since only Foxhound links it (STYLE.md, reason
//! 2).

mod domain_key;
mod politeness;
mod queues;

use std::sync::Arc;

use goodfetch_engine::Contribution;

pub use domain_key::DomainKeys;
pub use queues::DomainQueues;

/// What Foxhound's Frontier adds to the engine: its per-domain queues as
/// the gate in admission, with its per-domain Politeness as the rule of
/// their keys, the table they journal each domain's wait in, and the table
/// of the per-user Politeness file the rule keeps each domain's shared state
/// in.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-frontier")
        .with_run_state_tables(queues::TABLES)
        .with_politeness_tables(politeness::TABLES)
        .with_admission_gates(Arc::new(queues::DomainGates))
}
