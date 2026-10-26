//! The Console build Foxhound's executable embeds (ADR 0057), which
//! `build.rs` writes the table of: Collie's, until Foxhound's own build
//! exists (#1202), so that the major's Daemon serves a Console whichever
//! Product runs it (ADR 0098). Only the executable includes this file, so
//! the library Foxhound's Rust SDK is serves no Console (ADR 0031).

include!(concat!(env!("OUT_DIR"), "/console_build.rs"));

/// Foxhound's composition, with its Console build.
pub(crate) fn composition() -> goodfetch_engine::Composition {
    goodfetch_foxhound::composition().with_console(&CONSOLE)
}
