//! Foxhound, Goodfetch's Product for crawling, which contains Collie: the
//! crate published as Foxhound's Rust SDK, which holds Foxhound's executable
//! crate (ADRs 0087, 0090 and 0094). Its library exposes Foxhound's
//! composition of the engine: Collie's, then Foxhound's own contributions.
//! Its own crate, since it is a separate executable (STYLE.md, reason 1).
#![warn(missing_docs)]

mod composition;
mod sample;

pub use composition::{composition, composition_with};
