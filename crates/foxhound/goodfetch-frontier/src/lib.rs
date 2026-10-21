//! The Frontier of a Foxhound crawl (ADRs 0004 and 0087): the domain key
//! of each crawl request, and the bounded per-domain queues that fill the
//! engine's gate in admission, served round-robin, one request per domain
//! per turn, priority ordering requests within a domain only. Its own crate
//! under `crates/foxhound/`, since only Foxhound links it (STYLE.md, reason
//! 2).

mod domain_key;
mod queues;

use std::sync::Arc;

use goodfetch_engine::Contribution;

pub use domain_key::DomainKeys;
pub use queues::DomainQueues;

/// What Foxhound's Frontier adds to the engine: its per-domain queues as
/// the gate in admission, and the table they journal each domain's wait in.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-frontier")
        .with_run_state_tables(queues::TABLES)
        .with_admission_gates(Arc::new(queues::DomainGates))
}
