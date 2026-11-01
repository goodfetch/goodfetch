//! The Console build Foxhound's executable embeds (ADRs 0057 and 0087),
//! which `build.rs` writes the table of: Foxhound's own, Collie's screens
//! composed with Foxhound's, so that the major's Foxhound Daemon serves
//! every route of Collie's build and Foxhound's screens (ADRs 0094 and
//! 0098). Only the executable includes this file, so the library
//! Foxhound's Rust SDK is serves no Console (ADR 0031).

include!(concat!(env!("OUT_DIR"), "/console_build.rs"));

/// Foxhound's composition, with its Console build.
pub(crate) fn composition() -> goodfetch_engine::Composition {
    goodfetch_foxhound::composition().with_console(&CONSOLE)
}
