//! The environment map's strict reading (ADRs 0059 and 0083): `main` reads
//! the process environment once and passes the map; the `GOODFETCH_`
//! prefix is reserved, so an unknown variable is `setting_unknown`, naming
//! the closest deployment variable, and an empty one, or one the engine
//! cannot parse, `setting_invalid`. The environment carries no setting.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::net::SocketAddr;

use goodfetch_protocol::settings::{
    CHANNEL_VARIABLE, CHANNELS, DAEMON_ADDRESS_VARIABLE, DAEMON_DRAIN_GRACE_VARIABLE,
    DEPLOYMENT_PREFIX, DEPLOYMENT_VARIABLES, OPS_ADDRESS_VARIABLE,
};

use super::error::SettingsError;
use super::keys;

/// Checks every `GOODFETCH_` variable of `environment` against the closed
/// list of deployment variables.
///
/// # Errors
///
/// `setting_unknown` for a variable not on the list, `setting_invalid` for
/// an empty value, one that is not Unicode, a `GOODFETCH_CHANNEL` that
/// names no channel, a `GOODFETCH_DAEMON_DRAIN_GRACE` that is no whole
/// number of milliseconds or a `GOODFETCH_DAEMON_ADDRESS` or
/// `GOODFETCH_OPS_ADDRESS` that is no IP address and port, the first by
/// name.
pub fn check_environment(environment: &BTreeMap<OsString, OsString>) -> Result<(), SettingsError> {
    for (name, value) in environment {
        let Some(name) = name
            .to_str()
            .filter(|name| name.starts_with(DEPLOYMENT_PREFIX))
        else {
            continue;
        };
        if !DEPLOYMENT_VARIABLES.contains(&name) {
            return Err(SettingsError::Unknown {
                key: name.to_owned(),
                layer: None,
                closest: keys::nearest(name, DEPLOYMENT_VARIABLES.iter().copied())
                    .map(str::to_owned),
            });
        }
        let reason = match value.to_str() {
            _ if value.is_empty() => Some("it is empty; unset it instead".to_owned()),
            None => Some("it is not Unicode".to_owned()),
            Some(value) => unparsable(name, value),
        };
        if let Some(reason) = reason {
            return Err(SettingsError::Invalid {
                key: name.to_owned(),
                layer: None,
                reason,
            });
        }
    }
    Ok(())
}

/// Why `value` of the deployment variable `name` cannot be read as the
/// engine reads it later, the channel the executable came from, the
/// foreground Daemon's drain grace or one of its two addresses, or `None`;
/// each SDK's check is its twin.
fn unparsable(name: &str, value: &str) -> Option<String> {
    match name {
        CHANNEL_VARIABLE if !CHANNELS.contains(&value) => Some(format!(
            "`{value}` is no channel; one of {}",
            CHANNELS.join(", ")
        )),
        DAEMON_DRAIN_GRACE_VARIABLE if value.parse::<u64>().is_err() => {
            Some(format!("`{value}` is no whole number of milliseconds"))
        }
        DAEMON_ADDRESS_VARIABLE | OPS_ADDRESS_VARIABLE if value.parse::<SocketAddr>().is_err() => {
            Some(format!("`{value}` is no `host:port`"))
        }
        _ => None,
    }
}

/// What the deployment variable `name` takes, as the `fix` of its refusal
/// names it.
#[must_use]
pub fn taken_value(name: &str) -> String {
    match name {
        CHANNEL_VARIABLE => format!("one of {}", CHANNELS.join(", ")),
        DAEMON_DRAIN_GRACE_VARIABLE => "a whole number of milliseconds".to_owned(),
        DAEMON_ADDRESS_VARIABLE => "an IP address and port such as 127.0.0.1:7900".to_owned(),
        OPS_ADDRESS_VARIABLE => "an IP address and port such as 0.0.0.0:9464".to_owned(),
        _ => "a non-empty Unicode value".to_owned(),
    }
}

/// The value of the deployment variable `name`, when it is set.
pub fn deployment_variable<'a>(
    environment: &'a BTreeMap<OsString, OsString>,
    name: &str,
) -> Option<&'a OsStr> {
    environment
        .get(OsStr::new(name))
        .map(OsString::as_os_str)
        .filter(|value| !value.is_empty())
}
