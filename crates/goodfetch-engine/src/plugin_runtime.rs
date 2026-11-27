//! Plugin language runtimes (ADRs 0033, 0043, 0098 and 0115; issue 895):
//! resolves a manifest's declared [`Runtime`] (`node` or `python` and a
//! version) to its pinned download entry, fetches and verifies it into the
//! shared per-user cache at `plugin add`, and resolves it from the cache at
//! Plugin start without downloading when `egress.offline` is set.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use goodfetch_protocol::browser_cache::{ArtifactId, Pinner};
use goodfetch_protocol::plugin::Runtime;
use goodfetch_protocol::plugin::add::PLUGIN_ARTIFACT_REFUSED;
use goodfetch_protocol::settings::SettingsLayer;

use crate::browser_cache::{BrowserCache, HeldArtifact, PinnedRuntime};
use crate::call_error::CallError;
use crate::pinned_download::Fetching;
use crate::plugin_store::{PluginStore, runtime_artifact_id};
use crate::tasks::BlockingThreads;

/// The host platform identifier used for Plugin runtime builds, matching
/// the manifest's platform names (`linux-x64`, `linux-arm64`, `macos-arm64`,
/// `macos-x64`, `windows-x64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimePlatform {
    /// Linux x86-64.
    LinuxX64,
    /// Linux arm64.
    LinuxArm64,
    /// macOS Apple silicon.
    MacosArm64,
    /// macOS x86-64.
    MacosX64,
    /// Windows x86-64.
    WindowsX64,
}

impl RuntimePlatform {
    /// The platform this engine runs on.
    #[must_use]
    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                Self::MacosArm64
            } else {
                Self::MacosX64
            }
        } else if cfg!(target_os = "windows") {
            Self::WindowsX64
        } else if cfg!(target_arch = "aarch64") {
            Self::LinuxArm64
        } else {
            Self::LinuxX64
        }
    }

    /// Its canonical hyphenated name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxX64 => "linux-x64",
            Self::LinuxArm64 => "linux-arm64",
            Self::MacosArm64 => "macos-arm64",
            Self::MacosX64 => "macos-x64",
            Self::WindowsX64 => "windows-x64",
        }
    }
}

/// One pinned runtime entry in a [`RuntimeCatalog`]: where it downloads
/// from, its archive digest, and the relative path of its interpreter
/// executable inside the unpacked cache `files/` folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeEntry {
    /// Its pinned download metadata.
    pub pin: PinnedRuntime,
    /// Relative path to the interpreter executable inside `files/`.
    pub executable: String,
}

/// The catalog mapping `(runtime, platform)` to pinned runtime archives.
#[derive(Clone, Debug, Default)]
pub struct RuntimeCatalog {
    entries: BTreeMap<(String, String, RuntimePlatform), RuntimeEntry>,
}

impl RuntimeCatalog {
    /// An empty catalog.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The engine's built-in catalog of pinned runtimes. Tests inject their
    /// own entries pointing at a local stand-in host resolved via
    /// `network.resolve` (ADR 0059).
    #[must_use]
    pub fn built_in() -> Self {
        Self::default()
    }

    /// Inserts `entry` for `(runtime, platform)` into this catalog.
    #[must_use]
    pub fn with_entry(
        mut self,
        runtime: &Runtime,
        platform: RuntimePlatform,
        entry: RuntimeEntry,
    ) -> Self {
        if let Some((kind, version)) = runtime_key(runtime) {
            self.entries
                .insert((kind.to_owned(), version.to_owned(), platform), entry);
        }
        self
    }

    /// Looks up the pinned entry for `runtime` on `platform`.
    #[must_use]
    pub fn get(&self, runtime: &Runtime, platform: RuntimePlatform) -> Option<&RuntimeEntry> {
        let (kind, version) = runtime_key(runtime)?;
        self.entries
            .get(&(kind.to_owned(), version.to_owned(), platform))
    }
}

/// The `(kind, version)` key of `runtime`, or `None` for [`Runtime::None`].
fn runtime_key(runtime: &Runtime) -> Option<(&'static str, &str)> {
    match runtime {
        Runtime::None => None,
        Runtime::Node { version } => Some(("node", version.as_str())),
        Runtime::Python { version } => Some(("python", version.as_str())),
    }
}

/// The cache [`ArtifactId`] for `runtime` on `platform`, or `None` for
/// [`Runtime::None`].
#[must_use]
pub fn artifact_id(runtime: &Runtime, platform: RuntimePlatform) -> Option<ArtifactId> {
    runtime_artifact_id(runtime, platform.as_str())
}

/// Default relative path to the interpreter executable inside a cached
/// runtime's `files/` folder when no catalog entry overrides it.
#[must_use]
pub fn default_executable(runtime: &Runtime) -> Option<&'static str> {
    match runtime {
        Runtime::None => None,
        Runtime::Node { .. } => Some(if cfg!(windows) {
            "node.exe"
        } else {
            "bin/node"
        }),
        Runtime::Python { .. } => Some(if cfg!(windows) {
            "python.exe"
        } else {
            "bin/python3"
        }),
    }
}

/// The cache, runtime catalog and clock timestamp used to install a
/// Plugin's declared language runtime.
pub struct RuntimeTarget<'a> {
    /// The shared runtime cache.
    pub cache: &'a BrowserCache,
    /// The catalog mapping `(runtime, platform)` to pinned archive URL and digest.
    pub catalog: &'a RuntimeCatalog,
    /// Current timestamp for cache entry metadata.
    pub now: jiff::Timestamp,
}

/// Ensures the language runtime declared by a Plugin being added is cached
/// and digest-verified in `target.cache` (issue 895).
///
/// # Errors
///
/// `plugin_artifact_refused` when the manifest asks for a runtime version
/// or platform not in `target.catalog`, `artifact_unavailable_offline` when
/// missing under `egress.offline`, `artifact_download_failed` with its
/// `failure` detail when the download fails or its digest mismatches, or an
/// internal error on cache I/O failure.
pub async fn ensure_installed(
    runtime: &Runtime,
    adding: (&str, &str),
    store: &PluginStore,
    target: RuntimeTarget<'_>,
    fetching: &Fetching,
    blocking: &BlockingThreads,
) -> Result<Option<HeldArtifact>, CallError> {
    if *runtime == Runtime::None {
        return Ok(None);
    }
    let platform = RuntimePlatform::host();
    let Some(entry) = target.catalog.get(runtime, platform) else {
        let (kind, version) = runtime_key(runtime).unwrap_or(("unknown", ""));
        return Err(CallError::new(
            &PLUGIN_ARTIFACT_REFUSED,
            format!(
                "The Plugin `{}` declares runtime {kind} {version} on {}, which has no pinned runtime entry.",
                adding.0,
                platform.as_str(),
            ),
            &(),
        ));
    };
    let pinned_by = collect_pinners(store, runtime, Some(adding), blocking).await;
    target
        .cache
        .prepare_plugin_runtime(&entry.pin, &pinned_by, fetching, target.now)
        .await
        .map(|prepared| Some(prepared.artifact))
        .map_err(|error| error.to_call_error())
}

/// Resolves an already-cached runtime for starting a Plugin instance,
/// holding its shared cache lock. Under `offline`, a missing runtime fails
/// with `artifact_unavailable_offline` naming the artifact and every
/// installed Plugin that declares it, attempting no download.
///
/// # Errors
///
/// `artifact_unavailable_offline` when missing under `egress.offline`, or
/// an error when the cache cannot be read.
pub async fn resolve_for_start(
    runtime: &Runtime,
    store: &PluginStore,
    cache: &BrowserCache,
    catalog: &RuntimeCatalog,
    offline: Option<SettingsLayer>,
    blocking: &BlockingThreads,
    now: jiff::Timestamp,
) -> Result<Option<(HeldArtifact, PathBuf)>, CallError> {
    let platform = RuntimePlatform::host();
    let Some(id) = artifact_id(runtime, platform) else {
        return Ok(None);
    };
    let pinned_by = collect_pinners(store, runtime, None, blocking).await;
    let held = cache
        .held_plugin_runtime(&id, &pinned_by, offline, now)
        .await
        .map_err(|error| error.to_call_error())?;
    let Some(held) = held else {
        return Ok(None);
    };
    let executable = executable_path(&held.files(), runtime, catalog, platform);
    Ok(Some((held, executable)))
}

/// The absolute path to the interpreter executable inside `files`.
#[must_use]
pub fn executable_path(
    files: &Path,
    runtime: &Runtime,
    catalog: &RuntimeCatalog,
    platform: RuntimePlatform,
) -> PathBuf {
    let relative = catalog
        .get(runtime, platform)
        .map(|entry| entry.executable.as_str())
        .or_else(|| default_executable(runtime))
        .unwrap_or_default();
    files.join(relative)
}

/// Collects every installed Plugin pinning `runtime`, optionally including
/// one currently being added, sorted and deduplicated.
async fn collect_pinners(
    store: &PluginStore,
    runtime: &Runtime,
    adding: Option<(&str, &str)>,
    blocking: &BlockingThreads,
) -> Vec<Pinner> {
    let store = store.clone();
    let runtime = runtime.clone();
    let mut pinners = blocking
        .run(move || store.pinners_for_runtime(&runtime))
        .await;
    if let Some((name, version)) = adding {
        let pinner = Pinner::Plugin {
            name: name.to_owned(),
            version: version.to_owned(),
        };
        if !pinners.contains(&pinner) {
            pinners.push(pinner);
        }
    }
    pinners.sort_by(|left, right| pinner_key(left).cmp(&pinner_key(right)));
    pinners
}

fn pinner_key(pinner: &Pinner) -> (&str, &str) {
    match pinner {
        Pinner::Plugin { name, version } => (name.as_str(), version.as_str()),
        Pinner::Product { name, .. } => (name.name(), ""),
    }
}
