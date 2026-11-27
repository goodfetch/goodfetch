//! Plugin language runtimes in the shared per-user cache (ADRs 0033, 0043,
//! 0098 and 0115; issue 895): each runtime (`node` or `python`, by version
//! and platform) is downloaded by digest at `plugin add`, unpacked safely
//! into `<cache>/plugin_runtime/<name>/<build>/files/`, recorded with an
//! `entry.json` and shared `lock`, and verified before any Plugin runs
//! from it. Under `egress.offline`, a missing runtime fails with
//! `artifact_unavailable_offline` naming the artifact and every installed
//! Plugin that declares it; no download is attempted.
#![expect(
    clippy::disallowed_methods,
    reason = "blocking filesystem work runs on the tasks module's blocking policy, never on an async worker"
)]

use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;
use std::time::SystemTime;

use goodfetch_protocol::browser_cache::{
    ArtifactDownloadFailedDetails, ArtifactId, ArtifactUnavailableOfflineDetails, DownloadFailure,
    Pinner,
};
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};

use super::artifact::content_digest;
use super::extract;
use super::store::{self, Locking, Read};
use super::{BrowserCache, CacheError, HeldArtifact, Prepared};
use crate::pinned_download::{self, Digest, Failure, Fetching};
use crate::plugin_bundle;
use crate::plugin_oci::Compression;
use crate::tasks::BlockingThreads;

/// One Plugin language runtime pinned by URL and archive digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedRuntime {
    /// Its artifact id (`kind = PluginRuntime`, `name = node|python`,
    /// `build = <version>-<platform>`).
    pub id: ArtifactId,
    /// The pinned HTTPS URL serving its archive.
    pub url: String,
    /// The SHA-256 digest of the archive served by `url`.
    pub archive_digest: Digest,
}

impl BrowserCache {
    /// Ensures the Plugin runtime `runtime`, pinned by `pinned_by`, is in
    /// the cache and verified: returns it held if already present with its
    /// recorded content digest, otherwise downloads it from `runtime.url`
    /// under `fetching`, verifies `runtime.archive_digest`, unpacks it
    /// safely and installs it atomically.
    ///
    /// # Errors
    ///
    /// `artifact_unavailable_offline` when missing under `egress.offline`,
    /// `artifact_download_failed` with its `failure` detail when the
    /// download fails or its digest mismatches, and `CacheError::Cache`
    /// when the cache's files cannot be read or written.
    pub async fn prepare_plugin_runtime(
        &self,
        runtime: &PinnedRuntime,
        pinned_by: &[Pinner],
        fetching: &Fetching,
        now: jiff::Timestamp,
    ) -> Result<Prepared, CacheError> {
        for _ in 0..2 {
            if let Some(held) = self.present_runtime_now(&runtime.id, now).await? {
                return Ok(Prepared {
                    artifact: held,
                    downloaded: false,
                });
            }
            if let Some(layer) = fetching.offline {
                return Err(self.runtime_offline(&runtime.id, pinned_by, layer));
            }
            if !self.writable().await {
                return Err(CacheError::Cache {
                    path: self.root.clone(),
                    source: io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "the runtime cache is not writable",
                    ),
                });
            }
            if let Some(held) = self
                .download_runtime(runtime, pinned_by, fetching, now)
                .await?
            {
                return Ok(Prepared {
                    artifact: held,
                    downloaded: true,
                });
            }
        }
        match self.present_runtime_now(&runtime.id, now).await? {
            Some(held) => Ok(Prepared {
                artifact: held,
                downloaded: false,
            }),
            None => Err(CacheError::Cache {
                path: store::entry_folder(&self.root, &runtime.id),
                source: io::Error::new(
                    io::ErrorKind::NotFound,
                    "the Plugin runtime just installed in the cache is gone",
                ),
            }),
        }
    }

    /// Holds an already-cached Plugin runtime `id` without downloading:
    /// verifies its recorded digest on disk and takes its shared lock.
    /// When absent under `offline`, fails with
    /// `artifact_unavailable_offline` naming `id` and `pinned_by`.
    ///
    /// # Errors
    ///
    /// `artifact_unavailable_offline` when missing and `offline` is `Some`,
    /// or `CacheError::Cache` on unreadable cache files.
    pub async fn held_plugin_runtime(
        &self,
        id: &ArtifactId,
        pinned_by: &[Pinner],
        offline: Option<SettingsLayer>,
        now: jiff::Timestamp,
    ) -> Result<Option<HeldArtifact>, CacheError> {
        if let Some(held) = self.present_runtime_now(id, now).await? {
            return Ok(Some(held));
        }
        if let Some(layer) = offline {
            return Err(self.runtime_offline(id, pinned_by, layer));
        }
        Ok(None)
    }

    /// Checks whether the Plugin runtime `id` is already cached with intact
    /// files matching its `entry.json` digest.
    async fn present_runtime_now(
        &self,
        id: &ArtifactId,
        now: jiff::Timestamp,
    ) -> Result<Option<HeldArtifact>, CacheError> {
        let (cache, id) = (self.clone(), id.clone());
        self.blocking
            .run(move || cache.present_runtime(&id, now))
            .await
    }

    fn present_runtime(
        &self,
        id: &ArtifactId,
        now: jiff::Timestamp,
    ) -> Result<Option<HeldArtifact>, CacheError> {
        let folder = store::entry_folder(&self.root, id);
        let failed = CacheError::cache(&folder);
        let entry = match store::read_entry(&folder, id) {
            Read::Entry(entry) => entry,
            Read::Absent | Read::Unreadable => return Ok(None),
        };
        let Ok(expected) = entry.digest.parse::<Digest>() else {
            return Ok(None);
        };
        let lock = match store::lock_shared(&folder).map_err(failed)? {
            Locking::Taken(lock) => Some(lock),
            Locking::Held => return Ok(None),
            Locking::Missing => None,
        };
        let files = store::files(&folder);
        let actual = if files.is_dir() {
            Some(content_digest(&files).map_err(CacheError::cache(&files))?)
        } else {
            None
        };
        if actual == Some(expected) {
            if self.writes() {
                let mut updated = entry;
                updated.used_at = SystemTime::from(now);
                if let Err(error) = store::write_entry(&folder, &updated) {
                    tracing::warn!(
                        folder = %folder.display(),
                        %error,
                        "the use of a Plugin runtime in the cache could not be recorded",
                    );
                }
            }
            return Ok(Some(HeldArtifact {
                folder,
                _lock: lock,
            }));
        }
        drop(lock);
        if self.writes() {
            tracing::warn!(
                folder = %folder.display(),
                "a Plugin runtime in the cache does not match its recorded digest: removing it",
            );
            store::delete(&self.root, &folder, None).map_err(CacheError::cache(&folder))?;
        }
        Ok(None)
    }

    async fn download_runtime(
        &self,
        runtime: &PinnedRuntime,
        pinned_by: &[Pinner],
        fetching: &Fetching,
        now: jiff::Timestamp,
    ) -> Result<Option<HeldArtifact>, CacheError> {
        let Some(url) = fetching.located(&runtime.url) else {
            return Err(CacheError::Cache {
                path: self.root.clone(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "the pinned URL of runtime {} {} is no https URL without a query",
                        runtime.id.name, runtime.id.build
                    ),
                ),
            });
        };
        let root = self.root.clone();
        let partial = self
            .blocking
            .run(move || store::partial(&root))
            .await
            .map_err(CacheError::cache(&self.root))?;
        let archive = partial.path().join("archive");
        let file_name = url.file_name().to_owned();
        let host = url.host().to_owned();
        tracing::info!(
            runtime = %runtime.id.name,
            build = %runtime.id.build,
            %host,
            "downloading Plugin runtime into the cache",
        );
        let received = pinned_download::download(
            &url,
            runtime.archive_digest,
            fetching,
            &archive,
            &self.blocking,
            &mut |_, _| {},
        )
        .await;
        if let Err(failure) = received {
            return Err(self.runtime_failed(failure, &runtime.id, pinned_by, &host, archive));
        }
        let files = store::files(partial.path());
        let tree = unpack_runtime(&self.blocking, archive, files, file_name).await?;
        self.commit_partial(partial, &runtime.id, tree.to_string(), now)
            .await
    }

    fn runtime_offline(
        &self,
        artifact: &ArtifactId,
        pinned_by: &[Pinner],
        layer: SettingsLayer,
    ) -> CacheError {
        let cache = self.root.display().to_string();
        let plugins = describe_pinners(pinned_by);
        let message = format!(
            "the Plugin runtime {} {} ({plugins}) is not in the cache at {cache}, and `{}` refuses its download",
            artifact.name, artifact.build, EGRESS_OFFLINE.name,
        );
        CacheError::Offline {
            message,
            details: Box::new(ArtifactUnavailableOfflineDetails {
                artifact: artifact.clone(),
                pinned_by: pinned_by.to_vec(),
                cache,
                limit: SettingLimit {
                    key: EGRESS_OFFLINE.name.to_owned(),
                    value: serde_json::Value::Bool(true),
                    layer,
                    origin: None,
                },
                fix: format!(
                    "Run `plugin add` beforehand where `{}` is not set, or lift it where it was set",
                    EGRESS_OFFLINE.name,
                ),
            }),
        }
    }

    fn runtime_failed(
        &self,
        failure: Failure,
        artifact: &ArtifactId,
        pinned_by: &[Pinner],
        host: &str,
        archive: PathBuf,
    ) -> CacheError {
        match failure {
            Failure::Offline(layer) => self.runtime_offline(artifact, pinned_by, layer),
            Failure::Network => runtime_download_failed(
                artifact,
                pinned_by,
                host,
                DownloadFailure::Network,
                "the network failed",
            ),
            Failure::Refused(refused) => runtime_download_failed(
                artifact,
                pinned_by,
                host,
                DownloadFailure::Network,
                &refused.to_string(),
            ),
            Failure::Status(status) => runtime_download_failed(
                artifact,
                pinned_by,
                host,
                DownloadFailure::HttpStatus { status },
                &format!("{host} answered {status}"),
            ),
            Failure::DigestMismatch => runtime_download_failed(
                artifact,
                pinned_by,
                host,
                DownloadFailure::DigestMismatch,
                "what it sent does not have the pinned digest",
            ),
            Failure::File(source) => CacheError::Cache {
                path: archive,
                source,
            },
        }
    }
}

fn runtime_download_failed(
    artifact: &ArtifactId,
    pinned_by: &[Pinner],
    host: &str,
    failure: DownloadFailure,
    what: &str,
) -> CacheError {
    let plugins = describe_pinners(pinned_by);
    let message = format!(
        "downloading the Plugin runtime {} {} ({plugins}) from {host} failed: {what}",
        artifact.name, artifact.build,
    );
    CacheError::DownloadFailed {
        message,
        details: Box::new(ArtifactDownloadFailedDetails {
            artifact: artifact.clone(),
            pinned_by: pinned_by.to_vec(),
            host: host.to_owned(),
            failure,
            fix: format!("Retry `plugin add`, or check that {host} serves the pinned runtime"),
        }),
    }
}

fn describe_pinners(pinned_by: &[Pinner]) -> String {
    if pinned_by.is_empty() {
        return "used by a Plugin".to_owned();
    }
    let names: Vec<String> = pinned_by
        .iter()
        .map(|pinner| match pinner {
            Pinner::Plugin { name, version } => format!("{name} {version}"),
            Pinner::Product { name, version } => format!("{} {version}", name.name()),
        })
        .collect();
    format!("used by {}", names.join(", "))
}

/// Unpacks a verified Plugin runtime archive (`.tar.gz`, `.tgz`, `.tar`,
/// `.zip`, or single executable) into `files` and returns its tree digest.
async fn unpack_runtime(
    blocking: &BlockingThreads,
    archive: PathBuf,
    files: PathBuf,
    file_name: String,
) -> Result<Digest, CacheError> {
    let written = files.clone();
    blocking
        .run(move || {
            let path = std::path::Path::new(&file_name);
            let ext = path.extension();
            let is_tar_gz = ext.is_some_and(|e| e.eq_ignore_ascii_case("tgz"))
                || (ext.is_some_and(|e| e.eq_ignore_ascii_case("gz"))
                    && path
                        .file_stem()
                        .and_then(|stem| std::path::Path::new(stem).extension())
                        .is_some_and(|e| e.eq_ignore_ascii_case("tar")));
            if is_tar_gz {
                std::fs::create_dir_all(&files)?;
                let file = File::open(&archive)?;
                plugin_bundle::unpack(BufReader::new(file), Compression::Gzip, &files)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                std::fs::remove_file(&archive)?;
            } else if ext.is_some_and(|e| e.eq_ignore_ascii_case("tar")) {
                std::fs::create_dir_all(&files)?;
                let file = File::open(&archive)?;
                plugin_bundle::unpack(BufReader::new(file), Compression::None, &files)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                std::fs::remove_file(&archive)?;
            } else if ext.is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
                extract::unpack(&archive, &files)?;
            } else {
                extract::place(&archive, &files, &file_name)?;
            }
            content_digest(&files)
        })
        .await
        .map_err(CacheError::cache(&written))
}
