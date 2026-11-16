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
mod behind;
mod bounds;
mod domain_key;
mod forms;
mod identity;
mod invariants;
mod learn;
pub mod payload;
mod politeness;
mod queues;
mod robots_txt;
mod rules;
mod scope;
pub mod seeds;
pub mod sitemap;
mod staged;
pub mod trap_links;
mod unwritten;
mod url;

use std::sync::Arc;

use goodfetch_engine::Contribution;
use goodfetch_protocol::foxhound::frontier::{FRONTIER_LEARNING, LEARNING_SWITCHED_OFF};
use goodfetch_protocol::foxhound::settings::{POLITENESS_WEAKENED, PRESSING_HARDER};
use rusqlite::Connection;

pub use admit::{
    Candidate, Entering, Keyed, Link, admit, admit_in, admit_keyed, key_candidate, key_link,
};
pub use behind::Lesson;
pub use bounds::{Bounds, TABLES as BOUNDS_TABLES, completeness_reasons};
pub use domain_key::DomainKeys;
pub use identity::{
    Declared, Hop, PendingVisit, Reached, TABLES as VISITED_TABLES, admit_hop, admit_reached,
    fuzz_url_key, hold_reached, read_lookup, redirect_hop, redirect_reached, rewrite, url_key,
    visit_pending,
};
pub use invariants::Invariants;
pub use learn::{
    EndMerged, Ended, PageSeen, Redirect, completeness_reasons as learned_completeness_reasons,
    learn, learn_redirect,
};
pub use politeness::{
    Pace, crawl_delay_read, last_start, latency_measured, longest_wait, paces,
    refuse_second_copies, start_after, start_after_robots_txt, started_again, started_at_send,
};
pub use queues::DomainQueues;
pub use robots_txt::{SharedRobotsTxt, TABLES as ROBOTS_TXT_TABLES};
pub use rules::{TABLES as RULES_TABLES, learned_rules};
pub use scope::{Narrowing, Patterns, Scope, Strategy as ScopeStrategy, TABLES as SCOPE_TABLES};
pub use staged::{Admitting, Refused, Staged};
pub use unwritten::{Lookup, SharedBudget, UnwrittenVisits};
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
/// per Run as `politeness_weakened` (ADR 0086), and `frontier.learning`
/// switched off, recorded once per Run as `learning_switched_off` (ADR
/// 0138).
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
        .with_admission_gates(Arc::new(queues::DomainGates))
        .with_recorded_weakening(
            &FRONTIER_LEARNING,
            &LEARNING_SWITCHED_OFF,
            FRONTIER_LEARNING.default,
        );
    PRESSING_HARDER
        .into_iter()
        .fold(contribution, |contribution, key| {
            contribution.with_recorded_weakening(key.setting, &POLITENESS_WEAKENED, key.built_in)
        })
}

/// Has the Frontier's memory, when the Run holds the batches' visits
/// ([`Invariants::hold_unwritten`]), read on `connection`, the writer's,
/// the Frontier's state an admission would read otherwise, as the crawl
/// opens, after [`Invariants::fill_memory`]: the learned rules and
/// identities, the Bounds' counts and whether a trap mark is held. An
/// admission below the memory budget then reads none of it from the file
/// (#2169, ADR 0140's `F3.state_reads`).
///
/// # Errors
///
/// When the run file cannot be read.
pub fn keep_memory(invariants: &Invariants, connection: &Connection) -> rusqlite::Result<()> {
    admit::keep_memory(connection, invariants)?;
    invariants.unwritten().map_or(Ok(()), |unwritten| {
        trap_links::keep_marks(connection, unwritten)
    })
}

/// The statements a Run under `invariants` prepares once, as it starts or
/// resumes, on each connection of its run file's writer, so that no
/// page's admission prepares one, whichever connection it commits on
/// (ADR 0140, `F3.prepares`). Those the Run's start runs none of: the
/// Scope's lookup of a URL's measures, the learned rules' end of a streak,
/// their read of a parameter's offset and of a path's paged offsets, a
/// link seen again and a sitemap's read. And those a redirect hop's check
/// runs beside the Scope's lookup, which the start prepares only on the
/// connection its own write commits on, where a hop's check does not
/// (#2246): the rule that holds, the visited set's read and a learned
/// identity's. And those memory reads the learned rules and identities
/// with as an admission writes, with the step along the visited set's
/// methods that finds a rule's hosts' identities (#2169).
#[must_use]
pub fn admission_statements(invariants: &Invariants) -> Vec<&'static str> {
    invariants
        .scope()
        .strategy()
        .query()
        .into_iter()
        .chain([
            rules::STREAK_ENDED,
            rules::OFFSET_SEEN,
            rules::PAGED_OFFSETS,
        ])
        .chain(trap_links::SEEN_AGAIN)
        .chain(sitemap::READ)
        .chain([
            rules::HOLDING,
            rules::HELD,
            identity::VISITOR,
            identity::LEARNED,
            identity::FIRST_VISITED,
            unwritten::EVERY_LEARNED,
            unwritten::LEARNED_WITHIN,
        ])
        .collect()
}
