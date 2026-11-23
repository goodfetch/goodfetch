//! Bitwarden's and Vaultwarden's provider, `bw:item/field`: `bw get`
//! without interaction, unlocked by `BW_SESSION` in the engine's
//! environment, against whichever server `bw config server` set, a
//! Vaultwarden server included. `field` is one of the login's own fields,
//! read with `bw get <field>` (`totp` gives the current code, so the seed
//! never leaves the vault), or the name of a custom field, read from
//! `bw get item`. A translation point (ADR 0127): [`translate`] reads what
//! `bw` writes on its standard error.

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_engine::secrets::{ConfigProblem, ProviderFailure, Resolving, SecretProvider};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::SECRET_PROVIDERS_BW_COMMAND;
use secrecy::SecretString;

use crate::cli::{self, Answer, Cli};

/// What Bitwarden calls its CLI.
const BW: &str = "bw";

/// Bitwarden's and Vaultwarden's provider over the Bitwarden CLI.
#[derive(Clone, Debug)]
pub struct Bitwarden {
    cli: Cli,
}

impl Bitwarden {
    /// The provider `secret_providers.bw.command` configures in `resolved`,
    /// running `bw` with the engine's `environment`.
    pub fn configured(resolved: &Resolved, environment: BTreeMap<OsString, OsString>) -> Self {
        Self {
            cli: Cli::configured(&SECRET_PROVIDERS_BW_COMMAND, BW, resolved, environment),
        }
    }

    /// The provider running the `bw` CLI at `command`, as
    /// `secret_providers.bw.command` names it, with `environment`.
    pub fn new(command: Option<String>, environment: BTreeMap<OsString, OsString>) -> Self {
        Self {
            cli: Cli::new(&SECRET_PROVIDERS_BW_COMMAND, BW, command, environment),
        }
    }
}

impl SecretProvider for Bitwarden {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let (item, field) = reference
                .rest()
                .split_once('/')
                .expect("invariant: a `bw:` reference parses as item/field");
            let own = OWN_FIELDS.contains(&field);
            let object = if own { field } else { "item" };
            let stdout = match self
                .cli
                .run(
                    &["get", object, "--", item],
                    &[("BW_NOINTERACTION", "true")],
                )
                .await?
            {
                Answer::Value(stdout) => stdout,
                Answer::Failed { stderr, code } => return Err(translate(stderr, code)),
            };
            if own {
                cli::value(&stdout, BW)
            } else {
                custom_field(&stdout, field)
            }
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        self.cli.check().map(|_| ())
    }
}

/// The fields `bw get` reads itself; any other is a custom field's name.
const OWN_FIELDS: [&str; 5] = ["password", "username", "totp", "notes", "uri"];

/// The custom field `name` of the item `bw get item` wrote as `stdout`.
/// The item's JSON, which holds its other values too, is parsed from the
/// zeroized buffer; its parsed copy is dropped once the field is taken.
fn custom_field(
    stdout: &secrecy::zeroize::Zeroizing<Vec<u8>>,
    name: &str,
) -> Result<SecretString, ProviderFailure> {
    let item: serde_json::Value = serde_json::from_slice(stdout).map_err(|_| {
        ProviderFailure::Refused {
            fix: "The `bw` CLI answered `bw get item` with something other than an item; check that `secret_providers.bw.command` names the Bitwarden CLI.".to_owned(),
        }
    })?;
    item.get("fields")
        .and_then(serde_json::Value::as_array)
        .and_then(|fields| {
            fields
                .iter()
                .find(|field| field.get("name").and_then(serde_json::Value::as_str) == Some(name))
        })
        .and_then(|field| field.get("value"))
        .and_then(serde_json::Value::as_str)
        .map(|value| SecretString::from(value.to_owned()))
        .ok_or(ProviderFailure::Missing)
}

/// What `bw get` writes for each failure it is known to answer
/// (`bitwarden/clients`' `get.command.ts`, `base-program.ts` and
/// `response.ts`), matched without regard to case, and what it means.
const KNOWN: &[(&[&str], Meaning)] = &[
    (
        &[
            "not found.",
            "not a login.",
            "available for this login.",
            "no notes available",
        ],
        Meaning::Missing,
    ),
    (&["more than one result was found"], Meaning::Ambiguous),
    (&["you are not logged in"], Meaning::LoggedOut),
    (&["vault is locked"], Meaning::Locked),
    (
        &[
            "premium status is required",
            "restricted by organizational policy",
        ],
        Meaning::NotAllowed,
    ),
    (&["couldn't generate totp code"], Meaning::NoCode),
    (
        &[
            "fetcherror",
            "enotfound",
            "econnrefused",
            "etimedout",
            "eai_again",
        ],
        Meaning::Unreachable,
    ),
];

/// A known failure's meaning.
#[derive(Clone, Copy)]
enum Meaning {
    Missing,
    Ambiguous,
    LoggedOut,
    Locked,
    NotAllowed,
    NoCode,
    Unreachable,
}

/// What `bw` meant by failing with `stderr` and exit `code`.
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
            "Several Bitwarden items match this name; name the item by its ID in the reference.",
        ),
        Some(Meaning::LoggedOut) => refused(
            "The `bw` CLI is not logged in: run `bw login`, then `bw unlock`, and set `BW_SESSION` in the engine's environment.",
        ),
        Some(Meaning::Locked) => refused(
            "The Bitwarden vault is locked: run `bw unlock` and set the `BW_SESSION` it prints in the engine's environment.",
        ),
        Some(Meaning::NotAllowed) => refused(
            "The Bitwarden account or its organization's policy does not allow reading this field; ask its administrator.",
        ),
        Some(Meaning::NoCode) => ProviderFailure::NoCode {
            fix: "Bitwarden could not make a TOTP code from this item's seed; check the seed in the vault.".to_owned(),
        },
        Some(Meaning::Unreachable) => refused(
            "The `bw` CLI cannot reach its Bitwarden or Vaultwarden server from this host; check `bw config server`, the network and the proxy.",
        ),
        None => cli::unrecognized(BW, stderr, code),
    }
}

#[cfg(test)]
mod tests;
