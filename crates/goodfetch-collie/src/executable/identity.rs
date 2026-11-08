//! What the executable runs as (ADRs 0087 and 0090): its Product, its
//! Product version and the channel it came from, whose install command
//! `not_in_product`'s `fix` names; and the refusal of a command line naming
//! what only Foxhound carries, before any engine starts.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::ExitCode;

use super::output::{self, ColourChoice, Human};
use super::{exit_codes, other_product};
use clap::ValueEnum;
use goodfetch_engine::CallError;
use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{SettingsError, check_environment, deployment_variable};
use goodfetch_engine::{Channel, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::jsonrpc::ErrorObject;
use goodfetch_protocol::settings::CHANNEL_VARIABLE;

use super::Start;

/// The channel a release build of the executable was packaged for, which
/// the release pipeline stamps (area 1); none for a build from source.
const BUILT_FOR_CHANNEL: Option<&str> = option_env!("GOODFETCH_CHANNEL");

/// Why the executable cannot say what it runs as.
pub(super) enum Refused {
    /// A deployment variable it cannot take, `GOODFETCH_CHANNEL` among them.
    Setting(SettingsError),
    /// Its version is no Product version.
    Version,
}

/// What the executable `start` describes runs as, in `environment`, every
/// `GOODFETCH_` variable of which must be a deployment variable (ADR 0083).
/// Its channel is the one its channel's launcher, wrapper or image names in
/// `GOODFETCH_CHANNEL`, else the one a release build was stamped with, else
/// crates.io: a build from source (ADRs 0087 and 0090).
pub(super) fn identity(
    start: &Start,
    environment: &BTreeMap<OsString, OsString>,
) -> Result<Identity, Refused> {
    check_environment(environment).map_err(Refused::Setting)?;
    let named = deployment_variable(environment, CHANNEL_VARIABLE)
        .map(|name| name.to_str().unwrap_or_default())
        .or(BUILT_FOR_CHANNEL);
    let channel = match named {
        None => Channel::CratesIo,
        Some(name) => Channel::named(name).ok_or_else(|| {
            let known: Vec<&str> = Channel::ALL.into_iter().map(Channel::name).collect();
            Refused::Setting(SettingsError::Invalid {
                key: CHANNEL_VARIABLE.to_owned(),
                layer: None,
                reason: format!("`{name}` is no channel; one of {}", known.join(", ")),
            })
        })?,
    };
    let product_version = ProductVersion::parse(start.version).map_err(|_| Refused::Version)?;
    Ok(Identity {
        product: start.product,
        product_version,
        channel,
    })
}

/// Refuses the command line of `start` that `command_line` could not read,
/// failing with `error`, when it names what only Foxhound carries and the
/// executable is not Foxhound's, before any engine starts (ADR 0087), and
/// answers the exit code; `None` when it names nothing of the kind.
pub(super) fn refuse(
    start: &Start,
    command_line: &clap::Command,
    error: &clap::Error,
) -> Option<ExitCode> {
    if error.kind() != clap::error::ErrorKind::InvalidSubcommand
        || start.product.contains(Product::Foxhound)
    {
        return None;
    }
    let named = other_product::foxhound_only(command_line, &start.arguments)?;
    Some(refuse_named(start, &named))
}

/// Refuses `named` on the executable `start` describes, coloured as its
/// command line's `--color` asks, which clap could not read.
fn refuse_named(start: &Start, named: &other_product::Named) -> ExitCode {
    let human = Human::new(colour(&start.arguments), &start.environment);
    match identity(start, &start.environment) {
        Ok(identity) => other_product::refuse(&identity, named, &start.arguments, &human),
        Err(Refused::Setting(error)) => refused_setting(start, &error, &human),
        Err(Refused::Version) => {
            human.failure("the executable's version is no Product version", &[]);
            ExitCode::FAILURE
        }
    }
}

/// Fails the command line of `start` with the deployment variable `error`
/// refused, before any engine starts: printed as every command's failure
/// is, as JSON on stdout when the command line holds `--json`, and ended by
/// its Error code's row.
pub(super) fn refused_setting(start: &Start, error: &SettingsError, human: &Human) -> ExitCode {
    let json = start
        .arguments
        .iter()
        .skip(1)
        .any(|argument| argument == JSON);
    output::fail(failure_of(start, error), None, json, human)
}

/// The failure `error` is on the executable `start` describes, whose
/// channel is the one it was built for, since the variable that names
/// another may be the one refused.
fn failure_of(start: &Start, error: &SettingsError) -> ErrorObject {
    let Ok(product_version) = ProductVersion::parse(start.version) else {
        let message = error.to_string();
        return match goodfetch_protocol::entries::error_code(error.code()) {
            Some(definition) => {
                CallError::new(definition, message, &serde_json::Map::new()).to_error_object()
            }
            None => exit_codes::internal(message),
        };
    };
    let identity = Identity {
        product: start.product,
        product_version,
        channel: BUILT_FOR_CHANNEL
            .and_then(Channel::named)
            .unwrap_or(Channel::CratesIo),
    };
    local_settings_error(error, &identity).to_error_object()
}

/// The flag that prints the result, and a failure, as JSON.
const JSON: &str = "--json";

/// The colour a command line, `arguments`, asks for with `--color <when>` or
/// `--color=<when>`, read without clap, which refused the command line or
/// has not read it yet; the default when it asks for none it knows.
pub(super) fn colour(arguments: &[OsString]) -> ColourChoice {
    let mut asked = None;
    let mut arguments = arguments
        .iter()
        .skip(1)
        .filter_map(|argument| argument.to_str());
    while let Some(argument) = arguments.next() {
        let value = match argument.strip_prefix("--color") {
            Some("") => arguments.next(),
            Some(value) => value.strip_prefix('='),
            None => None,
        };
        if let Some(choice) = value.and_then(|value| ColourChoice::from_str(value, true).ok()) {
            asked = Some(choice);
        }
    }
    asked.unwrap_or_default()
}
