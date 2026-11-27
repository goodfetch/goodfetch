//! The Plugin lockfile (ADR 0033), `goodfetch-plugins.lock`: TOML beside
//! the project's configuration, or in the Goodfetch home's configuration
//! folder, that pins each Plugin `plugin add` installed to the digest of
//! its multi-platform OCI image index, like `.terraform.lock.hcl`. The
//! engine reads its TOML as JSON, as it reads a Recipe manifest: it first
//! takes its version with [`PluginLockfile::version_of`] and holds it to the
//! `plugin-lockfile` format's rule (ADR 0066), then [`PluginLockfile::read`]
//! reads the rest.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::ProtocolVersion;

/// The lockfile's file name.
pub const LOCKFILE_NAME: &str = "goodfetch-plugins.lock";

/// The lockfile format's version, `major.minor` (ADRs 0030 and 0066).
pub const LOCKFILE_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A Plugin lockfile: its version, then one `[[plugin]]` table for each
/// Plugin it pins, by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PluginLockfile {
    /// The version of the lockfile format it is written in.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: ProtocolVersion,
    /// The Plugins it pins, one entry for each name, in name order.
    #[serde(default, rename = "plugin", skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugins: Vec<LockedPlugin>,
}

/// One Plugin the lockfile pins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LockedPlugin {
    /// The Plugin's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its version, as its manifest names it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: String,
    /// The digest of its multi-platform image index, `sha256:<hex>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// The identity that signed it, once the Plugin index records one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signer: Option<String>,
    /// The digest of the approvals given to its manifest, once given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approvals: Option<String>,
}

/// Why a lockfile is refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LockfileError {
    /// It is not a lockfile.
    #[error("it does not read: {0}")]
    Malformed(#[from] serde_json::Error),
    /// It pins one name twice.
    #[error("it pins `{0}` twice")]
    Duplicate(String),
}

/// A pin that differs from the lockfile's for the same name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the Plugin lockfile pins `{name}` to `{locked}`, not `{found}`")]
pub struct DigestMismatch {
    /// The Plugin's name.
    pub name: String,
    /// The digest the lockfile pins.
    pub locked: String,
    /// The digest given.
    pub found: String,
}

/// The part of a lockfile read before the rest: its version.
#[derive(Deserialize)]
struct Header {
    version: ProtocolVersion,
}

impl PluginLockfile {
    /// A lockfile that pins nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            version: LOCKFILE_VERSION,
            plugins: Vec::new(),
        }
    }

    /// The version of the lockfile whose document, its TOML as JSON, is
    /// `document`, read before the rest, which a newer minor may have added
    /// to (ADR 0066).
    ///
    /// # Errors
    ///
    /// When `document` is no table with a `major.minor` `version`.
    pub fn version_of(document: &Value) -> Result<ProtocolVersion, LockfileError> {
        let header = Header::deserialize(document)?;
        Ok(header.version)
    }

    /// Reads a lockfile's document, its TOML as JSON, whose version the
    /// reader has accepted ([`Self::version_of`]).
    ///
    /// # Errors
    ///
    /// When it does not read, or pins one name twice.
    pub fn read(document: Value) -> Result<Self, LockfileError> {
        let lockfile = Self::deserialize(document)?;
        let mut names: Vec<&str> = lockfile
            .plugins
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect();
        names.sort_unstable();
        if let Some(twice) = names.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(LockfileError::Duplicate(twice[0].to_owned()));
        }
        Ok(lockfile)
    }

    /// The entry that pins `name`, if one does.
    #[must_use]
    pub fn locked(&self, name: &str) -> Option<&LockedPlugin> {
        self.plugins.iter().find(|plugin| plugin.name == name)
    }

    /// Removes the entry pinning `name`, returning it if it was present.
    pub fn remove(&mut self, name: &str) -> Option<LockedPlugin> {
        let index = self.plugins.iter().position(|plugin| plugin.name == name)?;
        Some(self.plugins.remove(index))
    }

    /// Pins `plugin`, in name order, unless the lockfile already pins its
    /// name: to the same digest, which it keeps, or to another.
    ///
    /// # Errors
    ///
    /// [`DigestMismatch`] when the lockfile pins its name to another digest.
    pub fn lock(&mut self, plugin: LockedPlugin) -> Result<(), DigestMismatch> {
        match self.locked(&plugin.name) {
            Some(locked) if locked.digest == plugin.digest => Ok(()),
            Some(locked) => Err(DigestMismatch {
                locked: locked.digest.clone(),
                name: plugin.name,
                found: plugin.digest,
            }),
            None => {
                let at = self
                    .plugins
                    .partition_point(|locked| locked.name < plugin.name);
                self.plugins.insert(at, plugin);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;
