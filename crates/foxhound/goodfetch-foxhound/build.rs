//! Embeds the Console build Foxhound's executable serves (ADR 0057): until
//! Foxhound's own build exists (#1202), Collie's, from the folder `xtask
//! console build` writes beside Collie's crate, so that the major's Daemon
//! serves a Console whichever Product runs it (ADR 0098). The table is
//! Collie's build script's (`build/console.rs`).
#![expect(
    clippy::disallowed_methods,
    reason = "a build script is synchronous and reads its own environment and files"
)]

#[path = "../../goodfetch-collie/build/console.rs"]
mod console;

fn main() {
    console::embed("../../goodfetch-collie/console-build");
}
