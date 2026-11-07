//! Foxhound's own commands (ADRs 0087, 0090 and 0099): the commands its
//! executable writes by hand among those Collie's executable generates from
//! Foxhound's catalog, which [`COMMANDS`] passes to Collie's executable
//! entry. `crawl start <url>…` builds the code-free crawl's `extraction` and
//! seeds from its flags and follows its Run under `--follow`
//! ([`crawl_start`]); `run export <run> <dataset> --out <path|->` writes the
//! Export's file ([`run_export`]). They live in Foxhound's crate, so the
//! crate both Products link names no Foxhound type for them (ADR 0087).

use clap::Command;
use goodfetch_collie::executable::generated::{self, OwnCommand, OwnCommands};
use goodfetch_protocol::catalog::Product;

mod crawl_start;
mod run_export;

/// Foxhound's executable's own commands, which its `main` passes to
/// Collie's executable entry.
pub const COMMANDS: OwnCommands = OwnCommands {
    commands: OWN,
    members,
};

/// The commands Foxhound's executable writes by hand.
const OWN: &[OwnCommand] = &[crawl_start::COMMAND, run_export::COMMAND];

/// The commands of the generated group `group` of Foxhound's executable.
fn members(group: Command) -> Command {
    generated::members(group, Product::Foxhound, OWN)
}

#[cfg(test)]
mod tests;
