//! Collie, Goodfetch's Product for browser automation: the crate published
//! as Collie's Rust SDK, which holds Collie's executable crate (ADRs 0087,
//! 0090 and 0094). Its library exposes Collie's composition of the engine,
//! which Foxhound's starts from, and, under the `executable` feature, the
//! executable's entry, which both Products' `main` call. Its own crate,
//! since it is a separate executable (STYLE.md, reason 1).
#![warn(missing_docs)]

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the pages read a cause and its rule; `blocked` carries the rest of the evidence (#711)"
    )
)]
mod block_detection;
mod composition;
#[cfg(feature = "executable")]
pub mod executable;
mod pins;
mod sample;

pub use composition::{composition, composition_with};
pub use pins::pinned_artifacts;
