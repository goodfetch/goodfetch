//! Collie's Console build, which `build.rs` embeds from `xtask console
//! build`'s output (ADR 0057), and the composition Collie's executable runs:
//! Collie's, serving that Console. Only the executable includes this file,
//! so the library an SDK crate links serves no Console (ADR 0031).

include!(concat!(env!("OUT_DIR"), "/console_build.rs"));

/// Collie's composition, with Collie's Console build.
pub(crate) fn composition() -> goodfetch_engine::Composition {
    goodfetch_collie::composition().with_console(&CONSOLE)
}
