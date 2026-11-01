//! A Foxhound crawl's Run (ADRs 0003, 0087, 0093 and 0102): `crawl.start`
//! begins it from its seeds, each a work item in its run state, and its
//! delivery leases each work item to the program's handler as a
//! `crawl_request.handle` request, commits the Records the handler staged
//! once while the lease holds, retries an attempt that failed or got no
//! answer under the Run's policy, and finishes the Run once every work item
//! is done; `crawl.serve` joins another Connection's handler to it, as
//! `workers=N` handler processes do. Each work item's ending commits its
//! outcome, which `run.outcomes` and `run.failed_requests` read from
//! committed state. A handler's `crawl_request.enqueue` adds a request
//! from inside its attempt through the Frontier, as
//! `crawl_request.enqueue_links` adds every link a locator matches on the
//! attempt's page, answering each element, a handler's
//! `crawl_request.push` checks and stages a Record with its attempt,
//! `run.add_seed` and
//! `run.add_seeds` add seeds to the running crawl, and what the crawl adds
//! to its Run's report goes in as the Run ends. Its own crate under
//! `crates/foxhound/`, since only
//! Foxhound links it (STYLE.md, reason 2); it fills the engine's
//! interfaces: a Run's driver, the tables a contribution keeps in every
//! run file, and reads of them.

mod code_free;
mod completeness;
mod dataset_list;
mod delivery;
mod enqueue;
mod intake;
mod labels;
mod lease;
mod outcomes;
mod policy;
mod progress;
mod push;
mod redeclare;
mod report;
mod retry;
mod seeding;
mod servable;
mod serving;
mod staging;
mod start;
mod tier_choice;
mod tier_race;
mod work_items;

pub use start::contribution;
