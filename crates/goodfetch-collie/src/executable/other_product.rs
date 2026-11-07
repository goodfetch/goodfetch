//! What only the other Product carries, on the command line (ADRs 0069,
//! 0087 and 0090): a command line its executable cannot read that names an
//! operation only Foxhound carries (`crawl start`, `run add-seed`), a command
//! only Foxhound's executable has, or a namespace of either that this
//! executable lacks (`crawl`), is refused with `not_in_product`, whose `fix`
//! names the install command of the channel the executable came from, and
//! exits with that code's row, `10`, rather than as a usage error. The names
//! come from the generated table of Foxhound's names, so this module names
//! no Foxhound type.

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Command;
use goodfetch_engine::{Identity, Requested};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::cli::{CommandError, CommandFailed};
use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::foxhound_names;
use serde_json::Value;

use super::exit_codes;
use super::output::{self, Human};

/// The flag whose value follows it, which is no word of the command.
const COLOR: &str = "--color";

/// The flag that prints the result as JSON.
const JSON: &str = "--json";

/// What only Foxhound carries that a command line names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Named {
    /// Its name as the catalog or the table writes it: an operation's
    /// (`crawl.start`), a command's path (`crawl.serve`) or a namespace's
    /// (`crawl_request`).
    pub(super) name: String,
    /// Whether it is an operation, which the failure names.
    pub(super) operation: bool,
}

/// What only Foxhound carries that `arguments`, a command line `command`
/// cannot read, names, if anything does.
pub(super) fn foxhound_only(command: &Command, arguments: &[OsString]) -> Option<Named> {
    let words = words(arguments);
    let (first, second) = (words.first()?, words.get(1));
    let operation = foxhound_names::OPERATIONS.iter().find(|name| {
        let (namespace, member) = name.split_once('.').unwrap_or((name, ""));
        spelled(namespace) == *first && second.is_some_and(|word| spelled(member) == *word)
    });
    if let Some(name) = operation {
        return Some(Named {
            name: (*name).to_owned(),
            operation: true,
        });
    }
    let path = foxhound_names::COMMANDS.iter().find(|path| {
        let parts: Vec<&str> = path.split('.').collect();
        words.len() >= parts.len() && parts.iter().zip(&words).all(|(part, word)| part == word)
    });
    if let Some(path) = path {
        return Some(Named {
            name: (*path).to_owned(),
            operation: false,
        });
    }
    if command.find_subcommand(first).is_some() {
        return None;
    }
    let namespace = foxhound_names::OPERATIONS
        .iter()
        .filter_map(|name| name.split_once('.').map(|(namespace, _)| namespace))
        .chain(
            foxhound_names::COMMANDS
                .iter()
                .filter_map(|path| path.split('.').next()),
        )
        .find(|namespace| spelled(namespace) == *first)?;
    Some(Named {
        name: namespace.to_owned(),
        operation: false,
    })
}

/// Refuses `named` on the engine `identity` runs as, with `not_in_product`,
/// printed as the command line asks, `arguments` holding `--json` or not,
/// and answers its exit code.
pub(super) fn refuse(
    identity: &Identity,
    named: &Named,
    arguments: &[OsString],
    human: &Human,
) -> ExitCode {
    let (message, details) =
        identity.not_in_product(Requested::Name(&named.name), Product::Foxhound, false);
    let details = match serde_json::to_value(details) {
        Ok(Value::Object(details)) => details,
        _ => serde_json::Map::new(),
    };
    let failed = CommandFailed {
        error: CommandError {
            code: NOT_IN_PRODUCT.name.to_owned(),
            group: NOT_IN_PRODUCT.group,
            retryable: NOT_IN_PRODUCT.retryable,
            message,
            operation: named.operation.then(|| named.name.clone()),
            details,
        },
    };
    let json = arguments.iter().skip(1).any(|argument| argument == JSON);
    output::failed(&failed, json, human);
    ExitCode::from(exit_codes::of_error(
        NOT_IN_PRODUCT.name,
        NOT_IN_PRODUCT.group,
    ))
}

/// The words of a command line, its executable and its flags left out.
fn words(arguments: &[OsString]) -> Vec<String> {
    let mut words = Vec::new();
    let mut arguments = arguments
        .iter()
        .skip(1)
        .filter_map(|argument| argument.to_str());
    while let Some(argument) = arguments.next() {
        if argument == COLOR {
            arguments.next();
        } else if !argument.starts_with('-') {
            words.push(argument.to_owned());
        }
    }
    words
}

/// The command line's spelling of a catalog word (ADR 0099).
fn spelled(word: &str) -> String {
    word.replace('_', "-")
}

#[cfg(test)]
mod tests;
