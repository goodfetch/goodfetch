//! The results store (ADR 0064; TESTING.md, After merge): the separate
//! `test-results` repository that keeps, for every workflow run attempt,
//! one compressed file holding the JUnit files as written, one JSON Lines
//! record per test run and one per job. Nothing is ever pruned. This module
//! owns the file's format, reads the store and ingests run attempts into it.

mod artifact;
mod ingest;
mod live_check;
mod release;
mod store;

pub(crate) use artifact::{Arch, Os};
pub(crate) use ingest::{Platform, Source, failed, ingest, pending, tested_commit};
pub(crate) use live_check::write as write_live_checks;
pub(crate) use release::{MAIN, RELEASE_SUITE, latest_on_main};
pub(crate) use store::{
    JobRun, Kept, Outcome, RunAttempt, TestRun, read_all, read_kept, read_month_and_previous,
};
