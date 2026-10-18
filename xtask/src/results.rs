//! The results store (ADR 0064; TESTING.md, After merge): the separate
//! `test-results` repository that keeps, for every workflow run attempt,
//! one compressed file holding the JUnit files as written, one JSON Lines
//! record per test run and one per job. Nothing is ever pruned. This module
//! owns the file's format, reads the store and ingests run attempts into it.

mod artifact;
mod ingest;
mod junit;
mod store;

pub(crate) use ingest::{Source, ingest};
pub(crate) use store::{Outcome, TestRun, read_all};
