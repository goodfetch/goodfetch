//! The Browser cache (ADRs 0024, 0043, 0098 and 0115): the per-user folder,
//! shared by both Products and every major, of the Pinned artifacts each
//! Product version asks for. A launch prepares its build here: the build is
//! checked against its pinned digest every time and held under a shared
//! lock; a build with the wrong digest is refused and removed; a missing one
//! is downloaded on first use through `pinned_download` with its progress
//! reported, unless `egress.offline` refuses it or the engine cannot write
//! the cache. `browser install`
//! prepares every pin ahead of time, and the clean-up deletes only what
//! nothing installed pins, nothing used for `browser_cache.max_unused_age`
//! and nothing holds.
//!
//! Its root is `GOODFETCH_BROWSERS_PATH` when that is set, otherwise
//! `browsers` in Goodfetch home (ADR 0059).

mod artifact;
mod clean_up;
mod copy;
mod extract;
mod platform;
mod store;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use goodfetch_protocol::browser_cache::{
    ARTIFACT_DOWNLOAD_FAILED, ARTIFACT_UNAVAILABLE_OFFLINE, ArtifactDownloadFailedDetails,
    ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails, BROWSER_NOT_INSTALLED,
    BrowserCacheEntry, BrowserNotInstalledDetails, DownloadFailure, DownloadProgress,
    InstallOutcome, MissingBrowser, PinnedBy,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::formats::BROWSER_CACHE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::{BROWSERS_PATH_VARIABLE, SettingLimit, SettingsLayer};

pub use artifact::{BuildPlatform, PinnedArtifact, Pins, content_digest};
pub use clean_up::{CleanUp, CleanUpReport, InstalledExecutable, MaxUnusedAge, kept_artifacts};
pub use copy::{ExecutableCopy, entry_present, hold as hold_copy, place_executable_copy};
pub use platform::{
    Arch, BrowserChoice, BrowserSource, CHROME_FOR_TESTING, CHROME_HEADLESS_SHELL, Os, Platform,
    PlatformNotice, browser_source,
};

use crate::call_error::CallError;
use crate::pinned_download::{self, Digest, Failure, Fetching};
use crate::tasks::BlockingThreads;
use store::{Locking, Read};

/// The folder of Goodfetch home the Browser cache sits in by default.
pub const HOME_FOLDER: &str = "browsers";

/// Where the Browser cache is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheLocation(PathBuf);

impl CacheLocation {
    /// `browsers_path`, from `GOODFETCH_BROWSERS_PATH`, when it is set;
    /// otherwise `browsers` in the Goodfetch home `home`.
    pub fn new(home: &Path, browsers_path: Option<PathBuf>) -> Self {
        Self(browsers_path.unwrap_or_else(|| home.join(HOME_FOLDER)))
    }

    /// The cache's root folder.
    pub fn root(&self) -> &Path {
        &self.0
    }
}

/// A build in the cache with its pinned digest, held under a shared lock
/// until it drops, so no clean-up deletes it while a browser runs from it.
pub struct HeldArtifact {
    folder: PathBuf,
    _lock: Option<store::Lock>,
}

impl HeldArtifact {
    /// The folder of its files.
    pub fn files(&self) -> PathBuf {
        store::files(&self.folder)
    }
}

/// What preparing an artifact did.
pub struct Prepared {
    /// The artifact, held.
    pub artifact: HeldArtifact,
    /// Whether it was downloaded into the cache now, after which the
    /// automatic clean-up may run.
    pub downloaded: bool,
}

/// Why the Browser cache could not give an artifact. Each variant but
/// [`CacheError::Cache`] is an Error code of ADR 0115.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    /// `browser_not_installed`.
    #[error("{message}")]
    BrowserNotInstalled {
        /// What went wrong, for people.
        message: String,
        /// The code's details.
        details: Box<BrowserNotInstalledDetails>,
    },
    /// `artifact_unavailable_offline`.
    #[error("{message}")]
    Offline {
        /// What went wrong, for people.
        message: String,
        /// The code's details.
        details: Box<ArtifactUnavailableOfflineDetails>,
    },
    /// `artifact_download_failed`.
    #[error("{message}")]
    DownloadFailed {
        /// What went wrong, for people.
        message: String,
        /// The code's details.
        details: Box<ArtifactDownloadFailedDetails>,
    },
    /// The cache's own files could not be read or written.
    #[error("the Browser cache at {path} failed: {source}")]
    Cache {
        /// The path that failed.
        path: PathBuf,
        /// What failed.
        source: io::Error,
    },
}

impl CacheError {
    /// Its Error code's name, or `internal_error` for the cache's own files.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BrowserNotInstalled { .. } => BROWSER_NOT_INSTALLED.name,
            Self::Offline { .. } => ARTIFACT_UNAVAILABLE_OFFLINE.name,
            Self::DownloadFailed { .. } => ARTIFACT_DOWNLOAD_FAILED.name,
            Self::Cache { .. } => ProtocolFailure::InternalError.name(),
        }
    }

    /// The failure a call answers with.
    pub fn to_call_error(&self) -> CallError {
        match self {
            Self::BrowserNotInstalled { message, details } => {
                CallError::new(&BROWSER_NOT_INSTALLED, message.clone(), details)
            }
            Self::Offline { message, details } => {
                CallError::new(&ARTIFACT_UNAVAILABLE_OFFLINE, message.clone(), details)
            }
            Self::DownloadFailed { message, details } => {
                CallError::new(&ARTIFACT_DOWNLOAD_FAILED, message.clone(), details)
            }
            Self::Cache { .. } => {
                CallError::protocol(ProtocolFailure::InternalError, self.to_string())
            }
        }
    }

    fn cache(path: &Path) -> impl FnOnce(io::Error) -> Self {
        let path = path.to_owned();
        move |source| Self::Cache { path, source }
    }
}

/// The command that installs `artifact`, pinned by `product`: its
/// executable's `browser install` (ADR 0098), which names the headless
/// shell, since only the builds a Run launches by default install bare
/// (#3224).
fn install_command(product: Product, artifact: &ArtifactId) -> String {
    let executable = product.executable_name();
    if artifact.name == CHROME_HEADLESS_SHELL {
        format!("{executable} browser install {CHROME_HEADLESS_SHELL}")
    } else {
        format!("{executable} browser install")
    }
}

fn describe(artifact: &ArtifactId, pinned_by: &PinnedBy) -> String {
    format!(
        "the {} build {} {}, pinned by {} {}",
        artifact.kind.as_str(),
        artifact.name,
        artifact.build,
        pinned_by.product.name(),
        pinned_by.product_version,
    )
}

/// Why a missing build cannot be put in the cache.
#[derive(Clone, Copy)]
enum Unreachable {
    /// The engine cannot write the cache.
    ReadOnly,
    /// Its pin names no archive digest, so no download could be checked.
    NoArchiveDigest,
}

/// What a check of the build at its pinned path found.
enum Present {
    /// The build, with its pinned digest, held.
    Held(HeldArtifact),
    /// No usable build: absent, being deleted, or refused and removed.
    Missing,
    /// A build with the wrong digest whose entry this engine cannot read,
    /// or in a cache it cannot write, so it can neither use nor remove it.
    Refused,
}

/// The Browser cache at one location.
#[derive(Clone, Debug)]
pub struct BrowserCache {
    root: PathBuf,
    blocking: BlockingThreads,
}

impl BrowserCache {
    /// The cache at `location`, whose hashing, unpacking and other file
    /// work runs on `blocking`; nothing is read until it is used.
    pub fn new(location: &CacheLocation, blocking: BlockingThreads) -> Self {
        Self {
            root: location.root().to_owned(),
            blocking,
        }
    }

    /// Its root folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether this engine can write it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing.
    pub async fn writable(&self) -> bool {
        let root = self.root.clone();
        self.blocking.run(move || store::writable(&root)).await
    }

    /// The build `artifact`, which `pinned_by` pins, held for a launch: the
    /// one in the cache when its files have the pinned digest, otherwise
    /// downloaded into it now, reporting progress to `progress`. A build
    /// with the wrong digest is refused and removed.
    ///
    /// # Errors
    ///
    /// `browser_not_installed` when the build is missing and the engine
    /// cannot write the cache, `artifact_unavailable_offline` when it is
    /// missing under `offline`, `artifact_download_failed` when its download
    /// fails, and `internal_error` when the cache's own files, the build's
    /// or those its archive unpacks to, cannot be read or written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves no partial build in place: an install in
    /// progress is removed, and the next call starts again.
    pub async fn prepare(
        &self,
        artifact: &PinnedArtifact,
        pinned_by: &PinnedBy,
        fetching: &Fetching,
        now: jiff::Timestamp,
        progress: &mut (dyn FnMut(DownloadProgress) + Send),
    ) -> Result<Prepared, CacheError> {
        for _ in 0..2 {
            match self.present_now(artifact, now).await? {
                Present::Held(held) => {
                    return Ok(Prepared {
                        artifact: held,
                        downloaded: false,
                    });
                }
                Present::Refused => {
                    return Err(self.not_installed(artifact, pinned_by, Unreachable::ReadOnly));
                }
                Present::Missing => {}
            }
            if let Some(layer) = fetching.offline {
                return Err(self.offline(artifact, pinned_by, layer));
            }
            let Some(archive_digest) = artifact.archive_digest else {
                return Err(self.not_installed(artifact, pinned_by, Unreachable::NoArchiveDigest));
            };
            if !self.writable().await {
                return Err(self.not_installed(artifact, pinned_by, Unreachable::ReadOnly));
            }
            if let Some(held) = self
                .download(artifact, archive_digest, pinned_by, fetching, now, progress)
                .await?
            {
                return Ok(Prepared {
                    artifact: held,
                    downloaded: true,
                });
            }
        }
        // Another process installed it in between, twice: it is there now.
        match self.present_now(artifact, now).await? {
            Present::Held(held) => Ok(Prepared {
                artifact: held,
                downloaded: false,
            }),
            Present::Missing | Present::Refused => {
                Err(self.not_installed(artifact, pinned_by, Unreachable::ReadOnly))
            }
        }
    }

    /// Prepares every artifact `pins` pins for `platform` (`None` for one no
    /// build is made for), then releases them: what `browser install` does
    /// for CI and images.
    ///
    /// # Errors
    ///
    /// As [`BrowserCache::prepare`], for the first artifact that fails.
    ///
    /// # Cancel safety
    ///
    /// As [`BrowserCache::prepare`]: artifacts installed before the drop
    /// stay installed.
    pub async fn install(
        &self,
        pins: &Pins,
        platform: Option<BuildPlatform>,
        fetching: &Fetching,
        now: jiff::Timestamp,
        progress: &mut (dyn FnMut(DownloadProgress) + Send),
    ) -> Result<Vec<(ArtifactId, InstallOutcome)>, CacheError> {
        let mut outcomes = Vec::new();
        for artifact in pins.for_platform(platform) {
            let prepared = self
                .prepare(artifact, &pins.pinned_by, fetching, now, progress)
                .await?;
            let outcome = if prepared.downloaded {
                InstallOutcome::Installed
            } else {
                InstallOutcome::AlreadyInstalled
            };
            outcomes.push((artifact.id.clone(), outcome));
        }
        Ok(outcomes)
    }

    /// Deletes what `request` allows: entries no artifact of `kept` names,
    /// unused for its age, and held by no process; nothing in a cache this
    /// engine cannot write, and no entry it cannot read. Finishes first any
    /// deletion an earlier run was cut off in.
    ///
    /// # Errors
    ///
    /// When the cache's files cannot be read or deleted.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a renamed entry, which the next
    /// clean-up deletes.
    pub async fn clean_up(&self, request: &CleanUp) -> Result<CleanUpReport, CacheError> {
        let (root, request) = (self.root.clone(), request.clone());
        self.blocking
            .run(move || clean_up::run(&root, &request))
            .await
            .map_err(CacheError::cache(&self.root))
    }

    /// The automatic clean-up after a download into the cache: at most once
    /// a day, never while `max_unused_age` is `null`. `None` when it did not
    /// run.
    ///
    /// # Errors
    ///
    /// When the cache's files cannot be read or deleted.
    ///
    /// # Cancel safety
    ///
    /// As [`BrowserCache::clean_up`].
    pub async fn clean_up_daily(
        &self,
        kept: BTreeSet<ArtifactId>,
        now: jiff::Timestamp,
        max_unused_age: MaxUnusedAge,
    ) -> Result<Option<CleanUpReport>, CacheError> {
        let root = self.root.clone();
        self.blocking
            .run(move || clean_up::run_daily(&root, kept, now, max_unused_age))
            .await
            .map_err(CacheError::cache(&self.root))
    }

    /// [`BrowserCache::present`], off the async workers.
    async fn present_now(
        &self,
        artifact: &PinnedArtifact,
        now: jiff::Timestamp,
    ) -> Result<Present, CacheError> {
        let (cache, artifact) = (self.clone(), artifact.clone());
        self.blocking
            .run(move || cache.present(&artifact, now))
            .await
    }

    fn present(
        &self,
        artifact: &PinnedArtifact,
        now: jiff::Timestamp,
    ) -> Result<Present, CacheError> {
        let folder = store::entry_folder(&self.root, &artifact.id);
        let failed = CacheError::cache(&folder);
        let entry = match store::read_entry(&folder, &artifact.id) {
            Read::Absent => return Ok(Present::Missing),
            Read::Entry(entry) => Some(entry),
            Read::Unreadable => None,
        };
        let lock = match store::lock_shared(&folder).map_err(failed)? {
            Locking::Taken(lock) => Some(lock),
            // A deletion holds it: the build is on its way out.
            Locking::Held => return Ok(Present::Missing),
            Locking::Missing => None,
        };
        let files = store::files(&folder);
        // A tree it cannot read now is no tampered one: it stays.
        let digest = if files.is_dir() {
            Some(content_digest(&files).map_err(CacheError::cache(&files))?)
        } else {
            None
        };
        if digest == Some(artifact.digest) {
            if let Some(mut entry) = entry
                && store::writable(&self.root)
            {
                entry.used_at = SystemTime::from(now);
                // A launch still runs when its use cannot be recorded; the
                // clean-up then counts the build unused since its last
                // recorded use.
                if let Err(error) = store::write_entry(&folder, &entry) {
                    tracing::warn!(
                        folder = %folder.display(),
                        %error,
                        "the use of a build in the Browser cache could not be recorded",
                    );
                }
            }
            return Ok(Present::Held(HeldArtifact {
                folder,
                _lock: lock,
            }));
        }
        drop(lock);
        if entry.is_none() || !store::writable(&self.root) {
            tracing::warn!(
                folder = %folder.display(),
                "a build in the Browser cache does not have its pinned digest, and cannot be removed",
            );
            return Ok(Present::Refused);
        }
        tracing::warn!(
            folder = %folder.display(),
            "a build in the Browser cache does not have its pinned digest: removing it",
        );
        store::delete(&self.root, &folder, None).map_err(CacheError::cache(&folder))?;
        Ok(Present::Missing)
    }

    /// The artifact `id` when the cache holds it with files of `digest`,
    /// held so the clean-up leaves it alone, its use recorded at `now`; a
    /// copy of another digest is removed where the cache is writable. It
    /// reads and hashes the artifact's files: run it on a blocking task.
    ///
    /// # Errors
    ///
    /// When the cache's files cannot be read or written.
    pub fn find(
        &self,
        id: &ArtifactId,
        digest: Digest,
        now: jiff::Timestamp,
    ) -> Result<Option<HeldArtifact>, CacheError> {
        let artifact = PinnedArtifact {
            id: id.clone(),
            platform: None,
            url: String::new(),
            archive_digest: None,
            application_settings: std::collections::BTreeMap::new(),
            digest,
        };
        Ok(match self.present(&artifact, now)? {
            Present::Held(held) => Some(held),
            Present::Missing | Present::Refused => None,
        })
    }

    /// Puts `files`, each a relative path with its bytes, in the cache as an
    /// artifact of `kind` named `name` whose build is the hexadecimal digest
    /// of those files, and answers that digest and the artifact, held: what
    /// the engine fetched itself from a configured source, such as a
    /// version of the Clean-up rules (ADR 0047). When the cache already
    /// holds it, that copy is answered. It writes and hashes the files: run
    /// it on a blocking task.
    ///
    /// # Errors
    ///
    /// When the cache cannot be written, or a path leaves its folder.
    pub fn keep(
        &self,
        kind: ArtifactKind,
        name: &str,
        files: &[(String, Vec<u8>)],
        now: jiff::Timestamp,
    ) -> Result<(Digest, HeldArtifact), CacheError> {
        let partial = store::partial(&self.root).map_err(CacheError::cache(&self.root))?;
        let folder = store::files(partial.path());
        let digest = store::write_files(&folder, files)
            .and_then(|()| content_digest(&folder))
            .map_err(CacheError::cache(partial.path()))?;
        let id = ArtifactId {
            kind,
            name: name.to_owned(),
            build: digest.hex(),
        };
        let entry = BrowserCacheEntry {
            format_version: BROWSER_CACHE.version,
            artifact: id.clone(),
            digest: digest.to_string(),
            installed_at: SystemTime::from(now),
            used_at: SystemTime::from(now),
        };
        let entry_folder = store::entry_folder(&self.root, &id);
        store::install(&self.root, partial, &id, &entry)
            .map_err(CacheError::cache(&entry_folder))?;
        match self.find(&id, digest, now)? {
            Some(held) => Ok((digest, held)),
            None => Err(CacheError::Cache {
                path: entry_folder,
                source: io::Error::new(
                    io::ErrorKind::NotFound,
                    "the artifact just put in the Browser cache is gone",
                ),
            }),
        }
    }

    /// Downloads and installs `artifact`; `None` when another process put it
    /// in place first.
    async fn download(
        &self,
        artifact: &PinnedArtifact,
        archive_digest: Digest,
        pinned_by: &PinnedBy,
        fetching: &Fetching,
        now: jiff::Timestamp,
        progress: &mut (dyn FnMut(DownloadProgress) + Send),
    ) -> Result<Option<HeldArtifact>, CacheError> {
        let Some(url) = fetching.located(&artifact.url) else {
            return Err(CacheError::Cache {
                path: self.root.clone(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "the pinned URL of {} is no https URL without a query",
                        artifact.id.name
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
        // A URL naming a ZIP archive is unpacked; any other serves one
        // executable, placed under its own name (ADR 0020's Lightpanda).
        let file_name = url.file_name().to_owned();
        let host = url.host().to_owned();
        let mut report = |received_bytes, total_bytes| {
            progress(DownloadProgress {
                artifact: artifact.id.clone(),
                host: host.clone(),
                received_bytes,
                total_bytes,
            });
        };
        tracing::info!(artifact = %artifact.id.name, build = %artifact.id.build, %host, "downloading into the Browser cache");
        let received = pinned_download::download(
            &url,
            archive_digest,
            fetching,
            &archive,
            &self.blocking,
            &mut report,
        )
        .await;
        if let Err(failure) = received {
            return Err(self.failed(failure, (artifact, pinned_by), &host, archive));
        }
        let files = store::files(partial.path());
        if unpacked(&self.blocking, archive.clone(), files, file_name).await? != artifact.digest {
            return Err(self.failed(
                Failure::DigestMismatch,
                (artifact, pinned_by),
                &host,
                archive,
            ));
        }
        let entry = BrowserCacheEntry {
            format_version: BROWSER_CACHE.version,
            artifact: artifact.id.clone(),
            digest: artifact.digest.to_string(),
            installed_at: SystemTime::from(now),
            used_at: SystemTime::from(now),
        };
        let folder = store::entry_folder(&self.root, &artifact.id);
        let (root, id) = (self.root.clone(), artifact.id.clone());
        let placed = self
            .blocking
            .run(move || store::install(&root, partial, &id, &entry))
            .await
            .map_err(CacheError::cache(&folder))?;
        if !placed {
            return Ok(None);
        }
        let locked = folder.clone();
        let lock = match self
            .blocking
            .run(move || store::lock_shared(&locked))
            .await
            .map_err(CacheError::cache(&folder))?
        {
            Locking::Taken(lock) => Some(lock),
            Locking::Held | Locking::Missing => None,
        };
        Ok(Some(HeldArtifact {
            folder,
            _lock: lock,
        }))
    }

    fn not_installed(
        &self,
        artifact: &PinnedArtifact,
        pinned_by: &PinnedBy,
        why: Unreachable,
    ) -> CacheError {
        let cache = self.root.display().to_string();
        let message = format!(
            "{} is not in the Browser cache at {cache}, {}",
            describe(&artifact.id, pinned_by),
            match why {
                Unreachable::ReadOnly => "which this engine cannot write",
                Unreachable::NoArchiveDigest =>
                    "and its pin names no archive digest yet, so it is never downloaded",
            },
        );
        CacheError::BrowserNotInstalled {
            message,
            details: Box::new(BrowserNotInstalledDetails {
                missing: MissingBrowser::Pinned {
                    artifact: artifact.id.clone(),
                    pinned_by: pinned_by.clone(),
                    cache,
                },
                fix: format!(
                    "Run `{}` where the Browser cache is writable, such as in the image's build, or point {BROWSERS_PATH_VARIABLE} at a cache that holds it",
                    install_command(pinned_by.product, &artifact.id),
                ),
            }),
        }
    }

    /// The error of a download of `artifact` from `host` that failed with
    /// `failure`, writing into the file at `archive`.
    fn failed(
        &self,
        failure: Failure,
        (artifact, pinned_by): (&PinnedArtifact, &PinnedBy),
        host: &str,
        archive: PathBuf,
    ) -> CacheError {
        let pinned = (artifact, pinned_by);
        match failure {
            Failure::Offline(layer) => self.offline(artifact, pinned_by, layer),
            Failure::Network => {
                download_failed(pinned, host, DownloadFailure::Network, "the network failed")
            }
            // A mirror the endpoint policy refuses never connected, as a
            // Clean-up rules source it refuses (ADR 0047): the message
            // says why in place of the network's failure.
            Failure::Refused(refused) => {
                download_failed(pinned, host, DownloadFailure::Network, &refused.to_string())
            }
            Failure::Status(status) => download_failed(
                pinned,
                host,
                DownloadFailure::HttpStatus { status },
                &format!("{host} answered {status}"),
            ),
            Failure::DigestMismatch => download_failed(
                pinned,
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

    fn offline(
        &self,
        artifact: &PinnedArtifact,
        pinned_by: &PinnedBy,
        layer: SettingsLayer,
    ) -> CacheError {
        let cache = self.root.display().to_string();
        let message = format!(
            "{} is not in the Browser cache at {cache}, and `egress.offline` refuses its download",
            describe(&artifact.id, pinned_by),
        );
        CacheError::Offline {
            message,
            details: Box::new(ArtifactUnavailableOfflineDetails {
                artifact: artifact.id.clone(),
                pinned_by: pinned_by.clone(),
                cache,
                limit: SettingLimit {
                    key: EGRESS_OFFLINE.name.to_owned(),
                    value: serde_json::Value::Bool(true),
                    layer,
                    origin: None,
                },
                fix: format!(
                    "Run `{}` beforehand where the Browser cache is writable, or lift `{}` where it was set",
                    install_command(pinned_by.product, &artifact.id),
                    EGRESS_OFFLINE.name,
                ),
            }),
        }
    }
}

/// Unpacks the archive at `archive` into `files`, or places it there as the
/// executable `file_name` names when it is no ZIP archive, and answers the
/// digest of what it wrote. The archive had its pinned digest: a failure
/// to write its files out, such as a full disk, is the cache's own.
async fn unpacked(
    blocking: &BlockingThreads,
    archive: PathBuf,
    files: PathBuf,
    file_name: String,
) -> Result<Digest, CacheError> {
    let written = files.clone();
    blocking
        .run(move || {
            if Path::new(&file_name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
            {
                extract::unpack(&archive, &files)?;
            } else {
                extract::place(&archive, &files, &file_name)?;
            }
            content_digest(&files)
        })
        .await
        .map_err(CacheError::cache(&written))
}

/// `artifact_download_failed` for `failure` from `host`, whose message
/// says `what` failed.
fn download_failed(
    (artifact, pinned_by): (&PinnedArtifact, &PinnedBy),
    host: &str,
    failure: DownloadFailure,
    what: &str,
) -> CacheError {
    let message = format!(
        "downloading {} from {host} failed: {what}",
        describe(&artifact.id, pinned_by),
    );
    CacheError::DownloadFailed {
        message,
        details: Box::new(ArtifactDownloadFailedDetails {
            artifact: artifact.id.clone(),
            pinned_by: pinned_by.clone(),
            host: host.to_owned(),
            failure,
            fix: format!(
                "Retry, or run `{}` beforehand, or point {BROWSERS_PATH_VARIABLE} at a prepared Browser cache",
                install_command(pinned_by.product, &artifact.id),
            ),
        }),
    }
}

/// The installed browser `source` names: the first of its candidates that
/// exists.
///
/// # Errors
///
/// `browser_not_installed` when none exists.
///
/// # Cancel safety
///
/// Dropping the future loses nothing.
pub async fn installed_browser(
    blocking: &BlockingThreads,
    choice: &BrowserChoice,
    candidates: &[PathBuf],
    product: Product,
) -> Result<PathBuf, CacheError> {
    let looked_in = candidates.to_vec();
    let found = blocking
        .run(move || looked_in.into_iter().find(|path| path.is_file()))
        .await;
    if let Some(found) = found {
        return Ok(found);
    }
    let (missing, fix) = match choice {
        BrowserChoice::Channel(channel) => (
            MissingBrowser::Channel {
                channel: *channel,
                looked_in: candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            },
            "Install Google Chrome of that channel, or name the browser by its path".to_owned(),
        ),
        BrowserChoice::Path(path) => (
            MissingBrowser::Path {
                path: path.display().to_string(),
            },
            "Correct the browser's path".to_owned(),
        ),
        BrowserChoice::Default => (
            MissingBrowser::Path {
                path: candidates
                    .first()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
            },
            format!(
                "Install the distribution's Chromium package, or name a browser by its path; {} runs Chrome for Testing where it has a build",
                product.name(),
            ),
        ),
    };
    let message = format!(
        "no browser at {}",
        match &missing {
            MissingBrowser::Channel { looked_in, .. } => looked_in.join(", "),
            MissingBrowser::Path { path } => path.clone(),
            MissingBrowser::Pinned { cache, .. } => cache.clone(),
        }
    );
    Err(CacheError::BrowserNotInstalled {
        message,
        details: Box::new(BrowserNotInstalledDetails { missing, fix }),
    })
}
