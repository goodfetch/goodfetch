//! The environment as a Secret provider (ADRs 0015 and 0092): `env:NAME`
//! is the variable `NAME` of the engine's environment map, which `main`
//! reads once (ADR 0083); for an Attached engine that is the program's own
//! environment, for a Daemon its own.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

use goodfetch_protocol::secret_reference::SecretReference;
use secrecy::SecretString;

use super::{ConfigProblem, ProviderFailure, Resolving, SecretProvider};

/// The `env:` provider over one environment map.
pub(super) struct Environment {
    variables: BTreeMap<OsString, OsString>,
}

impl Environment {
    pub(super) fn new(variables: BTreeMap<OsString, OsString>) -> Self {
        Self { variables }
    }
}

impl SecretProvider for Environment {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        let value = match self.variables.get(OsStr::new(reference.rest())) {
            None => Err(ProviderFailure::Missing),
            Some(value) if value.is_empty() => Err(ProviderFailure::Missing),
            Some(value) => value
                .to_str()
                .map(|value| SecretString::from(value.to_owned()))
                .ok_or_else(|| ProviderFailure::Refused {
                    fix: format!(
                        "The variable `{}` holds no UTF-8 text; set it to the value as text.",
                        reference.rest()
                    ),
                }),
        };
        Box::pin(std::future::ready(value))
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}
