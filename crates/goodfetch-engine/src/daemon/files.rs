//! The Daemon's files on the machine (ADRs 0024, 0030 and 0059), in its
//! major's per-major folder of Goodfetch home, so each major has its own
//! and both Products share them:
//!
//! ```text
//! <home>/<major>/daemon.json         the runtime file, mode 0600
//! <home>/<major>/daemon.sock         the socket, mode 0600 (macOS and Linux)
//! <home>/<major>/daemon.lock         held exclusively while a Daemon serves
//! <home>/<major>/daemon-start.lock   held by whatever starts, or hands over, one
//! ```
//!
//! A Daemon is running while its runtime file is there and its serving
//! lock is held: a Daemon that died, whatever PID its file names, holds no
//! lock, so a Client replaces it, and a PID the OS reused never passes for
//! it. The start lock makes two Clients starting it at once start one. A
//! socket path too long for the OS moves into a folder of its own in the
//! OS's temporary folder, mode `0700`, which the runtime file names.
//!
//! These are blocking calls, made on the tasks module's blocking policy or
//! by an executable outside its runtime.
#![expect(
    clippy::disallowed_methods,
    reason = "the Daemon's files are read and written on the tasks module's blocking policy or outside any runtime, never on an async worker"
)]

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{DaemonEndpoint, DaemonRuntimeFile};

use crate::home::Home;

const RUNTIME_FILE: &str = "daemon.json";
const SOCKET: &str = "daemon.sock";
const SERVING_LOCK: &str = "daemon.lock";
const START_LOCK: &str = "daemon-start.lock";

/// The longest socket path every OS takes: macOS's `sun_path` holds 104
/// bytes, its last a NUL.
const LONGEST_SOCKET_PATH: usize = 103;

/// How many bytes of the OS generator a token takes.
const TOKEN_BYTES: usize = 32;

/// The Daemon's files of one major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonFiles {
    folder: PathBuf,
}

/// A lock held until it drops.
#[derive(Debug)]
pub struct FileLock(#[expect(dead_code, reason = "held for its lock, which drops with it")] File);

impl DaemonFiles {
    /// The files of the major of `version` in `home`.
    pub fn of(home: &Home, version: &ProductVersion) -> Self {
        Self {
            folder: home.per_major(version),
        }
    }

    /// The runtime file's path.
    pub fn runtime_file(&self) -> PathBuf {
        self.folder.join(RUNTIME_FILE)
    }

    /// Where the socket goes when its path is short enough for the OS.
    pub fn socket(&self) -> PathBuf {
        self.folder.join(SOCKET)
    }

    /// The runtime file, or `None` when there is none, or it is not one.
    ///
    /// # Errors
    ///
    /// When it exists but cannot be read.
    pub fn read(&self) -> io::Result<Option<DaemonRuntimeFile>> {
        match fs::read(self.runtime_file()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The running Daemon's runtime file: there, and its serving lock held.
    ///
    /// # Errors
    ///
    /// When the files cannot be read.
    pub fn running(&self) -> io::Result<Option<DaemonRuntimeFile>> {
        let Some(found) = self.read()? else {
            return Ok(None);
        };
        let lock = match File::open(self.folder.join(SERVING_LOCK)) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match lock.try_lock() {
            Ok(()) => Ok(None),
            Err(TryLockError::WouldBlock) => Ok(Some(found)),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// Publishes `file`, readable and writable by its owner alone, in one
    /// rename.
    ///
    /// # Errors
    ///
    /// When the per-major folder cannot be written.
    pub fn publish(&self, file: &DaemonRuntimeFile) -> io::Result<()> {
        fs::create_dir_all(&self.folder)?;
        let partial = self.folder.join(".daemon.json.partial");
        remove(&partial)?;
        let mut written = owner_only(&partial)?;
        written.write_all(&serde_json::to_vec(file).expect("invariant: a runtime file encodes"))?;
        written.sync_all()?;
        drop(written);
        fs::rename(&partial, self.runtime_file())
    }

    /// Removes the runtime file and the socket of the Daemon of `pid`,
    /// unless they are another Daemon's by now.
    ///
    /// # Errors
    ///
    /// When they cannot be removed.
    pub fn unpublish(&self, pid: u32) -> io::Result<()> {
        let Some(found) = self.read()? else {
            return Ok(());
        };
        if found.pid != pid {
            return Ok(());
        }
        remove(&self.runtime_file())?;
        if let DaemonEndpoint::Unix(socket) = &found.endpoint {
            remove(socket)?;
        }
        Ok(())
    }

    /// The serving lock, which a Daemon holds while it serves; waits while
    /// another Daemon of the major holds it, such as one handing over.
    ///
    /// # Errors
    ///
    /// When the lock file cannot be opened.
    pub fn serving_lock(&self) -> io::Result<FileLock> {
        self.lock(SERVING_LOCK)
    }

    /// The start lock, which whatever starts the Daemon, or hands it over,
    /// holds until the Daemon it started is ready.
    ///
    /// # Errors
    ///
    /// When the lock file cannot be opened.
    pub fn start_lock(&self) -> io::Result<FileLock> {
        self.lock(START_LOCK)
    }

    fn lock(&self, name: &str) -> io::Result<FileLock> {
        fs::create_dir_all(&self.folder)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.folder.join(name))?;
        file.lock()?;
        Ok(FileLock(file))
    }

    /// Where the socket goes: in the per-major folder, or, when that path
    /// is too long for the OS, in a folder of its own in the OS's temporary
    /// folder, which lives as long as the answered guard.
    ///
    /// # Errors
    ///
    /// When that folder cannot be made.
    pub fn socket_place(&self) -> io::Result<(PathBuf, Option<tempfile::TempDir>)> {
        let socket = self.socket();
        if socket.as_os_str().len() <= LONGEST_SOCKET_PATH {
            fs::create_dir_all(&self.folder)?;
            return Ok((socket, None));
        }
        let folder = tempfile::Builder::new()
            .prefix("goodfetch-daemon-")
            .tempdir()?;
        Ok((folder.path().join(SOCKET), Some(folder)))
    }
}

/// A per-launch token from the OS generator, never from `Entropy` (ADR
/// 0059): 32 bytes, in lowercase hex.
///
/// # Errors
///
/// When the OS generator gives nothing.
pub fn new_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().fold(String::new(), |mut token, byte| {
        let _ = write!(token, "{byte:02x}");
        token
    }))
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(unix)]
fn owner_only(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn owner_only(path: &Path) -> io::Result<File> {
    // A per-user folder of Goodfetch home is the owner's alone on Windows.
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}
