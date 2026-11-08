//! The hidden `command-line` command (ADR 0072): the executable's command
//! line as JSON, each command's `name` and the `commands` under it, hidden
//! ones included, and the namespaces of the commands the `catalog_commands`
//! module generates, as the source of its committed `namespaces.rs`. `xtask
//! codegen` commits what [`printed_command_line`] gives each Product, through
//! Foxhound's `command-lines` example, so that `xtask` builds no engine crate and needs
//! no Console build; each Product's tests check the committed files against
//! what its executable prints.

use std::process::ExitCode;

use super::output;
use goodfetch_protocol::catalog::Product;
use serde_json::{Value, json};

use super::{OwnCommands, command};

/// The command line of `product`'s executable, whose own commands are
/// `own`, and the namespaces, as `command-line` prints them:
/// `{"command_line": <tree>, "namespaces": <source>}`.
#[must_use]
pub fn printed_command_line(product: Product, own: OwnCommands) -> Value {
    json!({
        "command_line": tree(&command(product, own)),
        "namespaces": super::catalog_commands::namespaces_source(),
    })
}

/// Prints [`printed_command_line`] of `product`'s executable on stdout.
pub(super) fn print(product: Product, own: OwnCommands) -> ExitCode {
    output::json(&printed_command_line(product, own));
    ExitCode::SUCCESS
}

/// `command` and the commands under it.
fn tree(command: &clap::Command) -> Value {
    json!({
        "name": command.get_name(),
        "commands": command.get_subcommands().map(tree).collect::<Vec<_>>(),
    })
}
