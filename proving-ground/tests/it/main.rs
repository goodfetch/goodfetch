//! The Proving ground as a black box (spec #152, Seam 1): each test starts
//! the binary on a seed and drives it with a plain HTTP client, never an
//! engine crate nor either Product, so the oracle is never tested with a tool
//! it judges.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start the Proving ground's binary as a child process"
)]

mod actionability;
mod agent;
mod browserless;
mod clients;
mod fake_model;
mod login;
mod model_recordings;
mod observations;
mod oracle;
mod pages;
mod portal;
mod proving_ground;
mod rendered;
mod replay;
mod schedule;
mod serve;
mod task_list;
mod transport;
