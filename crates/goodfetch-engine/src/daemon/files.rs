//! The Daemon's files on the machine (ADRs 0024, 0030 and 0059), in its
//! major's per-major folder of Goodfetch home, so each major has its own
//! and both Products share them, named by `goodfetch_protocol::home`'s
//! `DaemonFile`:
//!
//! ```text
//! <home>/<major>/daemon.json         the runtime file, mode 0600
//! <home>/<major>/daemon.sock         the socket, mode 0600 (macOS and Linux)
//! <home>/<major>/daemon.lock         held exclusively while a Daemon serves
//! <home>/<major>/daemon-start.lock   held by whatever starts, or hands over, one
//! <home>/<major>/daemon.token        the persistent token, mode 0600, of a
//!                                    socket-activated Daemon
//! <home>/<major>/daemon-service.json the per-user service `daemon install`
//!                                    registered
//! <home>/<major>/ops.token           the ops token, mode 0600, which the
//!                                    ops listener's `/metrics` takes
//! <home>/<major>/http-listener.token the HTTP listener's token, mode 0600,
//!                                    which `daemon http-listener-token`
//!                                    prints and `daemon rotate-token`
//!                                    replaces
//! ```
//!
//! A Daemon is running while its runtime file is there and its serving
//! lock is held: a Daemon that died, whatever PID its file names, holds no
//! lock, so a Client replaces it, and a PID the OS reused never passes for
//! it. The start lock makes two Clients starting it at once start one. A
//! socket path too long for the OS moves into a folder of its own in the
//! OS's temporary folder, or in `/tmp` when that one is too deep as well,
//! mode `0700`, which the runtime file names.
//!
//! A service's socket must stay where its service manager listens, so one
//! too long for the OS, or under systemd one whose path holds a byte that
//! systemd escapes, moves instead into `/tmp/goodfetch-daemon-<digest>`,
//! named by the per-major folder's path, mode `0700` and refused unless it
//! is the user's own. A socket-activated Daemon starts before any Client
//! could read a runtime file it writes, so its Clients' `hello` carries a
//! persistent token, which `daemon rotate-token` replaces (ADR 0031). The
//! ops token is a token of its own, scoped to reading the metrics of the
//! ops listener and taken by no other listener (ADR 0050); an operator may
//! write it before the Daemon starts, as a Kubernetes Secret mounted there
//! does, and the Daemon makes one when it finds none.
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
use goodfetch_protocol::daemon::{
    DaemonEndpoint, DaemonRuntimeFile, InstalledService, ServiceManager,
};
use goodfetch_protocol::home::DaemonFile;

use sha2::{Digest as _, Sha256};

use crate::endpoint::new_token;
use crate::home::Home;
use crate::transport::{self, LONGEST_SOCKET_PATH};

const RUNTIME_FILE: &str = DaemonFile::RuntimeFile.name();
const SOCKET: &str = DaemonFile::Socket.name();
const SERVING_LOCK: &str = DaemonFile::ServingLock.name();
const START_LOCK: &str = DaemonFile::StartLock.name();
const TOKEN_FILE: &str = DaemonFile::Token.name();
const SERVICE_FILE: &str = DaemonFile::ServiceRecord.name();
const OPS_TOKEN_FILE: &str = DaemonFile::OpsToken.name();
const HTTP_LISTENER_TOKEN_FILE: &str = DaemonFile::HttpListenerToken.name();
const CONSOLE_OPENER: &str = DaemonFile::ConsoleOpener.name();

/// The token in the file at `path`, or `None` when there is none.
fn read_token(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(token) => Ok(Some(token.trim().to_owned()).filter(|token| !token.is_empty())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

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

    /// Removes the runtime file of the Daemon of `pid`, unless it is
    /// another Daemon's by now, and keeps its socket, which the service
    /// manager that activated it owns.
    ///
    /// # Errors
    ///
    /// When it cannot be removed.
    pub fn unpublish_activated(&self, pid: u32) -> io::Result<()> {
        match self.read()? {
            Some(found) if found.pid == pid => remove(&self.runtime_file()),
            _ => Ok(()),
        }
    }

    /// The persistent token's file.
    pub fn token_file(&self) -> PathBuf {
        self.folder.join(TOKEN_FILE)
    }

    /// The persistent token, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// When its file exists but cannot be read.
    pub fn read_token(&self) -> io::Result<Option<String>> {
        read_token(&self.token_file())
    }

    /// The ops token's file.
    pub fn ops_token_file(&self) -> PathBuf {
        self.folder.join(OPS_TOKEN_FILE)
    }

    /// The ops token, made from the OS generator when there is none.
    ///
    /// # Errors
    ///
    /// When its file cannot be read or written, or the OS generator gives
    /// nothing.
    pub fn ops_token(&self) -> io::Result<String> {
        match read_token(&self.ops_token_file())? {
            Some(token) => Ok(token),
            None => self.write_token(OPS_TOKEN_FILE),
        }
    }

    /// The HTTP listener's token's file.
    pub fn http_listener_token_file(&self) -> PathBuf {
        self.folder.join(HTTP_LISTENER_TOKEN_FILE)
    }

    /// The HTTP listener's token, a token of its own (spec #608), made
    /// from the OS generator when there is none.
    ///
    /// # Errors
    ///
    /// When its file cannot be read or written, or the OS generator gives
    /// nothing.
    pub fn http_listener_token(&self) -> io::Result<String> {
        match read_token(&self.http_listener_token_file())? {
            Some(token) => Ok(token),
            None => self.rotate_http_listener_token(),
        }
    }

    /// Replaces the HTTP listener's token with a new one from the OS
    /// generator, readable and writable by its owner alone, in one rename.
    ///
    /// # Errors
    ///
    /// When its file cannot be written, or the OS generator gives nothing.
    pub fn rotate_http_listener_token(&self) -> io::Result<String> {
        self.write_token(HTTP_LISTENER_TOKEN_FILE)
    }

    /// The persistent token, made from the OS generator when there is none.
    ///
    /// # Errors
    ///
    /// When its file cannot be read or written, or the OS generator gives
    /// nothing.
    pub fn persistent_token(&self) -> io::Result<String> {
        match self.read_token()? {
            Some(token) => Ok(token),
            None => self.rotate_token(),
        }
    }

    /// Replaces the persistent token with a new one from the OS generator,
    /// readable and writable by its owner alone, in one rename.
    ///
    /// # Errors
    ///
    /// When its file cannot be written, or the OS generator gives nothing.
    pub fn rotate_token(&self) -> io::Result<String> {
        self.write_token(TOKEN_FILE)
    }

    /// Writes a new token from the OS generator to the file `name`,
    /// readable and writable by its owner alone, in one rename.
    fn write_token(&self, name: &str) -> io::Result<String> {
        let token = new_token().map_err(|error| io::Error::other(error.to_string()))?;
        fs::create_dir_all(&self.folder)?;
        let partial = self.folder.join(format!(".{name}.partial"));
        remove(&partial)?;
        let mut written = owner_only(&partial)?;
        written.write_all(token.as_bytes())?;
        written.sync_all()?;
        drop(written);
        fs::rename(&partial, self.folder.join(name))?;
        Ok(token)
    }

    /// Writes the page that opens the Console's sign-in link `link`,
    /// readable and writable by its owner alone, as the runtime file that
    /// holds the same token is, and answers its path: a browser opened on
    /// it goes on to the link, so the token is never in a command line
    /// another user's process list shows.
    ///
    /// # Errors
    ///
    /// When its file cannot be written.
    pub fn write_console_opener(&self, link: &str) -> io::Result<PathBuf> {
        let escaped = link
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let page = format!(
            "<!doctype html>\n<meta charset=\"utf-8\">\n<meta name=\"referrer\" content=\"no-referrer\">\n<meta http-equiv=\"refresh\" content=\"0;url={escaped}\">\n<title>Console</title>\n<a href=\"{escaped}\" rel=\"noreferrer\">Open the Console</a>\n"
        );
        fs::create_dir_all(&self.folder)?;
        let path = self.folder.join(CONSOLE_OPENER);
        let partial = self.folder.join(".console.html.partial");
        remove(&partial)?;
        let mut written = owner_only(&partial)?;
        written.write_all(page.as_bytes())?;
        written.sync_all()?;
        drop(written);
        fs::rename(&partial, &path)?;
        Ok(path)
    }

    /// Removes the persistent token.
    ///
    /// # Errors
    ///
    /// When its file cannot be removed.
    pub fn remove_token(&self) -> io::Result<()> {
        remove(&self.token_file())
    }

    /// The per-user service `daemon install` registered, or `None` when
    /// none is, or its record is not one.
    ///
    /// # Errors
    ///
    /// When its record exists but cannot be read.
    pub fn service(&self) -> io::Result<Option<InstalledService>> {
        match fs::read(self.folder.join(SERVICE_FILE)) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Records the installed `service`, in one rename.
    ///
    /// # Errors
    ///
    /// When the per-major folder cannot be written.
    pub fn record_service(&self, service: &InstalledService) -> io::Result<()> {
        fs::create_dir_all(&self.folder)?;
        let partial = self.folder.join(".daemon-service.json.partial");
        fs::write(
            &partial,
            serde_json::to_vec_pretty(service).expect("invariant: a service encodes"),
        )?;
        fs::rename(&partial, self.folder.join(SERVICE_FILE))
    }

    /// Forgets the installed service.
    ///
    /// # Errors
    ///
    /// When its record cannot be removed.
    pub fn forget_service(&self) -> io::Result<()> {
        remove(&self.folder.join(SERVICE_FILE))
    }

    /// Where the socket of a service that `manager` runs goes, the same at
    /// every start: in the per-major folder, or, when that path is too long
    /// for the OS, or `manager` is systemd and the path holds a byte it
    /// escapes, in `/tmp/goodfetch-daemon-<digest>`, the user's own, mode
    /// `0700`. systemd writes each listening socket's path with C escapes
    /// when its manager reloads, and reads back the escaped text as the
    /// path, so that socket would stop listening at the first reload any
    /// program asks for (#1781).
    ///
    /// # Errors
    ///
    /// When that folder cannot be made, or is not the user's own.
    pub fn service_socket(&self, manager: ServiceManager) -> io::Result<PathBuf> {
        let socket = self.socket();
        let kept = manager != ServiceManager::Systemd
            || !socket
                .as_os_str()
                .as_encoded_bytes()
                .iter()
                .any(|&byte| systemd_escapes(byte));
        if kept && socket.as_os_str().len() <= LONGEST_SOCKET_PATH {
            return Ok(socket);
        }
        fs::create_dir_all(&self.folder)?;
        let digest = Sha256::digest(self.folder.as_os_str().as_encoded_bytes());
        let name = digest[..8]
            .iter()
            .fold(String::from("goodfetch-daemon-"), |mut name, byte| {
                let _ = write!(name, "{byte:02x}");
                name
            });
        let folder = Path::new("/tmp").join(name);
        own_folder(&folder, &self.folder)?;
        Ok(folder.join(SOCKET))
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
    /// folder, or in `/tmp` when that one is too deep as well, which lives
    /// as long as the answered guard.
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
        let folder = transport::socket_folder("goodfetch-daemon-", SOCKET)?;
        Ok((folder.path().join(SOCKET), Some(folder)))
    }
}

/// Whether systemd's C escaping (`cescape`) changes `byte`: a control or
/// non-ASCII byte, `\`, `"` or `'`.
fn systemd_escapes(byte: u8) -> bool {
    !(b' '..=b'~').contains(&byte) || matches!(byte, b'\\' | b'"' | b'\'')
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

/// Makes `folder` the user's own, mode `0700`, or checks that it is: owned
/// by whoever owns `reference`, and readable by no one else.
#[cfg(unix)]
fn own_folder(folder: &Path, reference: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
    match fs::DirBuilder::new().mode(0o700).create(folder) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let found = fs::symlink_metadata(folder)?;
    let owner = fs::metadata(reference)?.uid();
    if !found.is_dir() || found.uid() != owner || found.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} is not a folder of this user's own, mode 0700",
                folder.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn own_folder(folder: &Path, _reference: &Path) -> io::Result<()> {
    fs::create_dir_all(folder)
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
