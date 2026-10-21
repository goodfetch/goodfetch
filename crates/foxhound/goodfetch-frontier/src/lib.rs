//! The Frontier of a Foxhound crawl (ADRs 0004, 0087, 0093, 0102 and
//! 0120): the one parse of every URL a crawl meets, by the WHATWG URL
//! Standard; `admit(candidate, context)`, which answers every seed and
//! enqueue added or not added with its reason; each seed's answer, which
//! `run.seeds` reads; the domain key of each crawl request; and the bounded
//! per-domain queues that fill the engine's gate in admission, served
//! round-robin, one request per domain per turn, priority ordering requests
//! within a domain only. Its own crate under `crates/foxhound/`, since only
//! Foxhound links it (STYLE.md, reason 2).

mod admit;
mod domain_key;
mod queues;
pub mod seeds;
mod url;

use std::sync::Arc;

use goodfetch_engine::Contribution;

pub use admit::{Candidate, Identified, admit};
pub use domain_key::DomainKeys;
pub use queues::DomainQueues;
pub use url::document_base;

/// A URL as the WHATWG URL Standard parsed it.
pub use ::url::Url;

/// What Foxhound's Frontier adds to the engine: `run.seeds`, its per-domain
/// queues as the gate in admission, and the tables that keep each seed's
/// answer and each domain's wait.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-frontier")
        .with_handler(seeds::handler())
        .with_run_state_tables(queues::TABLES)
        .with_run_state_tables(seeds::TABLES)
        .with_admission_gates(Arc::new(queues::DomainGates))
}
