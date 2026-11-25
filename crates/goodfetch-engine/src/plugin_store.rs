//! The Plugin store (ADR 0033, spec #596): the installed Plugins' unpacked
//! bundles in Goodfetch home, which both Products share, one folder per
//! Plugin named by its name and, in it, one folder named by its version
//! that holds the bundle, its manifest at its root:
//!
//! ```text
//! <home>/plugins/<name>/<version>/goodfetch-plugin.json
//! ```
//!
//! This is the store's reader: it resolves a Plugin's name to its bundle
//! and its manifest, read as any decoded input (ADR 0127). Adding, updating
//! and removing a Plugin (issues 889 and 928), the lockfile's digest and
//! the approvals in Operator policy (issue 900) are theirs; until then a
//! Plugin in the store is one the user put there.
//!
//! It reads the file system, so it blocks: call it from the tasks module's
//! blocking policy, never on an async worker.
#![expect(
    clippy::disallowed_methods,
    reason = "the store is read on the tasks module's blocking policy, never on an async worker"
)]

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, Rule};
use goodfetch_protocol::plugin::{
    FILE_NAME, MANIFEST_VERSION, ManifestError, PluginKindName, PluginManifest,
};

/// The Plugin store at one folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginStore {
    folder: PathBuf,
}

/// An installed Plugin: its unpacked bundle and the manifest at its root.
#[derive(Clone, Debug)]
pub struct Installed {
    /// The bundle's folder, `<name>/<version>` in the store.
    pub bundle: PathBuf,
    /// Its manifest, read and held to what its kinds allow.
    pub manifest: PluginManifest,
}

/// Why a name resolves to no Plugin of the store. Its text names the
/// Plugin, never a path of the user's.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The name would name another folder than one of the store's.
    #[error("`{0}` is no Plugin's name: it holds a path")]
    Name(String),
    /// No Plugin of the name is installed.
    #[error("no Plugin `{0}` is installed")]
    NotInstalled(String),
    /// The Plugin's folder holds no version, or more than one.
    #[error("the Plugin `{name}` has {versions} versions installed, where the store holds one")]
    Versions {
        /// The Plugin's name.
        name: String,
        /// How many versions its folder holds.
        versions: usize,
    },
    /// Its folder or its manifest cannot be read.
    #[error("the Plugin `{name}`'s files cannot be read: {source}")]
    Unreadable {
        /// The Plugin's name.
        name: String,
        /// The OS's error.
        source: io::Error,
    },
    /// Its manifest does not read, or declares what its kinds refuse.
    #[error("the Plugin `{name}`'s manifest: {source}")]
    Manifest {
        /// The Plugin's name.
        name: String,
        /// Why.
        source: ManifestError,
    },
    /// Its manifest is of a version this engine does not read.
    #[error(
        "the Plugin `{name}`'s manifest is of version {found}, which this engine, reading {MANIFEST_VERSION}, does not read ({compatibility:?})"
    )]
    ManifestVersion {
        /// The Plugin's name.
        name: String,
        /// The version its manifest is of.
        found: ProtocolVersion,
        /// How it stands against this engine's reader.
        compatibility: Compatibility,
    },
    /// Its manifest names another Plugin than its folder does.
    #[error("the manifest in the Plugin `{name}`'s folder names the Plugin `{named}`")]
    Misnamed {
        /// The folder's name.
        name: String,
        /// The name its manifest gives.
        named: String,
    },
    /// It is installed, and implements no kind the caller asked for.
    #[error("the Plugin `{name}` is no {kind} Plugin")]
    Kind {
        /// The Plugin's name.
        name: String,
        /// The kind asked for.
        kind: PluginKindName,
    },
}

impl PluginStore {
    /// The store at `folder`, the Goodfetch home's `plugins`.
    #[must_use]
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// The installed Plugin `name` of the kind `kind`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when `name` holds a path, no Plugin of the name is
    /// installed, its folder holds other than one version, its manifest
    /// cannot be read, is of a version this engine does not read or names
    /// another Plugin, or the Plugin is not of the kind `kind`.
    pub fn resolve(&self, name: &str, kind: &PluginKindName) -> Result<Installed, StoreError> {
        let unreadable = |source| StoreError::Unreadable {
            name: name.to_owned(),
            source,
        };
        if !is_one_folder(name) {
            return Err(StoreError::Name(name.to_owned()));
        }
        let folder = self.folder.join(name);
        let versions = match fs::read_dir(&folder) {
            Ok(entries) => entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(unreadable)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::NotInstalled(name.to_owned()));
            }
            Err(error) => return Err(unreadable(error)),
        };
        let [bundle] =
            <[PathBuf; 1]>::try_from(versions).map_err(|versions| StoreError::Versions {
                name: name.to_owned(),
                versions: versions.len(),
            })?;
        let manifest = read_manifest(name, &bundle)?;
        if manifest.name != name {
            return Err(StoreError::Misnamed {
                name: name.to_owned(),
                named: manifest.name,
            });
        }
        if !manifest.kinds.contains(kind) {
            return Err(StoreError::Kind {
                name: name.to_owned(),
                kind: kind.clone(),
            });
        }
        Ok(Installed { bundle, manifest })
    }
}

/// The manifest at the root of the Plugin `name`'s bundle `bundle`.
fn read_manifest(name: &str, bundle: &Path) -> Result<PluginManifest, StoreError> {
    let json = fs::read(bundle.join(FILE_NAME)).map_err(|source| StoreError::Unreadable {
        name: name.to_owned(),
        source,
    })?;
    let manifest_error = |source| StoreError::Manifest {
        name: name.to_owned(),
        source,
    };
    let found = PluginManifest::version_of(&json).map_err(manifest_error)?;
    match Rule::Default.compatibility(MANIFEST_VERSION, found) {
        Compatibility::Compatible => PluginManifest::read(&json).map_err(manifest_error),
        compatibility => Err(StoreError::ManifestVersion {
            name: name.to_owned(),
            found,
            compatibility,
        }),
    }
}

/// Whether `name` names one folder in the store's: one normal path
/// component.
fn is_one_folder(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) && !name.contains(['/', '\\'])
}

#[cfg(test)]
mod tests;
