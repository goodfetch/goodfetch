//! The crash VFS (ADR 0060), modelled on SQLite's own: an in-memory SQLite
//! VFS, built on `sqlite-plugin` and registered by name with the SQLite
//! `rusqlite` bundles, so the code under test opens its database with the
//! VFS name as a value and production passes none. It holds every write and
//! truncation until a sync of its file; a crash then keeps them (a process
//! crash) or drops, tears or reorders them from the seed (a power cut), and
//! refuses every later I/O through a handle the crashed instance opened.
//! Opening, deleting and looking up a file belong to the next instance, as a
//! restarted process's would. It also injects I/O errors and a full disk at
//! chosen operations.
//!
//! Files live in memory under the path SQLite opens them with, so a test
//! touches no real file and behaves alike on every OS. Creating and deleting
//! a file take effect at once and survive any crash. The locks and shared
//! memory serve several connections of one process, so WAL databases work.
//! The VFS claims no device characteristic: SQLite then expects any sector
//! to tear and writes to land out of order.
#![expect(
    clippy::disallowed_types,
    reason = "SQLite calls a VFS from any connection's thread: one Mutex guards the whole \
              in-memory disk for the length of one VFS call, with no lock order to test under loom"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fmt;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chacha20::ChaCha20Rng;
use chacha20::rand_core::Rng as _;
use rusqlite::{Connection, ErrorCode, OpenFlags};
use sqlite_plugin::flags::{AccessFlags, CreateMode, LockLevel, OpenMode, OpenOpts, ShmLockMode};
use sqlite_plugin::vars;
use sqlite_plugin::vfs::{RegisterOpts, Vfs, VfsHandle, VfsResult, register_static};

use crate::seeded;

/// A crash VFS registered under its name. Clones share one in-memory disk.
#[derive(Clone)]
pub struct CrashVfs {
    name: String,
    disk: Arc<Mutex<Disk>>,
}

/// The name alone: the disk is behind a lock SQLite may hold, and its
/// shared memory is written through raw addresses, so `Debug` reads
/// neither.
impl fmt::Debug for CrashVfs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CrashVfs")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl CrashVfs {
    /// Registers a crash VFS under `name`, its later draws taken from
    /// `seed`. The VFS stays registered until the process ends.
    ///
    /// # Errors
    ///
    /// [`CrashVfsError::NameTaken`] if this process already registered a
    /// crash VFS under `name`, [`CrashVfsError::NameInvalid`] if `name`
    /// holds a NUL, and [`CrashVfsError::Register`] if SQLite refuses it.
    pub fn register(name: &str, seed: u64) -> Result<Self, CrashVfsError> {
        let c_name = CString::new(name).map_err(|_| CrashVfsError::NameInvalid {
            name: name.to_owned(),
        })?;
        let mut names = registered_names();
        if !names.insert(name.to_owned()) {
            return Err(CrashVfsError::NameTaken {
                name: name.to_owned(),
            });
        }
        let disk = Arc::new(Mutex::new(Disk::new(seed)));
        let vfs = SqliteVfs {
            disk: Arc::clone(&disk),
        };
        register_static(
            c_name,
            vfs,
            RegisterOpts {
                make_default: false,
            },
        )
        .map_err(|code| {
            names.remove(name);
            CrashVfsError::Register { code }
        })?;
        Ok(Self {
            name: name.to_owned(),
            disk,
        })
    }

    /// The name the code under test opens its database with.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How many I/O operations the VFS has run, which is the index of the
    /// next one.
    ///
    /// The operations counted are opens, deletes, existence checks, reads,
    /// writes, truncations, syncs and size queries; locks and shared memory
    /// are not. One workload runs the same operations in the same order
    /// each time, so an index found once names the same operation again.
    pub fn operations(&self) -> u64 {
        lock(&self.disk).operations
    }

    /// Injects `fault` at the I/O operation numbered `index`, from 0 at
    /// registration (see [`CrashVfs::operations`]). A later call for the
    /// same index replaces the fault.
    pub fn fail_at(&self, index: u64, fault: IoFault) {
        lock(&self.disk).faults.insert(index, fault);
    }

    /// Frees the disk an [`IoFault::Full`] filled, as a person freeing space
    /// does: writes that grow a file succeed again.
    pub fn free_space(&self) {
        lock(&self.disk).full = false;
    }

    /// Crashes the instance: every file the code under test opened before
    /// now fails each I/O, and a database opened afterwards (the next
    /// process, as [`after_crash`] opens it) sees what the crash left on the
    /// disk. A process crash keeps every unsynced write; a power cut drops,
    /// tears or reorders each file's unsynced writes as the seed chooses,
    /// and always loses at least one of them. Returns what landed.
    pub fn crash(&self, kind: CrashKind) -> Crash {
        lock(&self.disk).crash(kind)
    }
}

/// A fault injected at one I/O operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoFault {
    /// The operation fails with SQLite's I/O error for it
    /// (`SQLITE_IOERR_WRITE` for a write, `SQLITE_IOERR_FSYNC` for a sync,
    /// `SQLITE_CANTOPEN` for an open).
    Error,
    /// The disk is full from this operation on, across crashes, for as long
    /// as the VFS lives: every write or truncation that would grow a file
    /// fails with `SQLITE_FULL`, and the rest still succeed.
    Full,
}

/// How the instance crashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashKind {
    /// The process dies; the OS still writes what it was given.
    Process,
    /// The machine loses power: only what was synced is sure to be there.
    PowerCut,
}

/// What a crash left: for each file with unsynced changes (writes and
/// truncations), in path order, which of them landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crash {
    /// The kind of crash.
    pub kind: CrashKind,
    /// The files that had unsynced changes.
    pub files: Vec<CrashedFile>,
}

/// One file's unsynced changes at a crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashedFile {
    /// The path SQLite opened it with.
    pub path: String,
    /// How many changes were unsynced.
    pub unsynced: usize,
    /// Which of them landed.
    pub landed: Landed,
}

/// Which unsynced changes of a file landed, numbered from 0 in the order
/// they were made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landed {
    /// All of them (a process crash).
    All,
    /// None of them.
    None,
    /// The first `whole` landed, then the first `bytes` bytes of the next
    /// (none of a truncation), then nothing.
    Torn {
        /// How many changes landed whole.
        whole: usize,
        /// How many bytes of the torn write landed.
        bytes: usize,
    },
    /// These landed, out of order with the ones that did not.
    Reordered {
        /// The changes that landed.
        changes: Vec<usize>,
    },
}

/// Opens the database at `path` again through `vfs`, as the next process
/// would after a crash, and checks it with `PRAGMA integrity_check`. The
/// connection it returns is the caller's, to check its acknowledged commits
/// against its reference model.
///
/// # Errors
///
/// [`AfterCrashError::Damaged`] with SQLite's findings if the database is
/// damaged: the check reports a problem, or SQLite refuses to read the file
/// as a database (`SQLITE_CORRUPT`, `SQLITE_NOTADB`).
/// [`AfterCrashError::Open`] if it cannot be opened for another reason (a
/// missing file included), and [`AfterCrashError::Check`] if the check
/// cannot run for another reason.
pub fn after_crash(vfs: &CrashVfs, path: &str) -> Result<Connection, AfterCrashError> {
    let connection = Connection::open_with_flags_and_vfs(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        vfs.name(),
    )
    .map_err(|error| damaged(error).unwrap_or_else(AfterCrashError::Open))?;
    let findings = connection
        .prepare("PRAGMA integrity_check")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| damaged(error).unwrap_or_else(AfterCrashError::Check))?;
    if findings != ["ok"] {
        return Err(AfterCrashError::Damaged { findings });
    }
    Ok(connection)
}

/// The damage `error` reports, or the error itself when it reports none.
fn damaged(error: rusqlite::Error) -> Result<AfterCrashError, rusqlite::Error> {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
            Ok(AfterCrashError::Damaged {
                findings: vec![error.to_string()],
            })
        }
        _ => Err(error),
    }
}

/// Why a database failed [`after_crash`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AfterCrashError {
    /// The database is damaged.
    #[error("the database is damaged after the crash: {findings:?}")]
    Damaged {
        /// SQLite's findings: the lines `PRAGMA integrity_check` returned,
        /// or the error SQLite refused the file with.
        findings: Vec<String>,
    },
    /// The database could not be opened.
    #[error("the database could not be opened after the crash")]
    Open(#[source] rusqlite::Error),
    /// `PRAGMA integrity_check` could not run.
    #[error("the integrity check could not run after the crash")]
    Check(#[source] rusqlite::Error),
}

/// Why a crash VFS could not be registered.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CrashVfsError {
    /// This process already registered a crash VFS under this name.
    #[error("a crash VFS named {name:?} is already registered")]
    NameTaken {
        /// The name.
        name: String,
    },
    /// The name holds a NUL, which SQLite cannot take.
    #[error("the VFS name {name:?} holds a NUL")]
    NameInvalid {
        /// The name.
        name: String,
    },
    /// SQLite refused the registration.
    #[error("SQLite refused to register the VFS: code {code}")]
    Register {
        /// SQLite's result code.
        code: i32,
    },
}

/// The names this process registered, so a second registration under one
/// fails instead of hiding the first.
fn registered_names() -> MutexGuard<'static, BTreeSet<String>> {
    static NAMES: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    NAMES.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock(disk: &Mutex<Disk>) -> MutexGuard<'_, Disk> {
    disk.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The number of shared-memory lock slots SQLite uses (`SQLITE_SHM_NLOCK`).
const SHM_LOCKS: usize = 8;

/// The in-memory disk behind one registered VFS.
struct Disk {
    /// Every file, by the path SQLite opened it with.
    files: BTreeMap<String, File>,
    /// The files deleted when closed, which a crash deletes too.
    temporary: BTreeSet<String>,
    /// The lock each open handle holds on each file.
    locks: BTreeMap<String, BTreeMap<u64, LockLevel>>,
    /// The shared memory of each database, by the instance that mapped it
    /// and the database's path. A crashed instance's stays until its last
    /// mapper lets it go.
    shared_memory: BTreeMap<(u64, String), SharedMemory>,
    /// The id of the next handle, and of the next unnamed temporary file.
    next_id: u64,
    /// The live instance: each crash starts the next one.
    instance: u64,
    /// The seed's stream, from which each power cut draws.
    stream: ChaCha20Rng,
    /// How many I/O operations ran.
    operations: u64,
    /// The faults still to inject, by the index of their operation.
    faults: BTreeMap<u64, IoFault>,
    /// Whether the disk is full.
    full: bool,
}

/// File names and sizes, never their bytes or the shared memory's.
impl fmt::Debug for Disk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sizes: BTreeMap<&str, usize> = self
            .files
            .iter()
            .map(|(path, file)| (path.as_str(), file.current.len()))
            .collect();
        formatter
            .debug_struct("Disk")
            .field("files", &sizes)
            .field("instance", &self.instance)
            .field("operations", &self.operations)
            .finish_non_exhaustive()
    }
}

impl Disk {
    fn new(seed: u64) -> Self {
        Self {
            files: BTreeMap::new(),
            temporary: BTreeSet::new(),
            locks: BTreeMap::new(),
            shared_memory: BTreeMap::new(),
            next_id: 0,
            instance: 0,
            // Its own domain of the seed, apart from a kill's moment.
            stream: seeded::stream(seed, b"crash-vfs"),
            operations: 0,
            faults: BTreeMap::new(),
            full: false,
        }
    }

    /// Counts one I/O operation and injects its fault: an
    /// [`IoFault::Error`] fails it with `code`, an [`IoFault::Full`] fills
    /// the disk.
    fn operate(&mut self, code: i32) -> VfsResult<()> {
        let index = self.operations;
        self.operations += 1;
        match self.faults.remove(&index) {
            Some(IoFault::Error) => Err(code),
            Some(IoFault::Full) => {
                self.full = true;
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Fails with `SQLITE_FULL` when the disk is full and `path` would
    /// grow to `size` bytes.
    fn refuse_growth(&self, path: &str, size: usize) -> VfsResult<()> {
        let grows = self
            .files
            .get(path)
            .is_some_and(|file| size > file.current.len());
        if self.full && grows {
            Err(vars::SQLITE_FULL)
        } else {
            Ok(())
        }
    }

    fn crash(&mut self, kind: CrashKind) -> Crash {
        let mut files = Vec::new();
        for (path, file) in &mut self.files {
            if file.unsynced.is_empty() {
                continue;
            }
            let landed = match kind {
                CrashKind::Process => Landed::All,
                CrashKind::PowerCut => draw_landed(&mut self.stream, &file.unsynced),
            };
            file.land(&landed);
            files.push(CrashedFile {
                path: path.clone(),
                unsynced: file.unsynced.len(),
                landed,
            });
            file.unsynced.clear();
        }
        for path in std::mem::take(&mut self.temporary) {
            self.files.remove(&path);
        }
        self.locks.clear();
        self.instance += 1;
        Crash { kind, files }
    }
}

/// Which of a file's unsynced `changes` a power cut lands, drawn from the
/// stream: none, a torn prefix, or a reordered subset, never all of them.
fn draw_landed(stream: &mut ChaCha20Rng, changes: &[Change]) -> Landed {
    let count = changes.len();
    match draw(stream, 3) {
        0 => Landed::None,
        1 => {
            let whole = draw(stream, count);
            let bytes = match &changes[whole] {
                Change::Write { data, .. } => draw(stream, data.len()),
                Change::Truncate { .. } => 0,
            };
            Landed::Torn { whole, bytes }
        }
        _ => {
            let lost = draw(stream, count);
            let changes = (0..count)
                .filter(|change| *change != lost && draw(stream, 2) == 1)
                .collect();
            Landed::Reordered { changes }
        }
    }
}

/// A number below `bound`, from the stream (with a modulo bias far below
/// anything a test notices).
fn draw(stream: &mut ChaCha20Rng, bound: usize) -> usize {
    let bound = u64::try_from(bound).expect("invariant: a usize fits in a u64");
    usize::try_from(stream.next_u64() % bound).expect("invariant: drawn below a usize bound")
}

/// One file: what a sync made durable, what reads see, and the changes
/// since the last sync, in order.
#[derive(Debug, Default)]
struct File {
    durable: Vec<u8>,
    current: Vec<u8>,
    unsynced: Vec<Change>,
}

impl File {
    /// Lands the unsynced changes a crash lets through on the durable
    /// bytes, which become what the next instance reads.
    fn land(&mut self, landed: &Landed) {
        match landed {
            Landed::All => self.durable.clone_from(&self.current),
            Landed::None => {}
            Landed::Torn { whole, bytes } => {
                for change in &self.unsynced[..*whole] {
                    change.apply(&mut self.durable);
                }
                if let Change::Write { offset, data } = &self.unsynced[*whole]
                    && *bytes > 0
                {
                    write_at(&mut self.durable, *offset, &data[..*bytes]);
                }
            }
            Landed::Reordered { changes } => {
                for change in changes {
                    self.unsynced[*change].apply(&mut self.durable);
                }
            }
        }
        self.current.clone_from(&self.durable);
    }
}

/// A change not yet synced.
#[derive(Debug, Clone)]
enum Change {
    Write { offset: usize, data: Vec<u8> },
    Truncate { size: usize },
}

impl Change {
    fn apply(&self, bytes: &mut Vec<u8>) {
        match self {
            Self::Write { offset, data } => write_at(bytes, *offset, data),
            Self::Truncate { size } => bytes.resize(*size, 0),
        }
    }
}

fn write_at(bytes: &mut Vec<u8>, offset: usize, data: &[u8]) {
    let end = offset + data.len();
    if bytes.len() < end {
        bytes.resize(end, 0);
    }
    bytes[offset..end].copy_from_slice(data);
}

/// A database's shared memory: its regions and its lock slots.
#[derive(Debug, Default)]
struct SharedMemory {
    regions: Vec<Region>,
    shared: [BTreeSet<u64>; SHM_LOCKS],
    exclusive: [Option<u64>; SHM_LOCKS],
    mapped_by: BTreeSet<u64>,
}

/// One region of shared memory. SQLite reads and writes it through the
/// address it was given, so the memory is never borrowed again from Rust
/// once that address is taken, `Debug` included.
struct Region {
    _memory: Vec<u8>,
    address: usize,
    size: usize,
}

/// Its address and size, never its bytes.
impl fmt::Debug for Region {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Region")
            .field("address", &format_args!("{:#x}", self.address))
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Region {
    fn new(size: usize) -> Self {
        let mut memory = vec![0_u8; size];
        let address = memory.as_mut_ptr().expose_provenance();
        Self {
            _memory: memory,
            address,
            size,
        }
    }

    fn pointer(&self) -> Option<NonNull<u8>> {
        NonNull::new(std::ptr::with_exposed_provenance_mut(self.address))
    }
}

/// An open file, as SQLite holds it.
#[derive(Debug)]
struct Handle {
    id: u64,
    /// The instance that opened it.
    instance: u64,
    path: String,
    readonly: bool,
    delete_on_close: bool,
}

impl Handle {
    /// The key of the shared memory this handle maps.
    fn memory_key(&self) -> (u64, String) {
        (self.instance, self.path.clone())
    }
}

impl VfsHandle for Handle {
    fn readonly(&self) -> bool {
        self.readonly
    }

    fn in_memory(&self) -> bool {
        false
    }
}

/// The VFS SQLite calls; it shares its disk with the [`CrashVfs`].
struct SqliteVfs {
    disk: Arc<Mutex<Disk>>,
}

impl Disk {
    /// Fails with `code` when `handle` belongs to a crashed instance, whose
    /// every I/O fails.
    fn refuse_crashed(&self, handle: &Handle, code: i32) -> VfsResult<()> {
        if handle.instance == self.instance {
            Ok(())
        } else {
            Err(code)
        }
    }

    fn file(&mut self, path: &str) -> VfsResult<&mut File> {
        self.files.get_mut(path).ok_or(vars::SQLITE_IOERR)
    }

    fn other_locks<'a>(&'a self, handle: &'a Handle) -> impl Iterator<Item = LockLevel> + 'a {
        self.locks
            .get(&handle.path)
            .into_iter()
            .flatten()
            .filter(move |(id, _)| **id != handle.id)
            .map(|(_, level)| *level)
    }

    fn set_lock(&mut self, handle: &Handle, level: LockLevel) {
        let locks = self.locks.entry(handle.path.clone()).or_default();
        if level == LockLevel::Unlocked {
            locks.remove(&handle.id);
        } else {
            locks.insert(handle.id, level);
        }
    }

    fn lock_of(&self, handle: &Handle) -> LockLevel {
        self.locks
            .get(&handle.path)
            .and_then(|locks| locks.get(&handle.id))
            .copied()
            .unwrap_or(LockLevel::Unlocked)
    }
}

impl Vfs for SqliteVfs {
    type Handle = Handle;

    fn open(&self, path: Option<&str>, opts: OpenOpts) -> VfsResult<Self::Handle> {
        let mut disk = lock(&self.disk);
        disk.operate(vars::SQLITE_CANTOPEN)?;
        let id = disk.next_id;
        disk.next_id += 1;
        let path = path.map_or_else(|| format!("temporary-{id}"), str::to_owned);
        let mode = opts.mode();
        let exists = disk.files.contains_key(&path);
        match mode {
            OpenMode::ReadOnly
            | OpenMode::ReadWrite {
                create: CreateMode::None,
            } if !exists => {
                return Err(vars::SQLITE_CANTOPEN);
            }
            OpenMode::ReadWrite {
                create: CreateMode::MustCreate,
            } if exists => return Err(vars::SQLITE_CANTOPEN),
            _ => {}
        }
        disk.files.entry(path.clone()).or_default();
        if opts.delete_on_close() {
            disk.temporary.insert(path.clone());
        }
        Ok(Handle {
            id,
            instance: disk.instance,
            path,
            readonly: mode.is_readonly(),
            delete_on_close: opts.delete_on_close(),
        })
    }

    fn delete(&self, path: &str) -> VfsResult<()> {
        let mut disk = lock(&self.disk);
        disk.operate(vars::SQLITE_IOERR_DELETE)?;
        disk.files
            .remove(path)
            .map(|_| ())
            .ok_or(vars::SQLITE_IOERR_DELETE_NOENT)
    }

    fn access(&self, path: &str, _flags: AccessFlags) -> VfsResult<bool> {
        let mut disk = lock(&self.disk);
        disk.operate(vars::SQLITE_IOERR_ACCESS)?;
        Ok(disk.files.contains_key(path))
    }

    fn file_size(&self, handle: &mut Self::Handle) -> VfsResult<usize> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_FSTAT)?;
        disk.operate(vars::SQLITE_IOERR_FSTAT)?;
        Ok(disk.file(&handle.path)?.current.len())
    }

    fn truncate(&self, handle: &mut Self::Handle, size: usize) -> VfsResult<()> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_TRUNCATE)?;
        disk.operate(vars::SQLITE_IOERR_TRUNCATE)?;
        disk.refuse_growth(&handle.path, size)?;
        let file = disk.file(&handle.path)?;
        let change = Change::Truncate { size };
        change.apply(&mut file.current);
        file.unsynced.push(change);
        Ok(())
    }

    fn write(&self, handle: &mut Self::Handle, offset: usize, data: &[u8]) -> VfsResult<usize> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_WRITE)?;
        disk.operate(vars::SQLITE_IOERR_WRITE)?;
        disk.refuse_growth(&handle.path, offset + data.len())?;
        let file = disk.file(&handle.path)?;
        let change = Change::Write {
            offset,
            data: data.to_vec(),
        };
        change.apply(&mut file.current);
        file.unsynced.push(change);
        Ok(data.len())
    }

    fn read(&self, handle: &mut Self::Handle, offset: usize, data: &mut [u8]) -> VfsResult<usize> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_READ)?;
        disk.operate(vars::SQLITE_IOERR_READ)?;
        let current = &disk.file(&handle.path)?.current;
        let available = current.get(offset..).unwrap_or_default();
        let count = available.len().min(data.len());
        data[..count].copy_from_slice(&available[..count]);
        Ok(count)
    }

    fn sync(&self, handle: &mut Self::Handle) -> VfsResult<()> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_FSYNC)?;
        disk.operate(vars::SQLITE_IOERR_FSYNC)?;
        let file = disk.file(&handle.path)?;
        file.durable.clone_from(&file.current);
        file.unsynced.clear();
        Ok(())
    }

    fn lock(&self, handle: &mut Self::Handle, level: LockLevel) -> VfsResult<()> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_LOCK)?;
        if disk.lock_of(handle) >= level {
            return Ok(());
        }
        let strongest_other = disk.other_locks(handle).max();
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
                disk.set_lock(handle, LockLevel::Pending);
                level == LockLevel::Exclusive && strongest_other >= Some(LockLevel::Shared)
            }
        };
        if busy {
            return Err(vars::SQLITE_BUSY);
        }
        disk.set_lock(handle, level);
        Ok(())
    }

    fn unlock(&self, handle: &mut Self::Handle, level: LockLevel) -> VfsResult<()> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_UNLOCK)?;
        if disk.lock_of(handle) > level {
            disk.set_lock(handle, level);
        }
        Ok(())
    }

    fn check_reserved_lock(&self, handle: &mut Self::Handle) -> VfsResult<bool> {
        let disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_CHECKRESERVEDLOCK)?;
        Ok(disk
            .locks
            .get(&handle.path)
            .into_iter()
            .flatten()
            .any(|(_, level)| *level >= LockLevel::Reserved))
    }

    fn close(&self, handle: Self::Handle) -> VfsResult<()> {
        let mut disk = lock(&self.disk);
        disk.set_lock(&handle, LockLevel::Unlocked);
        // A crashed instance's temporary files went with the crash.
        if handle.delete_on_close && handle.instance == disk.instance {
            disk.files.remove(&handle.path);
            disk.temporary.remove(&handle.path);
        }
        Ok(())
    }

    fn device_characteristics(&self, _handle: &mut Self::Handle) -> VfsResult<i32> {
        // None: a sector may be torn, and writes may land out of order.
        Ok(0)
    }

    fn shm_map(
        &self,
        handle: &mut Self::Handle,
        region_idx: usize,
        region_size: usize,
        extend: bool,
    ) -> VfsResult<Option<NonNull<u8>>> {
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_SHMMAP)?;
        let memory = disk.shared_memory.entry(handle.memory_key()).or_default();
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
        let mut disk = lock(&self.disk);
        disk.refuse_crashed(handle, vars::SQLITE_IOERR_SHMLOCK)?;
        let memory = disk
            .shared_memory
            .get_mut(&handle.memory_key())
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
        let mut disk = lock(&self.disk);
        let Some(memory) = disk.shared_memory.get_mut(&handle.memory_key()) else {
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
            disk.shared_memory.remove(&handle.memory_key());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
