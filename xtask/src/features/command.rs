//! The `command:` Features: one per command path of each Product's
//! executable, from clap's `Command` (ADR 0072), its names below the
//! executable's joined by dots (`browsers.install`): a Feature ID is also a
//! test runner's tag, and Vitest refuses a tag name with a space.
//! A command group is reached through the commands under it, so only a
//! command with none below it is a Feature; a hidden command is one too.
//! Foxhound's executable keeps every command of Collie's (ADRs 0090 and
//! 0095), so a command is Foxhound's alone when ADR 0087's generated table
//! of Foxhound's names lists it, and both Products' otherwise.

use std::collections::BTreeSet;

use serde::Deserialize;

use super::entry::{Carriers, Entry};
use super::files::{Kind, Status};

/// The section of the table of Foxhound's names that lists the commands only
/// Foxhound's executable carries.
pub(crate) const SECTION: &str = "commands";

/// A command and those under it, as clap's `Command` holds them and the
/// executable's hidden `command-line` command prints them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandTree {
    /// Its name.
    name: String,
    /// The commands under it.
    commands: Vec<CommandTree>,
}

impl CommandTree {
    /// The command named `name` directly under this one.
    pub(crate) fn command(&self, name: &str) -> Option<&Self> {
        self.commands.iter().find(|command| command.name == name)
    }

    /// Whether commands sit under it, so the word after it names one.
    pub(crate) fn is_group(&self) -> bool {
        !self.commands.is_empty()
    }

    /// The paths of the commands below this executable's root.
    fn paths(&self) -> BTreeSet<String> {
        fn walk(tree: &CommandTree, prefix: &str, paths: &mut BTreeSet<String>) {
            let path = if prefix.is_empty() {
                tree.name.clone()
            } else {
                format!("{prefix}.{}", tree.name)
            };
            if tree.commands.is_empty() {
                paths.insert(path);
            } else {
                for command in &tree.commands {
                    walk(command, &path, paths);
                }
            }
        }
        let mut paths = BTreeSet::new();
        for command in &self.commands {
            walk(command, "", &mut paths);
        }
        paths
    }
}

/// The command lines of both Products' executables.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandLines {
    pub(crate) collie: CommandTree,
    pub(crate) foxhound: CommandTree,
}

impl CommandLines {
    /// The paths of the commands only Foxhound's executable carries: the
    /// table's `commands` section.
    pub(crate) fn foxhound_only(&self) -> BTreeSet<String> {
        let collie = self.collie.paths();
        self.foxhound
            .paths()
            .into_iter()
            .filter(|path| !collie.contains(path))
            .collect()
    }
}

/// Every command of either executable; none when the workspace has no
/// executable crate.
pub(super) fn entries(command_lines: Option<&CommandLines>) -> Vec<Entry> {
    let paths: BTreeSet<String> = command_lines
        .into_iter()
        .flat_map(|lines| {
            lines
                .collie
                .paths()
                .into_iter()
                .chain(lines.foxhound.paths())
        })
        .collect();
    paths
        .into_iter()
        .map(|path| Entry {
            kind: Kind::Command,
            name: path,
            area: None,
            milestone: None,
            status: Status::Active,
            carriers: Carriers::Table(SECTION),
        })
        .collect()
}
