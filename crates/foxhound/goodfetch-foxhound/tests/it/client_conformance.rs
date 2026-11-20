//! The Client conformance cases of `conformance/clients/cases.toml` that
//! list the `cli` surface (ADR 0069) and need what only Foxhound's
//! executable carries: one module per case named by its ID, each test
//! named by the mode it runs in. A case Collie's executable carries runs
//! once, on Collie's (ADR 0094).

mod sequence_follows_until_it_ends;

/// Foxhound's executable.
const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");
