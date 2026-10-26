//! Embeds Collie's Console build (ADR 0057), which `xtask console build`
//! puts in `console-build/` beside this script: the table Collie's
//! executable includes (`build/console.rs`).
#![expect(
    clippy::disallowed_methods,
    reason = "a build script is synchronous and reads its own environment and files"
)]

#[path = "build/console.rs"]
mod console;

fn main() {
    console::embed("console-build");
}
