//! Goodfetch's contracts, defined once as Rust types (ADR 0038): every entry
//! of the Operation catalog and every contract generated beside it, each
//! marked with the smallest Product that carries it by where it is defined,
//! Foxhound's own in the `foxhound` module (ADRs 0095 and 0099).
//!
//! The `emit` example writes the committed JSON Schema bundle and catalog
//! file; `xtask codegen` runs it and generates every other surface from them.
//! The entries here are samples that carry the pipeline until each area
//! defines its own contracts.
#![warn(missing_docs)]

pub mod catalog;
pub mod emit;
pub mod foxhound;
pub mod foxhound_names;
pub mod journey;
pub mod result_document;
pub mod sample;
pub mod secret_reference;
pub mod wire_name;
