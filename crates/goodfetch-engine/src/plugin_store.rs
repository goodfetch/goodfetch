//! The Plugin store (ADR 0033, spec #596): the installed Plugins' unpacked
//! bundles in Goodfetch home, which both Products share, one folder per
//! Plugin named by its name and, in it, one folder named by its version
//! that holds the bundle, its manifest at its root:
//!
//! ```text
//! <home>/plugins/<name>/<version>/goodfetch-plugin.json
//! ```
//!
//! It resolves a Plugin's name to its bundle and its manifest, read as any
//! decoded input (ADR 0127), lists the settings schemas the installed
//! Plugins declare, which `config.explain` answers, and adds a Plugin
//! `plugin add` unpacked into a staging folder beside it, moved into place
//! whole (issue 889). Updating and removing a Plugin (issue 928), the
//! lockfile's digest and the approvals in Operator policy (issue 900) are
//! theirs.
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

use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, Pinner};
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, Rule};
use goodfetch_protocol::plugin::{
    FILE_NAME, MANIFEST_VERSION, ManifestError, PluginKindName, PluginManifest, Runtime,
};
use goodfetch_protocol::settings::PluginSettingsSchema;

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
    /// The digest of the image index `plugin add` added it from,
    /// `sha256:<hex>`, recorded beside the bundle as
    /// `<name>/<version>.digest`; none for a Plugin placed in the store
    /// otherwise, which an Operator policy's pin refuses.
    pub digest: Option<String>,
}

/// The file beside the bundle `bundle` that records its digest.
fn digest_file(bundle: &Path) -> PathBuf {
    let mut file = bundle.as_os_str().to_owned();
    file.push(".digest");
    PathBuf::from(file)
}

/// Whether `entry`, of a Plugin's folder, records a bundle's digest rather
/// than holding a version.
fn is_digest_file(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_file())
        && entry.path().extension().is_some_and(|it| it == "digest")
}

/// The digest recorded beside `bundle`, none when nothing records it.
fn recorded_digest(name: &str, bundle: &Path) -> Result<Option<String>, StoreError> {
    match fs::read_to_string(digest_file(bundle)) {
        Ok(digest) => Ok(Some(digest.trim().to_owned())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StoreError::Unreadable {
            name: name.to_owned(),
            source,
        }),
    }
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
    /// Its manifest's version would name another folder than one of the
    /// Plugin's.
    #[error("the Plugin `{name}`'s version `{version}` holds a path")]
    Version {
        /// The Plugin's name.
        name: String,
        /// The version its manifest gives.
        version: String,
    },
    /// A version of the Plugin is installed already, which `plugin update`
    /// replaces (issue 928).
    #[error("the Plugin `{name}` is installed already, at version {version}")]
    Installed {
        /// The Plugin's name.
        name: String,
        /// The version installed.
        version: String,
    },
}

impl PluginStore {
    /// The store at `folder`, the Goodfetch home's `plugins`.
    #[must_use]
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// Its folder.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
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
        let installed = self.installed(name)?;
        if !installed.manifest.kinds.contains(kind) {
            return Err(StoreError::Kind {
                name: name.to_owned(),
                kind: *kind,
            });
        }
        Ok(installed)
    }

    /// Every installed Plugin in the store that resolves cleanly, sorted by
    /// name. A Plugin that does not resolve is skipped with a `warn`; no
    /// store yields an empty list.
    ///
    /// # Errors
    ///
    /// The OS's error when the store's folder cannot be listed.
    pub fn installed_all(&self) -> io::Result<Vec<Installed>> {
        let entries = match fs::read_dir(&self.folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut names = entries
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names
            .into_iter()
            .filter_map(|name| match self.installed(&name) {
                Ok(installed) => Some(installed),
                Err(error) => {
                    tracing::warn!(%error, "an installed Plugin cannot be read");
                    None
                }
            })
            .collect())
    }

    /// Each installed Plugin whose manifest declares a `settings_schema`,
    /// by name, with its version and that schema. A Plugin that does not
    /// resolve is left out, why logged at `warn`; no store, none.
    ///
    /// # Errors
    ///
    /// The OS's error when the store's folder cannot be listed.
    pub fn settings_schemas(&self) -> io::Result<Vec<PluginSettingsSchema>> {
        Ok(self
            .installed_all()?
            .into_iter()
            .filter_map(|Installed { manifest, .. }| {
                manifest.settings_schema.map(|schema| PluginSettingsSchema {
                    name: manifest.name,
                    version: manifest.version,
                    schema,
                })
            })
            .collect())
    }

    /// Every Plugin runtime artifact declared by any installed Plugin in
    /// the store for `platform`, for cache clean-up pinning (ADRs 0033 and
    /// 0098).
    ///
    /// # Errors
    ///
    /// The OS's error when the store's folder cannot be listed.
    pub fn declared_runtimes(&self, platform: &str) -> io::Result<Vec<ArtifactId>> {
        let mut ids: Vec<ArtifactId> = self
            .installed_all()?
            .into_iter()
            .filter_map(|installed| runtime_artifact_id(&installed.manifest.runtime, platform))
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Every installed Plugin whose manifest declares `runtime`, as
    /// [`Pinner::Plugin`]s sorted by name and version.
    pub fn pinners_for_runtime(&self, runtime: &Runtime) -> Vec<Pinner> {
        if *runtime == Runtime::None {
            return Vec::new();
        }
        self.installed_all()
            .unwrap_or_default()
            .into_iter()
            .filter(|installed| installed.manifest.runtime == *runtime)
            .map(|installed| Pinner::Plugin {
                name: installed.manifest.name,
                version: installed.manifest.version,
            })
            .collect()
    }

    /// A new, empty folder beside the store's, on its file system, for a
    /// bundle to be unpacked into and then [`add`](Self::add)ed; it is
    /// removed with all it still holds when dropped. Being outside the
    /// store's folder, it is never read as a Plugin.
    ///
    /// # Errors
    ///
    /// The OS's error when the folder cannot be made.
    pub fn staging(&self) -> io::Result<tempfile::TempDir> {
        let beside = self.folder.parent().unwrap_or(&self.folder);
        fs::create_dir_all(beside)?;
        tempfile::Builder::new()
            .prefix(".plugin-staging-")
            .tempdir_in(beside)
    }

    /// Adds the Plugin `name` whose bundle is unpacked at `staged`, a
    /// folder of [`staging`](Self::staging), from the image index of
    /// `digest`: its manifest is read, must name `name` and a version that
    /// names one folder, the digest is recorded beside where the bundle
    /// goes, and the folder is moved whole to `<name>/<version>`, so the
    /// store never holds part of a bundle.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when `name` or the version holds a path, the manifest
    /// cannot be read, is of a version this engine does not read or names
    /// another Plugin, a version of the Plugin is installed already, or the
    /// folder cannot be moved.
    pub fn add(&self, name: &str, staged: &Path, digest: &str) -> Result<Installed, StoreError> {
        if !is_one_folder(name) {
            return Err(StoreError::Name(name.to_owned()));
        }
        let manifest = read_manifest(name, staged)?;
        if manifest.name != name {
            return Err(StoreError::Misnamed {
                name: name.to_owned(),
                named: manifest.name,
            });
        }
        if !is_one_folder(&manifest.version) {
            return Err(StoreError::Version {
                name: name.to_owned(),
                version: manifest.version,
            });
        }
        match self.installed(name) {
            Ok(installed) => {
                return Err(StoreError::Installed {
                    name: name.to_owned(),
                    version: installed.manifest.version,
                });
            }
            Err(StoreError::NotInstalled(_) | StoreError::Versions { versions: 0, .. }) => {}
            Err(error) => return Err(error),
        }
        let unreadable = |source| StoreError::Unreadable {
            name: name.to_owned(),
            source,
        };
        let folder = self.folder.join(name);
        fs::create_dir_all(&folder).map_err(unreadable)?;
        let bundle = folder.join(&manifest.version);
        let recorded = digest_file(&bundle);
        fs::write(&recorded, digest).map_err(unreadable)?;
        if let Err(error) = fs::rename(staged, &bundle) {
            if let Err(left) = fs::remove_file(&recorded) {
                tracing::warn!(error = %left, "a Plugin's digest record outlived its failed add");
            }
            return Err(unreadable(error));
        }
        Ok(Installed {
            bundle,
            manifest,
            digest: Some(digest.to_owned()),
        })
    }

    /// Removes the installed Plugin `name` from the store, returning `true`
    /// if a folder was deleted and `false` if it was not installed
    /// (ADR 0033, issue 928).
    ///
    /// # Errors
    ///
    /// [`StoreError::Name`] when `name` holds a path, or
    /// [`StoreError::Unreadable`] if the folder cannot be removed.
    pub fn remove(&self, name: &str) -> Result<bool, StoreError> {
        if !is_one_folder(name) {
            return Err(StoreError::Name(name.to_owned()));
        }
        let folder = self.folder.join(name);
        match fs::remove_dir_all(&folder) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(StoreError::Unreadable {
                name: name.to_owned(),
                source,
            }),
        }
    }

    /// The installed Plugin `name`, of any kind.
    pub(crate) fn installed(&self, name: &str) -> Result<Installed, StoreError> {
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
                .filter_map(|entry| match entry {
                    Ok(entry) if is_digest_file(&entry) => None,
                    entry => Some(entry.map(|entry| entry.path())),
                })
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
        let digest = recorded_digest(name, &bundle)?;
        Ok(Installed {
            bundle,
            manifest,
            digest,
        })
    }
}

/// The [`ArtifactId`] for `runtime` on `platform`, or `None` for
/// [`Runtime::None`].
pub fn runtime_artifact_id(runtime: &Runtime, platform: &str) -> Option<ArtifactId> {
    let (name, version) = match runtime {
        Runtime::None => return None,
        Runtime::Node { version } => ("node", version),
        Runtime::Python { version } => ("python", version),
    };
    Some(ArtifactId {
        kind: ArtifactKind::PluginRuntime,
        name: name.to_owned(),
        build: format!("{version}-{platform}"),
    })
}

/// The manifest at the root of the Plugin `name`'s bundle `bundle`.
pub(crate) fn read_manifest(name: &str, bundle: &Path) -> Result<PluginManifest, StoreError> {
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
