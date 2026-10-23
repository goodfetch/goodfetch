//! The environment map's strict reading (ADRs 0059 and 0083): `main` reads
//! the process environment once and passes the map; the `GOODFETCH_`
//! prefix is reserved, so an unknown variable is `setting_unknown`, naming
//! the closest deployment variable, and an empty one `setting_invalid`.
//! The environment carries no setting.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

use goodfetch_protocol::settings::{DEPLOYMENT_PREFIX, DEPLOYMENT_VARIABLES};

use super::error::SettingsError;
use super::keys;

/// Checks every `GOODFETCH_` variable of `environment` against the closed
/// list of deployment variables.
///
/// # Errors
///
/// `setting_unknown` for a variable not on the list, `setting_invalid` for
/// an empty value or one that is not Unicode, the first by name.
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
        let reason = if value.is_empty() {
            Some("it is empty; unset it instead")
        } else if value.to_str().is_none() {
            Some("it is not Unicode")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(SettingsError::Invalid {
                key: name.to_owned(),
                layer: None,
                reason: reason.to_owned(),
            });
        }
    }
    Ok(())
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
