//! The Client conformance cases of `conformance/clients/cases.toml` that
//! list the `cli` surface (ADR 0069), one module per case named by its ID,
//! each test named by the mode it runs in.

mod deprecated_command_warns;
mod interrupt_cancels_the_call;
mod sequence_follows_until_it_ends;

/// Collie's executable, which carries every case's commands.
const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");
