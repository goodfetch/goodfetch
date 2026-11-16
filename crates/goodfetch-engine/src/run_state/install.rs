//! The per-install layout's database (ADR 0002): one SQLite database,
//! `run-state.sqlite` in the per-major folder, holding the run file of
//! every Run of the install and major, which the Daemon alone writes.
//!
//! Each Run keeps its run file, byte for byte the file the per-run layout
//! keeps in its run folder: the database stores the bytes of every file
//! SQLite opens under the run folders, the run file and its WAL, in chunks,
//! and a VFS registered under one name serves them to the run state's own
//! connections at the paths the per-run layout gives them. So the run
//! state's writer, its readers, Checkpoints and their restores work
//! unchanged, and export, import and Checkpoint give the same per-run file
//! in both layouts.
//!
//! Each write of a run file is one transaction of the database, which
//! commits without syncing; a sync of the run file commits with
//! `synchronous=FULL`, which syncs the database's WAL and so every write
//! before it. A process crash then keeps every write, and a power cut keeps
//! a prefix of them up to at least the last sync: what a disk promises
//! SQLite, so the run file's own `synchronous` gives each Run its
//! durability (`run_state.durability`) as in the per-run layout. The
//! database itself opens through the VFS the run state's options name, the
//! crash VFS in tests (ADR 0060). Locks and shared memory live in this
//! process, the one writer's.
#![expect(
    clippy::disallowed_types,
    reason = "SQLite calls a VFS from any connection's thread: one Mutex guards one install database for the length of one VFS call, and one the registry, never both at once, so there is no lock order to test under loom"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::path::{Component, Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::RUN_STATE;
use rusqlite::{Connection, ErrorCode, OptionalExtension as _};
use sqlite_plugin::flags::{AccessFlags, CreateMode, LockLevel, OpenMode, OpenOpts, ShmLockMode};
use sqlite_plugin::vars;
use sqlite_plugin::vfs::{RegisterOpts, Vfs, VfsHandle, VfsResult, register_static};

use super::{RUN_FILE, RunStateError, RunStateOptions, sqlite_error};
use crate::home::{Home, RunStateLocation};
use crate::sqlite_store::{
    Access, Durability, File, Folder, FullFsync, OpenError, Opening, Schema, Synchronous, Wait,
    WalOnClose, run_cached,
};

/// The install database's file name, in the per-major folder.
pub const INSTALL_FILE: &str = "run-state.sqlite";

/// The name the VFS that serves run files from install databases is
/// registered under, once per process.
const VFS_NAME: &str = "goodfetch-install";

/// The size of a chunk of a run file's bytes.
const CHUNK: usize = 4096;

/// The number of shared-memory lock slots SQLite uses (`SQLITE_SHM_NLOCK`).
const SHM_LOCKS: usize = 8;

/// The names of a run folder's files, between `?1`, the folder's name and
/// its `/`, and `?2`, the same with `/` raised to `0`, the next byte: a
/// range on the primary key, which SQLite searches, never a scan of every
/// Run's files, and no `LIKE`, whose `_` a Run ID may hold.
const ERASED: &str = "name >= ?1 AND name < ?2";

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS files (name TEXT NOT NULL PRIMARY KEY, size INTEGER NOT NULL) STRICT",
    "CREATE TABLE IF NOT EXISTS chunks (
        name TEXT NOT NULL,
        number INTEGER NOT NULL,
        data BLOB NOT NULL,
        PRIMARY KEY (name, number)
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS syncs (id INTEGER NOT NULL PRIMARY KEY, count INTEGER NOT NULL) STRICT",
    "INSERT INTO syncs (id, count) VALUES (1, 0) ON CONFLICT (id) DO NOTHING",
];

/// An open install database, serving the run files under its run folders
/// through [`InstallDatabase::vfs`] until it is dropped.
#[derive(Debug)]
pub struct InstallDatabase {
    runs: PathBuf,
    install: Arc<Install>,
}

impl InstallDatabase {
    /// Opens the install database at `path`, through the VFS `options`
    /// name, creating it when it does not exist, to serve the run files of
    /// the run folders under `runs`. It blocks: run it under a blocking
    /// policy.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be opened, or another install database of
    /// this process already serves `runs`; `Format` when it is of a format
    /// this engine cannot read.
    pub fn open(
        path: &Path,
        runs: &Path,
        options: &RunStateOptions,
    ) -> Result<Self, RunStateError> {
        registered().map_err(|code| RunStateError::Sqlite {
            path: path.to_owned(),
            message: format!("SQLite refused the install database's VFS: code {code}"),
        })?;
        let connection = open_database(path, options.vfs.as_deref())?;
        let install = Arc::new(Install {
            path: path.to_owned(),
            state: Mutex::new(State {
                connection,
                locks: BTreeMap::new(),
                shared_memory: BTreeMap::new(),
                next_id: 0,
                unsynced: false,
            }),
        });
        let mut installs = lock(installs());
        if installs.contains_key(runs) {
            return Err(RunStateError::Sqlite {
                path: path.to_owned(),
                message: format!(
                    "another install database of this process serves `{}`",
                    runs.display()
                ),
            });
        }
        installs.insert(runs.to_owned(), Arc::clone(&install));
        Ok(Self {
            runs: runs.to_owned(),
            install,
        })
    }

    /// Opens the install database of `home`'s major of `version`,
    /// [`INSTALL_FILE`] in its per-major folder, serving the run folders
    /// `location` names, through the VFS `options` name. It blocks: run it
    /// under a blocking policy.
    ///
    /// # Errors
    ///
    /// As [`InstallDatabase::open`].
    pub fn of(
        home: &Home,
        version: &ProductVersion,
        location: &RunStateLocation,
        options: &RunStateOptions,
    ) -> Result<Self, RunStateError> {
        Self::open(
            &home.per_major(version).join(INSTALL_FILE),
            &location.folder(home, version),
            options,
        )
    }

    /// The folder holding the database: the filesystem of every Run's
    /// run state in the per-install layout (ADR 0122).
    pub fn folder(&self) -> &Path {
        self.install
            .path
            .parent()
            .expect("invariant: the install database's path names a file in a folder")
    }

    /// The name of the VFS the run state opens its run files through, as
    /// [`RunStateOptions::vfs`].
    pub fn vfs(&self) -> &'static str {
        VFS_NAME
    }

    /// The Run IDs whose run folder holds a run file.
    ///
    /// # Errors
    ///
    /// `Sqlite` when the database cannot be read.
    pub fn runs(&self) -> Result<Vec<String>, RunStateError> {
        let state = lock(&self.install.state);
        let mut statement = state
            .connection
            .prepare("SELECT name FROM files WHERE name LIKE '%/' || ?1 ORDER BY name")
            .map_err(|error| sqlite_error(&self.install.path, &error))?;
        let names = statement
            .query_map([RUN_FILE], |row| row.get::<_, String>(0))
            .and_then(Iterator::collect::<rusqlite::Result<Vec<String>>>)
            .map_err(|error| sqlite_error(&self.install.path, &error))?;
        Ok(names
            .into_iter()
            .filter_map(|name| name.split_once('/').map(|(run, _)| run.to_owned()))
            .collect())
    }

    /// Deletes every file of the run folder `run_id`, its run file and its
    /// WAL, and makes the deletion durable, then truncates the database's
    /// WAL with `wal_checkpoint(TRUNCATE)`: the database deletes with
    /// `secure_delete` on, so neither it nor its `-wal` and `-shm` files
    /// keep their bytes (ADR 0002). Answers whether it held any. It blocks:
    /// run it under a blocking policy.
    ///
    /// # Errors
    ///
    /// `Sqlite` when a file of the run folder is open, or the database
    /// cannot be written or its WAL truncated.
    pub fn erase(&self, run_id: &str) -> Result<bool, RunStateError> {
        let failed = |error: rusqlite::Error| sqlite_error(&self.install.path, &error);
        let prefix = format!("{run_id}/");
        let mut state = lock(&self.install.state);
        if state
            .locks
            .keys()
            .chain(state.shared_memory.keys())
            .any(|name| name.starts_with(&prefix))
        {
            return Err(RunStateError::Sqlite {
                path: self.install.path.clone(),
                message: format!("a file of the run folder `{run_id}` is open"),
            });
        }
        let after = format!("{run_id}0");
        let mut erased = false;
        state
            .commit(|state| {
                state
                    .connection
                    .prepare_cached(&format!("DELETE FROM chunks WHERE {ERASED}"))?
                    .execute([&prefix, &after])?;
                erased = state
                    .connection
                    .prepare_cached(&format!("DELETE FROM files WHERE {ERASED}"))?
                    .execute([&prefix, &after])?
                    > 0;
                Ok(())
            })
            .map_err(failed)?;
        state.sync().map_err(failed)?;
        let busy: i64 = state
            .connection
            .prepare_cached("PRAGMA wal_checkpoint(TRUNCATE)")
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map_err(failed)?;
        if busy != 0 {
            return Err(RunStateError::Sqlite {
                path: self.install.path.clone(),
                message: "a reader held the WAL, so it was not truncated".to_owned(),
            });
        }
        Ok(erased)
    }
}

impl Drop for InstallDatabase {
    fn drop(&mut self) {
        let mut installs = lock(installs());
        if installs
            .get(&self.runs)
            .is_some_and(|install| Arc::ptr_eq(install, &self.install))
        {
            installs.remove(&self.runs);
        }
    }
}

/// Opens the database itself, in WAL mode with commits that do not sync,
/// and checks its format.
fn open_database(path: &Path, vfs: Option<&str>) -> Result<Connection, RunStateError> {
    let failed = |error: rusqlite::Error| sqlite_error(path, &error);
    let connection = File::new(path, vfs)
        .open(&Opening {
            folder: if vfs.is_none() {
                Folder::Make
            } else {
                Folder::Exists
            },
            access: Access::Create,
            wait: Wait::Switch(super::SWITCH_TIMEOUT),
            durability: Durability {
                synchronous: Synchronous::Normal,
                opening: Synchronous::Normal,
                full_fsync: FullFsync::CommitsAndCheckpoints,
                wal_on_close: WalOnClose::Checkpointed,
            },
            counting: None,
            schema: Some(Schema {
                format: &RUN_STATE,
                statements: &[SCHEMA],
            }),
        })
        .map_err(|error| match error {
            OpenError::Format(refusal) => refusal.into(),
            other => sqlite_error(path, &other),
        })?;
    connection.set_prepared_statement_cache_capacity(super::STATEMENTS);
    // Every page a Run's rewritten or deleted chunks free is zeroed, so a
    // discarded Run's bytes are kept nowhere in the database (ADR 0002).
    connection
        .pragma_update(None, "secure_delete", "ON")
        .map_err(failed)?;
    Ok(connection)
}

/// Registers the VFS once per process; its result stays.
fn registered() -> Result<(), i32> {
    static REGISTERED: OnceLock<Result<(), i32>> = OnceLock::new();
    *REGISTERED.get_or_init(|| {
        let name = CString::new(VFS_NAME).expect("invariant: the VFS name holds no NUL");
        register_static(
            name,
            InstallVfs,
            RegisterOpts {
                make_default: false,
            },
        )
        .map(drop)
    })
}

/// The install databases open in this process, by the folder of run
/// folders each serves.
fn installs() -> &'static Mutex<BTreeMap<PathBuf, Arc<Install>>> {
    static INSTALLS: Mutex<BTreeMap<PathBuf, Arc<Install>>> = Mutex::new(BTreeMap::new());
    &INSTALLS
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The install database `path` is under, and the file's name in it: its
/// path below the run folders, `/`-separated; none when no install
/// database of this process serves it.
fn find(path: &str) -> Option<(Arc<Install>, String)> {
    let path = Path::new(path);
    let installs = lock(installs());
    installs.iter().find_map(|(runs, install)| {
        let below = path.strip_prefix(runs).ok()?;
        let parts: Option<Vec<&str>> = below
            .components()
            .map(|component| match component {
                Component::Normal(part) => part.to_str(),
                _ => None,
            })
            .collect();
        let parts = parts.filter(|parts| !parts.is_empty())?;
        Some((Arc::clone(install), parts.join("/")))
    })
}

/// One install database, as the VFS reaches it.
#[derive(Debug)]
struct Install {
    path: PathBuf,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    connection: Connection,
    /// The lock each open handle holds on each file.
    locks: BTreeMap<String, BTreeMap<u64, LockLevel>>,
    /// The shared memory of each run file.
    shared_memory: BTreeMap<String, SharedMemory>,
    next_id: u64,
    /// Whether a write committed since the last sync.
    unsynced: bool,
}

/// What an I/O of the database failed with, for SQLite: `SQLITE_FULL` on a
/// full disk, else `code`.
fn io_error(code: i32) -> impl Fn(rusqlite::Error) -> i32 {
    move |error| match error.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => vars::SQLITE_FULL,
        _ => code,
    }
}

impl State {
    fn size(&self, name: &str) -> rusqlite::Result<Option<usize>> {
        self.connection
            .prepare("SELECT size FROM files WHERE name = ?1")?
            .query_row([name], |row| row.get::<_, i64>(0))
            .optional()
            .map(|size| size.map(|size| usize::try_from(size).unwrap_or(usize::MAX)))
    }

    fn chunk(&self, name: &str, number: usize) -> rusqlite::Result<Vec<u8>> {
        let found: Option<Vec<u8>> = self
            .connection
            .prepare("SELECT data FROM chunks WHERE name = ?1 AND number = ?2")?
            .query_row((name, to_i64(number)), |row| row.get(0))
            .optional()?;
        let mut chunk = found.unwrap_or_default();
        chunk.resize(CHUNK, 0);
        Ok(chunk)
    }

    fn put_chunk(&self, name: &str, number: usize, data: &[u8]) -> rusqlite::Result<()> {
        self.connection
            .prepare(
                "INSERT INTO chunks (name, number, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (name, number) DO UPDATE SET data = excluded.data",
            )?
            .execute((name, to_i64(number), data))
            .map(drop)
    }

    fn set_size(&self, name: &str, size: usize) -> rusqlite::Result<()> {
        self.connection
            .prepare(
                "INSERT INTO files (name, size) VALUES (?1, ?2)
                 ON CONFLICT (name) DO UPDATE SET size = excluded.size",
            )?
            .execute((name, to_i64(size)))
            .map(drop)
    }

    /// Runs `work` in one transaction of the database, which commits
    /// without syncing.
    fn commit(&mut self, work: impl FnOnce(&Self) -> rusqlite::Result<()>) -> rusqlite::Result<()> {
        let done = (|| {
            super::run_cached(&self.connection, "BEGIN IMMEDIATE")?;
            match work(self) {
                Ok(()) => super::run_cached(&self.connection, "COMMIT"),
                Err(error) => {
                    let _ = super::run_cached(&self.connection, "ROLLBACK");
                    Err(error)
                }
            }
        })();
        if done.is_ok() {
            self.unsynced = true;
        }
        done
    }

    fn write(&self, name: &str, offset: usize, data: &[u8]) -> rusqlite::Result<()> {
        let size = self.size(name)?.unwrap_or(0);
        let end = offset + data.len();
        let mut at = offset;
        while at < end {
            let number = at / CHUNK;
            let start = at % CHUNK;
            let count = (CHUNK - start).min(end - at);
            let mut chunk = self.chunk(name, number)?;
            chunk[start..start + count].copy_from_slice(&data[at - offset..at - offset + count]);
            self.put_chunk(name, number, &chunk)?;
            at += count;
        }
        self.set_size(name, size.max(end))
    }

    fn truncate(&self, name: &str, size: usize) -> rusqlite::Result<()> {
        let kept = size.div_ceil(CHUNK);
        self.connection
            .prepare("DELETE FROM chunks WHERE name = ?1 AND number >= ?2")?
            .execute((name, to_i64(kept)))?;
        if !size.is_multiple_of(CHUNK) {
            let number = size / CHUNK;
            let mut chunk = self.chunk(name, number)?;
            chunk[size % CHUNK..].fill(0);
            self.put_chunk(name, number, &chunk)?;
        }
        self.set_size(name, size)
    }

    fn read(&self, name: &str, offset: usize, data: &mut [u8]) -> rusqlite::Result<usize> {
        let size = self.size(name)?.unwrap_or(0);
        let end = size.min(offset + data.len());
        let mut at = offset;
        while at < end {
            let number = at / CHUNK;
            let start = at % CHUNK;
            let count = (CHUNK - start).min(end - at);
            let chunk = self.chunk(name, number)?;
            data[at - offset..at - offset + count].copy_from_slice(&chunk[start..start + count]);
            at += count;
        }
        Ok(end.saturating_sub(offset))
    }

    fn delete(&self, name: &str) -> rusqlite::Result<bool> {
        self.connection
            .prepare("DELETE FROM chunks WHERE name = ?1")?
            .execute([name])?;
        Ok(self
            .connection
            .prepare("DELETE FROM files WHERE name = ?1")?
            .execute([name])?
            > 0)
    }

    /// Makes every write so far durable: a commit with
    /// `synchronous=FULL` syncs the database's WAL, which holds them all.
    fn sync(&mut self) -> rusqlite::Result<()> {
        if !self.unsynced {
            return Ok(());
        }
        let synced = (|| {
            self.connection.pragma_update(None, "synchronous", "FULL")?;
            let committed = run_cached(
                &self.connection,
                "UPDATE syncs SET count = count + 1 WHERE id = 1",
            );
            self.connection
                .pragma_update(None, "synchronous", "NORMAL")?;
            committed
        })();
        if synced.is_ok() {
            self.unsynced = false;
        }
        synced
    }

    fn other_locks<'a>(&'a self, handle: &'a Handle) -> impl Iterator<Item = LockLevel> + 'a {
        self.locks
            .get(&handle.name)
            .into_iter()
            .flatten()
            .filter(move |(id, _)| **id != handle.id)
            .map(|(_, level)| *level)
    }

    fn set_lock(&mut self, handle: &Handle, level: LockLevel) {
        let locks = self.locks.entry(handle.name.clone()).or_default();
        if level == LockLevel::Unlocked {
            locks.remove(&handle.id);
        } else {
            locks.insert(handle.id, level);
        }
        if locks.is_empty() {
            self.locks.remove(&handle.name);
        }
    }

    fn lock_of(&self, handle: &Handle) -> LockLevel {
        self.locks
            .get(&handle.name)
            .and_then(|locks| locks.get(&handle.id))
            .copied()
            .unwrap_or(LockLevel::Unlocked)
    }
}

fn to_i64(number: usize) -> i64 {
    i64::try_from(number).unwrap_or(i64::MAX)
}

/// A run file's shared memory: its regions and its lock slots.
#[derive(Debug, Default)]
struct SharedMemory {
    regions: Vec<Region>,
    shared: [BTreeSet<u64>; SHM_LOCKS],
    exclusive: [Option<u64>; SHM_LOCKS],
    mapped_by: BTreeSet<u64>,
}

/// One region of shared memory. SQLite reads and writes it through the
/// address it was given, so the memory is never borrowed again from Rust
/// once that address is taken.
#[derive(Debug)]
struct Region {
    _memory: Vec<u8>,
    address: usize,
}

impl Region {
    fn new(size: usize) -> Self {
        let mut memory = vec![0_u8; size];
        let address = memory.as_mut_ptr().expose_provenance();
        Self {
            _memory: memory,
            address,
        }
    }

    fn pointer(&self) -> Option<NonNull<u8>> {
        NonNull::new(std::ptr::with_exposed_provenance_mut(self.address))
    }
}

/// A file SQLite opened through the VFS.
struct Handle {
    id: u64,
    install: Arc<Install>,
    name: String,
    readonly: bool,
}

impl VfsHandle for Handle {
    fn readonly(&self) -> bool {
        self.readonly
    }

    fn in_memory(&self) -> bool {
        false
    }
}

/// The VFS SQLite calls: it finds each path's install database.
struct InstallVfs;

impl Vfs for InstallVfs {
    type Handle = Handle;

    fn open(&self, path: Option<&str>, opts: OpenOpts) -> VfsResult<Self::Handle> {
        // A temporary file has no path: the run state keeps none.
        let (install, name) = path.and_then(find).ok_or(vars::SQLITE_CANTOPEN)?;
        let mut state = lock(&install.state);
        let mode = opts.mode();
        let exists = state
            .size(&name)
            .map_err(io_error(vars::SQLITE_CANTOPEN))?
            .is_some();
        match mode {
            OpenMode::ReadOnly
            | OpenMode::ReadWrite {
                create: CreateMode::None,
            } if !exists => return Err(vars::SQLITE_CANTOPEN),
            OpenMode::ReadWrite {
                create: CreateMode::MustCreate,
            } if exists => return Err(vars::SQLITE_CANTOPEN),
            _ => {}
        }
        if !exists {
            state
                .commit(|state| state.set_size(&name, 0))
                .map_err(io_error(vars::SQLITE_CANTOPEN))?;
        }
        let id = state.next_id;
        state.next_id += 1;
        drop(state);
        Ok(Handle {
            id,
            install,
            name,
            readonly: mode.is_readonly(),
        })
    }

    fn delete(&self, path: &str) -> VfsResult<()> {
        let (install, name) = find(path).ok_or(vars::SQLITE_IOERR_DELETE)?;
        let mut state = lock(&install.state);
        let mut deleted = false;
        state
            .commit(|state| {
                deleted = state.delete(&name)?;
                Ok(())
            })
            .map_err(io_error(vars::SQLITE_IOERR_DELETE))?;
        if deleted {
            Ok(())
        } else {
            Err(vars::SQLITE_IOERR_DELETE_NOENT)
        }
    }

    fn access(&self, path: &str, _flags: AccessFlags) -> VfsResult<bool> {
        let Some((install, name)) = find(path) else {
            return Ok(false);
        };
        let state = lock(&install.state);
        state
            .size(&name)
            .map(|size| size.is_some())
            .map_err(io_error(vars::SQLITE_IOERR_ACCESS))
    }

    fn file_size(&self, handle: &mut Self::Handle) -> VfsResult<usize> {
        let state = lock(&handle.install.state);
        state
            .size(&handle.name)
            .map(Option::unwrap_or_default)
            .map_err(io_error(vars::SQLITE_IOERR_FSTAT))
    }

    fn truncate(&self, handle: &mut Self::Handle, size: usize) -> VfsResult<()> {
        let mut state = lock(&handle.install.state);
        let name = handle.name.clone();
        state
            .commit(|state| state.truncate(&name, size))
            .map_err(io_error(vars::SQLITE_IOERR_TRUNCATE))
    }

    fn write(&self, handle: &mut Self::Handle, offset: usize, data: &[u8]) -> VfsResult<usize> {
        let mut state = lock(&handle.install.state);
        let name = handle.name.clone();
        state
            .commit(|state| state.write(&name, offset, data))
            .map_err(io_error(vars::SQLITE_IOERR_WRITE))?;
        Ok(data.len())
    }

    fn read(&self, handle: &mut Self::Handle, offset: usize, data: &mut [u8]) -> VfsResult<usize> {
        let state = lock(&handle.install.state);
        state
            .read(&handle.name, offset, data)
            .map_err(io_error(vars::SQLITE_IOERR_READ))
    }

    fn sync(&self, handle: &mut Self::Handle) -> VfsResult<()> {
        let mut state = lock(&handle.install.state);
        state.sync().map_err(io_error(vars::SQLITE_IOERR_FSYNC))
    }

    fn lock(&self, handle: &mut Self::Handle, level: LockLevel) -> VfsResult<()> {
        let mut state = lock(&handle.install.state);
        if state.lock_of(handle) >= level {
            return Ok(());
        }
        let strongest_other = state.other_locks(handle).max();
        let busy = match level {
            LockLevel::Unlocked => false,
            LockLevel::Shared => strongest_other >= Some(LockLevel::Pending),
            LockLevel::Reserved => strongest_other >= Some(LockLevel::Reserved),
            LockLevel::Pending | LockLevel::Exclusive => {
                if strongest_other >= Some(LockLevel::Pending) {
                    return Err(vars::SQLITE_BUSY);
                }
                // A writer waiting for readers holds PENDING, so no new
                // reader starts.
                state.set_lock(handle, LockLevel::Pending);
                level == LockLevel::Exclusive && strongest_other >= Some(LockLevel::Shared)
            }
        };
        if busy {
            return Err(vars::SQLITE_BUSY);
        }
        state.set_lock(handle, level);
        Ok(())
    }

    fn unlock(&self, handle: &mut Self::Handle, level: LockLevel) -> VfsResult<()> {
        let mut state = lock(&handle.install.state);
        if state.lock_of(handle) > level {
            state.set_lock(handle, level);
        }
        Ok(())
    }

    fn check_reserved_lock(&self, handle: &mut Self::Handle) -> VfsResult<bool> {
        let state = lock(&handle.install.state);
        Ok(state
            .locks
            .get(&handle.name)
            .into_iter()
            .flatten()
            .any(|(_, level)| *level >= LockLevel::Reserved))
    }

    fn close(&self, handle: Self::Handle) -> VfsResult<()> {
        let mut state = lock(&handle.install.state);
        state.set_lock(&handle, LockLevel::Unlocked);
        Ok(())
    }

    fn device_characteristics(&self, _handle: &mut Self::Handle) -> VfsResult<i32> {
        // None claimed, as of a disk that may tear or reorder writes.
        Ok(0)
    }

    fn shm_map(
        &self,
        handle: &mut Self::Handle,
        region_idx: usize,
        region_size: usize,
        extend: bool,
    ) -> VfsResult<Option<NonNull<u8>>> {
        let mut state = lock(&handle.install.state);
        let memory = state.shared_memory.entry(handle.name.clone()).or_default();
        if memory.regions.len() <= region_idx {
            if !extend {
                return Ok(None);
            }
            memory
                .regions
                .resize_with(region_idx + 1, || Region::new(region_size));
        }
        memory.mapped_by.insert(handle.id);
        Ok(memory.regions[region_idx].pointer())
    }

    fn shm_lock(
        &self,
        handle: &mut Self::Handle,
        offset: u32,
        count: u32,
        mode: ShmLockMode,
    ) -> VfsResult<()> {
        let mut state = lock(&handle.install.state);
        let memory = state
            .shared_memory
            .get_mut(&handle.name)
            .ok_or(vars::SQLITE_IOERR_SHMLOCK)?;
        let start = usize::try_from(offset).map_err(|_| vars::SQLITE_IOERR_SHMLOCK)?;
        let end = start + usize::try_from(count).map_err(|_| vars::SQLITE_IOERR_SHMLOCK)?;
        let slots = start..end.min(SHM_LOCKS);
        let id = handle.id;
        match mode {
            ShmLockMode::LockShared => {
                if slots
                    .clone()
                    .any(|slot| memory.exclusive[slot].is_some_and(|owner| owner != id))
                {
                    return Err(vars::SQLITE_BUSY);
                }
                for slot in slots {
                    memory.shared[slot].insert(id);
                }
            }
            ShmLockMode::LockExclusive => {
                if slots.clone().any(|slot| {
                    memory.exclusive[slot].is_some_and(|owner| owner != id)
                        || memory.shared[slot].iter().any(|owner| *owner != id)
                }) {
                    return Err(vars::SQLITE_BUSY);
                }
                for slot in slots {
                    memory.exclusive[slot] = Some(id);
                }
            }
            ShmLockMode::UnlockShared => {
                for slot in slots {
                    memory.shared[slot].remove(&id);
                }
            }
            ShmLockMode::UnlockExclusive => {
                for slot in slots {
                    if memory.exclusive[slot] == Some(id) {
                        memory.exclusive[slot] = None;
                    }
                }
            }
        }
        Ok(())
    }

    fn shm_barrier(&self, _handle: &mut Self::Handle) {
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    }

    fn shm_unmap(&self, handle: &mut Self::Handle, _delete: bool) -> VfsResult<()> {
        // The memory goes with its last mapper, never before: another
        // connection may still read it through its address.
        let mut state = lock(&handle.install.state);
        let Some(memory) = state.shared_memory.get_mut(&handle.name) else {
            return Ok(());
        };
        memory.mapped_by.remove(&handle.id);
        for slot in 0..SHM_LOCKS {
            memory.shared[slot].remove(&handle.id);
            if memory.exclusive[slot] == Some(handle.id) {
                memory.exclusive[slot] = None;
            }
        }
        if memory.mapped_by.is_empty() {
            state.shared_memory.remove(&handle.name);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
