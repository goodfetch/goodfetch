//! The Proving ground as a black box (spec #152, Seam 1): each test starts
//! the binary on a seed and drives it with a plain HTTP client, never an
//! engine crate nor either Product, so the oracle is never tested with a tool
//! it judges.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start the Proving ground's binary as a child process"
)]

mod agent;
mod clients;
mod fake_model;
mod model_recordings;
mod observations;
mod oracle;
mod pages;
mod proving_ground;
mod replay;
mod serve;
mod task_list;
mod transport;
