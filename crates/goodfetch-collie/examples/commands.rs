//! Writes both Products' command lines into the folder given as the only
//! argument, as `commands.json`: `{"collie": <tree>, "foxhound": <tree>}`,
//! each tree a command's `name` and the `commands` under it, hidden ones
//! included. `xtask codegen` runs it for a workspace it was not built from,
//! and builds the same trees in process on its own (ADR 0072).
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous build tool; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::PathBuf;

use goodfetch_collie::executable::command;
use goodfetch_protocol::catalog::Product;
use serde_json::{Value, json};

fn tree(command: &clap::Command) -> Value {
    json!({
        "name": command.get_name(),
        "commands": command.get_subcommands().map(tree).collect::<Vec<_>>(),
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: commands <folder>")?,
    );
    let lines = json!({
        "collie": tree(&command(Product::Collie)),
        "foxhound": tree(&command(Product::Foxhound)),
    });
    let mut text = serde_json::to_string_pretty(&lines)?;
    text.push('\n');
    std::fs::write(folder.join("commands.json"), text)?;
    Ok(())
}
