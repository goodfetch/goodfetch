//! The Daemon's runtime file (ADR 0024) as plain values: the file of a
//! Goodfetch home's per-major folder through which a Client finds the
//! Daemon of its major, holding its endpoint, per-launch token, Product
//! version and PID. `goodfetch-scripted-engine --mode socket` writes it, so
//! the CLI and every SDK find the Scripted engine exactly as they find a
//! Daemon (ADR 0069). Test support takes no Goodfetch crate, so
//! `goodfetch-test-client`'s tests read a Daemon's own runtime file
//! (`goodfetch-protocol`'s `DaemonRuntimeFile`) as this one, and its lock's
//! and socket's names as the Daemon's, to keep the two from drifting.
#![expect(
    clippy::disallowed_methods,
    reason = "test support writes and reads the runtime file outside any async worker (ADR 0062)"
)]

use std::fmt;
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The runtime file's name in the per-major folder.
pub const RUNTIME_FILE: &str = "daemon.json";

/// The start of the name of the folder under `/tmp` the Scripted engine
/// listens in when its socket's path in the home is longer than the OS takes
/// (104 bytes on macOS), with a space and a non-ASCII letter like a test's
/// home (ADR 0073). A `TestEnv` removes one a runtime file of its home names
/// at teardown, since a killed Daemon cannot.
pub const SOCKET_FOLDER_PREFIX: &str = "goodfetch é ";

/// The Unix socket's name in the per-major folder, where its path fits
/// the OS's limit.
pub const DAEMON_SOCKET: &str = "daemon.sock";

/// The lock a live Daemon holds beside its runtime file: a runtime file
/// whose lock nobody holds is a dead Daemon's.
pub const DAEMON_SERVING_LOCK: &str = "daemon.lock";

/// The token `goodfetch-scripted-engine --mode socket` writes unless
/// `--token` names another, which the corpus's Daemon scripts expect in
/// `hello`.
pub const SCRIPTED_DAEMON_TOKEN: &str = "scripted-daemon-token";

/// Where a Daemon listens: a Unix socket on macOS and Linux, loopback TCP
/// on Windows (ADR 0024).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonEndpoint {
    /// A Unix socket's path.
    Unix(PathBuf),
    /// A loopback TCP address.
    Tcp(SocketAddr),
}

/// The endpoint as a Client names it: the socket's path or the address.
impl fmt::Display for DaemonEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(formatter, "{}", path.display()),
            Self::Tcp(address) => write!(formatter, "{address}"),
        }
    }
}

/// What a Daemon's runtime file holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFile {
    /// Where the Daemon listens.
    pub endpoint: DaemonEndpoint,
    /// The per-launch token a Client's `hello` carries.
    pub token: String,
    /// The Daemon's Product version.
    pub version: String,
    /// The Daemon's process.
    pub pid: u32,
}

impl RuntimeFile {
    /// The runtime file of the Daemon of `product_version`'s major in
    /// `home`: `<home>/<major>/daemon.json`, the major being `1` for any
    /// `1.x.y` and `0.4` for any `0.4.y` (ADR 0030).
    pub fn path(home: &Path, product_version: &str) -> PathBuf {
        home.join(major_folder(product_version)).join(RUNTIME_FILE)
    }

    /// Reads the runtime file at `path`.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or holds no runtime file.
    pub fn read(path: &Path) -> Result<Self, RuntimeFileError> {
        let text = std::fs::read_to_string(path).map_err(|source| RuntimeFileError::Read {
            path: path.to_owned(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| RuntimeFileError::Invalid {
            path: path.to_owned(),
            source,
        })
    }

    /// Writes it at `path`, readable by its owner only on Unix, making its
    /// folder if needed and replacing any file there whole, so a Client
    /// never reads half of it.
    ///
    /// # Errors
    ///
    /// The folder or the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), RuntimeFileError> {
        let failed = |source| RuntimeFileError::Write {
            path: path.to_owned(),
            source,
        };
        let folder = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(folder).map_err(failed)?;
        let partial = path.with_extension("json.partial");
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        restrict_to_owner(&mut options);
        let mut file = options.open(&partial).map_err(failed)?;
        let text = serde_json::to_string(self).expect("invariant: a runtime file serializes");
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(failed)?;
        drop(file);
        std::fs::rename(&partial, path).map_err(failed)
    }
}

/// The per-major folder's name for a Product version.
pub(crate) fn major_folder(product_version: &str) -> String {
    let mut numbers = product_version.split(['.', '-']);
    let major = numbers.next().unwrap_or("0");
    match (major, numbers.next()) {
        ("0", Some(minor)) => format!("0.{minor}"),
        (major, _) => major.to_owned(),
    }
}

#[cfg(unix)]
fn restrict_to_owner(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn restrict_to_owner(_options: &mut OpenOptions) {}

/// Why a runtime file cannot be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RuntimeFileError {
    /// The file cannot be read.
    #[error("the runtime file {} cannot be read: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The file holds no runtime file.
    #[error("the runtime file {} holds no runtime file: {source}", path.display())]
    Invalid {
        /// The file.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
    /// The file or its folder cannot be written.
    #[error("the runtime file {} cannot be written: {source}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
}
