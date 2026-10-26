//! 1Password's provider, `op://vault/item/field` and
//! `op://vault/item/section/field`: `op read --no-newline <reference>`,
//! signed in by `OP_SERVICE_ACCOUNT_TOKEN` or the desktop app's
//! integration in the engine's environment. A translation point
//! (ADR 0127): [`translate`] reads what `op` writes on its standard error.

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_engine::secrets::{ConfigProblem, ProviderFailure, Resolving, SecretProvider};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::SECRET_PROVIDERS_OP_COMMAND;

use crate::cli::{self, Answer, Cli};

/// What 1Password calls its CLI.
const OP: &str = "op";

/// 1Password's provider over its CLI.
#[derive(Clone, Debug)]
pub struct OnePassword {
    cli: Cli,
}

impl OnePassword {
    /// The provider `secret_providers.op.command` configures in `resolved`,
    /// running `op` with the engine's `environment`.
    pub fn configured(resolved: &Resolved, environment: BTreeMap<OsString, OsString>) -> Self {
        Self {
            cli: Cli::configured(&SECRET_PROVIDERS_OP_COMMAND, OP, resolved, environment),
        }
    }

    /// The provider running the `op` CLI at `command`, as
    /// `secret_providers.op.command` names it, with `environment`.
    pub fn new(command: Option<String>, environment: BTreeMap<OsString, OsString>) -> Self {
        Self {
            cli: Cli::new(&SECRET_PROVIDERS_OP_COMMAND, OP, command, environment),
        }
    }
}

impl SecretProvider for OnePassword {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            match self
                .cli
                .run(&["read", "--no-newline", reference.as_str()], &[])
                .await?
            {
                Answer::Value(stdout) => cli::value(&stdout, OP),
                Answer::Failed { stderr, code } => Err(translate(stderr, code)),
            }
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        self.cli.check().map(|_| ())
    }
}

/// What `op` writes for each failure it is known to answer, matched
/// without regard to case, and what it means.
const KNOWN: &[(&[&str], Meaning)] = &[
    (
        &[
            "isn't an item",
            "isn't a vault",
            "does not have a field",
            "isn't a field",
        ],
        Meaning::Missing,
    ),
    (&["more than one item matches"], Meaning::Ambiguous),
    (
        &[
            "not currently signed in",
            "is not signed in",
            "unauthorized",
        ],
        Meaning::SignedOut,
    ),
    (
        &[
            "dial tcp",
            "no such host",
            "connection refused",
            "i/o timeout",
            "network is unreachable",
        ],
        Meaning::Unreachable,
    ),
];

/// A known failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Missing,
    Ambiguous,
    SignedOut,
    Unreachable,
}

/// What `op` meant by failing with `stderr` and exit `code`.
pub(crate) fn translate(stderr: String, code: Option<i32>) -> ProviderFailure {
    let lower = stderr.to_lowercase();
    let known = KNOWN
        .iter()
        .find(|(texts, _)| texts.iter().any(|text| lower.contains(text)))
        .map(|(_, meaning)| *meaning);
    let refused = |fix: &str| ProviderFailure::Refused {
        fix: fix.to_owned(),
    };
    match known {
        Some(Meaning::Missing) => ProviderFailure::Missing,
        Some(Meaning::Ambiguous) => refused(
            "Several 1Password items match this reference; name the vault and the item by their IDs.",
        ),
        Some(Meaning::SignedOut) => refused(
            "The `op` CLI is not signed in: set `OP_SERVICE_ACCOUNT_TOKEN` in the engine's environment or turn on the 1Password app's CLI integration.",
        ),
        Some(Meaning::Unreachable) => refused(
            "The `op` CLI cannot reach 1Password from this host; check its network and proxy, then try again.",
        ),
        None => cli::unrecognized(OP, stderr, code),
    }
}

#[cfg(test)]
mod tests;
