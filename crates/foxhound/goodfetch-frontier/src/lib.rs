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
//! schema hashes a resume compares (ADR 0082); and on the browser tier,
//! which links a person cannot see, traps not added (`trap_links`, ADR
//! 0035). Its own crate under `crates/foxhound/`, since only Foxhound
//! links it (STYLE.md, reason 2).

mod admit;
pub mod backoff_classes;
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
pub mod sitemap;
pub mod trap_links;
mod url;

use std::sync::Arc;

use goodfetch_engine::Contribution;
use goodfetch_protocol::foxhound::settings::{POLITENESS_WEAKENED, PRESSING_HARDER};

pub use admit::{Candidate, admit};
pub use bounds::{Bounds, TABLES as BOUNDS_TABLES, completeness_reasons};
pub use domain_key::DomainKeys;
pub use identity::{
    Declared, Hop, TABLES as VISITED_TABLES, admitted_identities, fuzz_url_key, redirect_hop,
    redirect_reached, rewrite, url_key,
};
pub use learn::{EndMerged, PageSeen, Redirect, learn, learn_redirect};
pub use politeness::{
    Pace, crawl_delay_read, last_start, latency_measured, longest_wait, refuse_second_copies,
    start_after, start_after_robots_txt, started_again, started_at_send,
};
pub use queues::DomainQueues;
pub use robots_txt::SharedRobotsTxt;
pub use rules::{
    TABLES as RULES_TABLES, completeness_reasons as learned_completeness_reasons, learned_rules,
};
pub use scope::{Narrowing, Patterns, Scope, Strategy as ScopeStrategy, TABLES as SCOPE_TABLES};
pub use url::{document_base, resolve_link};

/// A URL as the WHATWG URL Standard parsed it.
pub use ::url::Url;

/// A document's encoding, as the WHATWG Encoding Standard names it, in
/// which its links' queries are percent-encoded (ADR 0120).
pub use ::encoding_rs::Encoding;

/// What Foxhound's Frontier adds to the engine: `run.seeds`, its per-domain
/// queues as the gate in admission, with its per-domain Politeness as the
/// rule of their keys, the tables that keep each seed's answer and each
/// domain's wait, the table of the per-user Politeness file the rule
/// keeps each domain's shared state in, and the Politeness and
/// Site-preferences values that press a site harder, each recorded once
/// per Run as `politeness_weakened` (ADR 0086).
pub fn contribution() -> Contribution {
    let contribution = Contribution::new("goodfetch-frontier")
        .with_handler(seeds::handler())
        .with_run_state_tables(queues::TABLES)
        .with_run_state_tables(seeds::TABLES)
        .with_run_state_tables(sitemap::TABLES)
        .with_run_state_tables(trap_links::TABLES)
        .with_run_state_tables(identity::TABLES)
        .with_run_state_tables(scope::TABLES)
        .with_run_state_tables(bounds::TABLES)
        .with_run_state_tables(rules::TABLES)
        .with_run_state_tables(payload::TABLES)
        .with_politeness_tables(politeness::TABLES)
        .with_politeness_tables(robots_txt::TABLES)
        .with_admission_gates(Arc::new(queues::DomainGates));
    PRESSING_HARDER
        .into_iter()
        .fold(contribution, |contribution, setting| {
            contribution.with_recorded_weakening(setting, &POLITENESS_WEAKENED)
        })
}
