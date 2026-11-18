//! The invariants a test's run file gives, built as the Run's start
//! builds them, and seeds checked against them.

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_frontier::{Bounds, Scope};
use goodfetch_protocol::foxhound::crawl::Seed;
use rusqlite::Connection;

use super::Invariants;
use crate::labels::Fragments;
use crate::work_items::{self, CheckedSeed};

/// The Label `page`, which every test's seeds name, declared in the run
/// file as a start declaring it records it: it goes after the frontier's
/// Payload tables.
pub(crate) const PAGE_DECLARED: RunStateTables = RunStateTables {
    name: "the test's declared Label",
    statements: &[
        "INSERT INTO payload_labels (label) VALUES ('page') ON CONFLICT (label) DO NOTHING",
    ],
};

/// The invariants of `scope`, with the Bounds, Payload rules and fragments
/// the run file `connection` holds.
pub(crate) fn within(connection: &Connection, scope: Scope) -> rusqlite::Result<Invariants> {
    Ok(Invariants::new(
        goodfetch_frontier::Invariants::new(scope, Bounds::read(connection)?)
            .expect("invariant: the test's path budgets are globs"),
        PayloadRules::read(connection)?,
        Fragments::read(connection)?,
    ))
}

/// `seeds`, checked as the call that gave them checks them.
pub(crate) fn checked(
    invariants: &Invariants,
    seeds: impl IntoIterator<Item = Seed>,
) -> Vec<CheckedSeed> {
    work_items::check_seeds(invariants, seeds, &CountedWork::default())
        .expect("invariant: the test's seeds are valid")
}
