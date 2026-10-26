//! Embeds Foxhound's Console build (ADRs 0057 and 0087), which `xtask
//! console build` puts in `console-build/` beside this script: Collie's
//! screens composed with Foxhound's, so that the major's Foxhound Daemon
//! serves every route of Collie's build and Foxhound's screens (ADRs 0094
//! and 0098). The table is Collie's build script's (`build/console.rs`).
#![expect(
    clippy::disallowed_methods,
    reason = "a build script is synchronous and reads its own environment and files"
)]

#[path = "../../goodfetch-collie/build/console.rs"]
mod console;

fn main() {
    console::embed("console-build");
}
