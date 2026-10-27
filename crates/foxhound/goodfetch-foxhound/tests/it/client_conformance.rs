//! The Client conformance cases of `conformance/clients/cases.toml` that
//! list the `cli` surface (ADR 0069), through Foxhound's executable, which
//! carries Collie's commands unchanged and passes each case Collie's
//! passes: one module per case named by its ID, each test named by the
//! mode it runs in.

mod deprecated_command_warns;
#[cfg(unix)]
mod interrupt_cancels_the_call;
mod sequence_follows_until_it_ends;

/// Foxhound's executable.
const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");
