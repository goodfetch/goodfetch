//! Writes both Products' command lines into the folder given as the only
//! argument, as `command-lines.json`: `{"collie": <printed>, "foxhound":
//! <printed>}`, each what that Product's executable prints for its hidden
//! `command-line` command. `xtask codegen` runs it and commits the result
//! (ADR 0072): it needs neither executable's Console build, and Foxhound's
//! executable starts from Collie's entry, whose command line this is.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous build tool; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::PathBuf;

use goodfetch_collie::executable::printed_command_line;
use goodfetch_protocol::catalog::Product;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: command-lines <folder>")?,
    );
    let printed = json!({
        "collie": printed_command_line(Product::Collie),
        "foxhound": printed_command_line(Product::Foxhound),
    });
    std::fs::write(
        folder.join("command-lines.json"),
        serde_json::to_vec(&printed)?,
    )?;
    Ok(())
}
