//! Whether a command an enforced row names exists: a recipe of the justfile, a
//! command of `xtask`, a file of the repository, or text a recipe runs.

use std::path::Path;

/// `…` in a command elides arguments: each part is looked for on its own.
const ELISION: char = '…';

pub(crate) fn exists(command: &str, root: &Path, justfile: &str, xtask: &clap::Command) -> bool {
    let mut words = command.split_whitespace();
    match words.next() {
        None => false,
        Some("just") => words.next().is_some_and(|name| is_recipe(justfile, name)),
        Some("xtask") => is_command(xtask, words),
        Some(program) if program.contains('/') => root.join(program).is_file(),
        Some(_) => command
            .split(ELISION)
            .all(|part| justfile.contains(part.trim())),
    }
}

/// Whether a line of `justfile` starts the recipe `name`, or aliases it.
fn is_recipe(justfile: &str, name: &str) -> bool {
    justfile.lines().any(|line| {
        if let Some(alias) = line.strip_prefix("alias ") {
            return alias.split_whitespace().next() == Some(name);
        }
        let start = line.strip_prefix('@').unwrap_or(line);
        start.strip_prefix(name).is_some_and(|rest| {
            rest.starts_with([':', ' ']) && rest.contains(':') && !rest.contains(":=")
        })
    })
}

/// Whether `words` walk `xtask`'s commands: each word a subcommand while there
/// are any, each `--flag` one its command takes. A word past the last
/// subcommand is that command's value.
fn is_command<'a>(xtask: &clap::Command, words: impl Iterator<Item = &'a str>) -> bool {
    let mut command = xtask;
    for word in words {
        if let Some(flag) = word.strip_prefix("--") {
            let flag = flag.split('=').next().unwrap_or(flag);
            if !command
                .get_arguments()
                .any(|argument| argument.get_long() == Some(flag))
            {
                return false;
            }
        } else if command.has_subcommands() {
            match command.find_subcommand(word) {
                Some(subcommand) => command = subcommand,
                None => return false,
            }
        } else {
            return true;
        }
    }
    !command.has_subcommands()
}
