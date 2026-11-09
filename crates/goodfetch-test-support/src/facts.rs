//! The facts test support reads from crates it may not depend on (ADR
//! 0062): the browser builds the Products pin, which `browsers` lays out,
//! and the engine's `tracing` targets, which the exercise record keeps.
//! `xtask codegen` writes them from their sources.

include!("../generated/facts.rs");
