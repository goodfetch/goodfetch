//! Locating or downloading `plugin-shim` for process Plugins (ADRs 0033,
//! 0051, 0090 and 0115; issue 1077).
//!
//! A Product executable (`goodfetch-collie` or `goodfetch-foxhound`) carries
//! the hidden `plugin-shim` mode itself, so an Attached engine or Daemon runs
//! `std::env::current_exe()`. A linked Rust engine (`goodfetch-collie` or
//! `goodfetch-foxhound` crate) runs inside the user's program, which never
//! links V8 or `plugin-shim`: it finds the executable through the Product's
//! own engine-path environment variable (`GOODFETCH_COLLIE_ENGINE_PATH` for
//! Collie, `GOODFETCH_FOXHOUND_ENGINE_PATH` for Foxhound), or downloads the
//! pinned `EngineExecutable` into the shared Browser cache on first use,
//! verified by SHA-256 digest.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, PinnedBy};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::{COLLIE_ENGINE_PATH_VARIABLE, FOXHOUND_ENGINE_PATH_VARIABLE};

use crate::browser_cache::{BrowserCache, HeldArtifact, PinnedArtifact};
use crate::call_error::CallError;
use crate::pinned_download::{Digest, Fetching};
use crate::tasks::BlockingThreads;

/// Where Goodfetch release executables download from by default.
const RELEASES: &str = "https://github.com/goodfetch/goodfetch/releases/download";

/// The engine-path deployment variable that names `product`'s executable
/// (ADR 0090).
#[must_use]
pub const fn engine_path_variable(product: Product) -> &'static str {
    match product {
        Product::Collie => COLLIE_ENGINE_PATH_VARIABLE,
        Product::Foxhound => FOXHOUND_ENGINE_PATH_VARIABLE,
    }
}

/// The host platform segment used for pinned `EngineExecutable` artifacts.
#[must_use]
pub const fn host_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-arm64"
        } else {
            "macos-x64"
        }
    } else if cfg!(target_os = "windows") {
        "windows-x64"
    } else if cfg!(target_arch = "aarch64") {
        "linux-arm64"
    } else {
        "linux-x64"
    }
}

/// The default pinned [`PinnedArtifact`] for `product` at `product_version` on
/// this host platform. Until a release publishes executable digests,
/// `archive_digest` is `None` so an unpinned build is never downloaded without
/// verification, while `egress.offline` still refuses a missing executable
/// with `artifact_unavailable_offline` naming the artifact and what pins it.
#[must_use]
pub fn default_pin(product: Product, product_version: &ProductVersion) -> PinnedArtifact {
    let executable = product.executable_name();
    let platform = host_platform();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    PinnedArtifact {
        id: ArtifactId {
            kind: ArtifactKind::EngineExecutable,
            name: executable.to_owned(),
            build: format!("{product_version}-{platform}"),
        },
        platform: None,
        url: format!("{RELEASES}/v{product_version}/{executable}-{platform}{suffix}"),
        archive_digest: None,
        digest: Digest::of(&[]),
        application_settings: BTreeMap::new(),
    }
}

/// How an engine locates its `plugin-shim` launcher executable.
#[derive(Clone, Debug, Default)]
pub enum PluginShim {
    /// The engine runs inside a Product executable, which carries
    /// `plugin-shim` itself (`std::env::current_exe()`).
    #[default]
    CurrentExecutable,
    /// A linked Rust engine: resolves `plugin-shim` from the Product's
    /// engine-path variable or downloads the pinned `EngineExecutable` into
    /// the shared cache on first use.
    Linked(Box<LinkedShim>),
}

/// Configuration for a linked Rust engine's `plugin-shim` resolution.
#[derive(Clone, Debug)]
pub struct LinkedShim {
    /// Explicit executable path read from the Product's own engine-path
    /// environment variable, when set.
    pub env_path: Option<PathBuf>,
    /// The Product version pinning the engine executable.
    pub pinned_by: PinnedBy,
    /// The pinned `EngineExecutable` artifact downloaded on first use when
    /// `env_path` is absent.
    pub pin: PinnedArtifact,
}

/// A resolved `plugin-shim` launcher executable, keeping any shared cache lock
/// held while the Run uses it.
pub struct ResolvedShim {
    /// Absolute path to the executable supporting the hidden `plugin-shim`
    /// command.
    pub launcher: PathBuf,
    /// Shared cache lock held when the launcher comes from the Browser cache.
    pub held: Option<HeldArtifact>,
}

impl PluginShim {
    /// Constructs a linked-engine [`PluginShim`] for `product` at
    /// `product_version`, using `pin` or [`default_pin`] when `None`.
    #[must_use]
    pub fn linked(
        product: Product,
        product_version: ProductVersion,
        pin: Option<PinnedArtifact>,
    ) -> Self {
        let pin = pin.unwrap_or_else(|| default_pin(product, &product_version));
        Self::Linked(Box::new(LinkedShim {
            env_path: None,
            pinned_by: PinnedBy {
                product,
                product_version,
            },
            pin,
        }))
    }

    /// Reads the Product's own engine-path variable from `environment` into a
    /// [`Self::Linked`] shim if no explicit path is already set, ignoring the
    /// other Product's variable (ADR 0090).
    #[must_use]
    pub fn with_environment(mut self, environment: &BTreeMap<OsString, OsString>) -> Self {
        if let Self::Linked(ref mut linked) = self
            && linked.env_path.is_none()
        {
            let variable = engine_path_variable(linked.pinned_by.product);
            linked.env_path = environment
                .get(OsStr::new(variable))
                .filter(|value| !value.is_empty())
                .map(PathBuf::from);
        }
        self
    }

    /// Whether resolving this shim may access the shared cache or network
    /// (false for [`Self::CurrentExecutable`] or a linked shim with an
    /// explicit `env_path`).
    #[must_use]
    pub fn needs_cache(&self) -> bool {
        matches!(self, Self::Linked(linked) if linked.env_path.is_none())
    }

    /// Resolves the `plugin-shim` launcher executable.
    ///
    /// For [`Self::CurrentExecutable`], returns `std::env::current_exe()`.
    /// For [`Self::Linked`], returns `env_path` when set; otherwise prepares
    /// the pinned `EngineExecutable` in `cache`, downloading and checking its
    /// digest on first use.
    ///
    /// # Errors
    ///
    /// `artifact_unavailable_offline` when missing under `egress.offline`,
    /// `artifact_download_failed` when the download fails or its digest does
    /// not match, or `internal_error` when `current_exe()` or cache I/O fails.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves no partial executable in the cache.
    pub async fn resolve(
        &self,
        cache: &BrowserCache,
        fetching: Option<&Fetching>,
        now: jiff::Timestamp,
        blocking: &BlockingThreads,
    ) -> Result<ResolvedShim, CallError> {
        match self {
            Self::CurrentExecutable => {
                let launcher = blocking.run(std::env::current_exe).await.map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!("the engine's executable path cannot be read: {error}"),
                    )
                })?;
                Ok(ResolvedShim {
                    launcher,
                    held: None,
                })
            }
            Self::Linked(linked) => {
                if let Some(path) = &linked.env_path {
                    return Ok(ResolvedShim {
                        launcher: path.clone(),
                        held: None,
                    });
                }
                let fetching = fetching.ok_or_else(|| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        "pinned download settings were not prepared for engine executable",
                    )
                })?;
                let prepared = cache
                    .prepare(
                        &linked.pin,
                        &linked.pinned_by,
                        fetching,
                        now,
                        &mut |progress| {
                            tracing::debug!(
                                artifact = %linked.pin.id.name,
                                received = progress.received_bytes,
                                total = ?progress.total_bytes,
                                "downloading engine executable for plugin-shim",
                            );
                        },
                    )
                    .await
                    .map_err(|error| error.to_call_error())?;
                let launcher = executable_in_files(
                    &prepared.artifact.files(),
                    &linked.pin.url,
                    linked.pinned_by.product,
                );
                Ok(ResolvedShim {
                    launcher,
                    held: Some(prepared.artifact),
                })
            }
        }
    }
}

/// Path to the executable inside a prepared `EngineExecutable`'s `files/`
/// directory.
fn executable_in_files(files: &Path, url: &str, product: Product) -> PathBuf {
    let segment = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| product.executable_name());
    if Path::new(segment)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        let name = if cfg!(windows) {
            format!("{}.exe", product.executable_name())
        } else {
            product.executable_name().to_owned()
        };
        files.join(name)
    } else {
        files.join(segment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_product_reads_only_its_own_engine_path_variable() {
        let version = ProductVersion::parse("0.1.0").unwrap();
        let env = BTreeMap::from([
            (
                OsString::from(COLLIE_ENGINE_PATH_VARIABLE),
                OsString::from("/opt/collie"),
            ),
            (
                OsString::from(FOXHOUND_ENGINE_PATH_VARIABLE),
                OsString::from("/opt/foxhound"),
            ),
        ]);

        let collie =
            PluginShim::linked(Product::Collie, version.clone(), None).with_environment(&env);
        let foxhound = PluginShim::linked(Product::Foxhound, version, None).with_environment(&env);

        match collie {
            PluginShim::Linked(linked) => {
                assert_eq!(linked.env_path, Some(PathBuf::from("/opt/collie")));
            }
            PluginShim::CurrentExecutable => panic!("expected linked shim"),
        }
        match foxhound {
            PluginShim::Linked(linked) => {
                assert_eq!(linked.env_path, Some(PathBuf::from("/opt/foxhound")));
            }
            PluginShim::CurrentExecutable => panic!("expected linked shim"),
        }

        let foxhound_only = BTreeMap::from([(
            OsString::from(FOXHOUND_ENGINE_PATH_VARIABLE),
            OsString::from("/opt/foxhound"),
        )]);
        let collie_missing = PluginShim::linked(
            Product::Collie,
            ProductVersion::parse("0.1.0").unwrap(),
            None,
        )
        .with_environment(&foxhound_only);
        assert!(collie_missing.needs_cache());
    }
}
