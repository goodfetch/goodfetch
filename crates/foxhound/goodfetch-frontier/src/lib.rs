//! The Frontier of a Foxhound crawl (ADRs 0004, 0053, 0087, 0093, 0102
//! and 0120): the one parse of every URL a crawl meets, by the WHATWG URL
//! Standard; `admit(candidate, context)`, which answers every seed and
//! enqueue added or not added with its reason; each seed's answer, which
//! `run.seeds` reads; the domain key of each crawl request; the bounded
//! per-domain queues that fill the engine's gate in admission, served
//! round-robin, one request per domain per turn, priority and `forefront`
//! ordering requests within a domain only; and the per-domain Politeness
//! their keys follow in the per-user Politeness file, shared by every run of
//! the user (the per-domain cap, delay, backoff, `Retry-After` and 429
//! halving); and the rules it learns from the pages a crawl fetched
//! (ADR 0005): query parameters with no effect, equivalent forms (ADR
//! 0120), trap paths and cursor pagination preferred to offset; and each
//! crawl request's Payload, kept apart from the engine's fields, checked
//! against its Label's JSON Schema and `crawl.payload.max_size`, with the
//! schema hashes a resume compares (ADR 0082). Its own crate under `crates/foxhound/`, since only Foxhound
//! links it (STYLE.md, reason 2).

mod admit;
mod bounds;
mod domain_key;
mod forms;
mod identity;
mod learn;
pub mod payload;
mod politeness;
mod queues;
mod robots_txt;
mod rules;
mod scope;
pub mod seeds;
mod url;

use std::sync::Arc;

use goodfetch_engine::Contribution;

pub use admit::{Candidate, admit};
pub use bounds::{Bounds, TABLES as BOUNDS_TABLES, completeness_reasons};
pub use domain_key::DomainKeys;
pub use identity::{
    Declared, Hop, TABLES as VISITED_TABLES, fuzz_url_key, redirect_hop, redirect_reached, url_key,
};
pub use learn::{EndMerged, PageSeen, Redirect, learn, learn_redirect};
pub use politeness::{last_start, longest_wait, refuse_second_copies, started_again};
pub use queues::DomainQueues;
pub use robots_txt::SharedRobotsTxt;
pub use rules::{
    TABLES as RULES_TABLES, completeness_reasons as learned_completeness_reasons, learned_rules,
};
pub use scope::{Narrowing, Patterns, Scope, Strategy as ScopeStrategy, TABLES as SCOPE_TABLES};
pub use url::document_base;

/// A URL as the WHATWG URL Standard parsed it.
pub use ::url::Url;

/// What Foxhound's Frontier adds to the engine: `run.seeds`, its per-domain
/// queues as the gate in admission, with its per-domain Politeness as the
/// rule of their keys, the tables that keep each seed's answer and each
/// domain's wait, and the table of the per-user Politeness file the rule
/// keeps each domain's shared state in.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-frontier")
        .with_handler(seeds::handler())
        .with_run_state_tables(queues::TABLES)
        .with_run_state_tables(seeds::TABLES)
        .with_run_state_tables(identity::TABLES)
        .with_run_state_tables(scope::TABLES)
        .with_run_state_tables(bounds::TABLES)
        .with_run_state_tables(rules::TABLES)
        .with_run_state_tables(payload::TABLES)
        .with_politeness_tables(politeness::TABLES)
        .with_politeness_tables(robots_txt::TABLES)
        .with_admission_gates(Arc::new(queues::DomainGates))
}
