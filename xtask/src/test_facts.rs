//! The facts `xtask` reads from crates it builds none of (ADR 0072): the
//! Chrome for Testing build Collie pins and Playwright's revision of it,
//! which `compare` runs, and the `tracing` targets the exercise audit reads.
//! `xtask codegen` writes them from their sources (`codegen::facts`).

include!("../generated/test_facts.rs");
