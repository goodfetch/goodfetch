//! The private endpoint's folder, socket path and token file. Blocking
//! calls, made on the tasks module's blocking policy.
#![expect(
    clippy::disallowed_methods,
    reason = "the private endpoint's files are written on the tasks module's blocking policy, never on an async worker"
)]

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::endpoint::new_token;
use crate::transport;

/// The socket's name in the folder.
const SOCKET: &str = "engine.sock";

/// The token file's name in the folder.
const TOKEN_FILE: &str = "engine.token";

/// An Attached engine's private endpoint files: its folder, removed when
/// the value drops, and the token its `hello`s carry.
#[derive(Debug)]
pub(crate) struct PrivateFiles {
    folder: tempfile::TempDir,
    token: String,
}

impl PrivateFiles {
    /// Makes the folder, mode `0700`, in the OS's temporary folder, or in
    /// `/tmp` when the socket's path there is too long for the OS, and
    /// writes a new token into its file, mode `0600`.
    ///
    /// # Errors
    ///
    /// When the folder or the file cannot be made, or the OS generator
    /// gives no token.
    pub(crate) fn create() -> io::Result<Self> {
        let folder = transport::socket_folder("goodfetch-engine-", SOCKET)?;
        let token = new_token().map_err(|error| io::Error::other(error.to_string()))?;
        let mut file = owner_only(&folder.path().join(TOKEN_FILE))?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        Ok(Self { folder, token })
    }

    /// Where its socket goes on macOS and Linux.
    pub(crate) fn socket(&self) -> PathBuf {
        self.folder.path().join(SOCKET)
    }

    /// The token file's path.
    pub(crate) fn token_file(&self) -> PathBuf {
        self.folder.path().join(TOKEN_FILE)
    }

    /// The token.
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

/// Reads the token a private endpoint's token file holds.
///
/// # Errors
///
/// When the file cannot be read.
pub(crate) fn read_token(token_file: &Path) -> io::Result<String> {
    Ok(std::fs::read_to_string(token_file)?.trim().to_owned())
}

#[cfg(unix)]
fn owner_only(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn owner_only(path: &Path) -> io::Result<File> {
    // A user's temporary folder is the user's own on Windows.
    OpenOptions::new().write(true).create_new(true).open(path)
}
