//! A SQLite VFS that counts syncs and opens (ADR 0140) and is otherwise the
//! OS's own: it opens each file through SQLite's default VFS, counting each
//! database it opens, then puts one method table in front of the file's that
//! counts `xSync` and calls the original, so the real files take the real
//! writes, at the real sector size and device characteristics, and the count
//! is what the file does in production. It
//! names each file it opened, so a sync seen from outside the process, as
//! `xtask counts --strace` sees one, can be held to its count. It sits
//! where the crash VFS sits (ADR 0060), named by a value production leaves
//! empty; the crash VFS's own disk would pad the WAL to a sector and sync more.
#![expect(
    unsafe_code,
    reason = "a SQLite VFS is C function pointers over SQLite's own structs, and SQLite's API offers no other way to see a sync (ADR 0140)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "SQLite calls a VFS from any connection's thread: one relaxed count nothing synchronizes on, and one Mutex over the tables an open reads, with no lock order to test under loom"
)]

use std::ffi::{CStr, CString, c_char, c_int};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::ffi;

/// A counting VFS, registered for the process's life under a name of its own.
#[derive(Clone, Debug)]
pub struct CountingVfs {
    name: String,
    syncs: Arc<AtomicU64>,
    opens: Arc<AtomicU64>,
    files: Arc<Mutex<Vec<PathBuf>>>,
}

impl CountingVfs {
    /// Registers a new one in front of SQLite's default VFS.
    ///
    /// # Panics
    ///
    /// When SQLite has no default VFS or refuses the new one.
    #[must_use]
    pub fn register() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!("counting-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let c_name = CString::new(name.clone()).expect("invariant: a name with no NUL");
        let syncs = Arc::new(AtomicU64::new(0));
        let opens = Arc::new(AtomicU64::new(0));
        let files = Arc::new(Mutex::new(Vec::new()));
        // SAFETY: a null name asks for the default VFS, which SQLite never
        // frees while the process lives.
        let base = unsafe { ffi::sqlite3_vfs_find(ptr::null()) };
        assert!(!base.is_null(), "invariant: SQLite has a default VFS");
        let shared = Box::leak(Box::new(Shared {
            base,
            syncs: Arc::clone(&syncs),
            opens: Arc::clone(&opens),
            files: Arc::clone(&files),
            tables: Mutex::new(Vec::new()),
        }));
        // SAFETY: `base` is live, and this copy of it differs only in its
        // name, its data and its `xOpen`; the name and the data are leaked, so
        // they outlive the registration, which lasts as long as the process.
        let code = unsafe {
            let vfs = Box::leak(Box::new(*base));
            vfs.pNext = ptr::null_mut();
            vfs.zName = c_name.into_raw();
            vfs.pAppData = ptr::from_mut(shared).cast();
            vfs.xOpen = Some(open);
            ffi::sqlite3_vfs_register(vfs, 0)
        };
        assert_eq!(code, ffi::SQLITE_OK, "SQLite refused the counting VFS");
        Self {
            name,
            syncs,
            opens,
            files,
        }
    }

    /// The name the code under test opens its database with.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The syncs of every file opened through it so far.
    #[must_use]
    pub fn syncs(&self) -> u64 {
        self.syncs.load(Ordering::Relaxed)
    }

    /// How many times a database was opened through it so far, by any
    /// connection: a store that keeps its connection opens its file once.
    /// Its journal's and WAL's opens are not counted.
    #[must_use]
    pub fn opens(&self) -> u64 {
        self.opens.load(Ordering::Relaxed)
    }

    /// Each file opened through it so far, once, by the full path SQLite
    /// opened it at: a database's and its journal's or WAL's. A temporary
    /// file, which has no name, is not one.
    #[must_use]
    pub fn files(&self) -> Vec<PathBuf> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// What `xOpen` and `xSync` reach through their VFS and their method table.
struct Shared {
    base: *mut ffi::sqlite3_vfs,
    syncs: Arc<AtomicU64>,
    opens: Arc<AtomicU64>,
    files: Arc<Mutex<Vec<PathBuf>>>,
    /// One table per original method table the default VFS gave: it has a few,
    /// one per locking style.
    tables: Mutex<Vec<&'static Table>>,
}

/// An original method table with its `xSync` replaced. A file's `pMethods`
/// points at `methods`, its first field, so `xSync` finds the rest from it.
#[repr(C)]
struct Table {
    methods: ffi::sqlite3_io_methods,
    original: *const ffi::sqlite3_io_methods,
    syncs: Arc<AtomicU64>,
}

// SAFETY: the pointers are SQLite's static method tables and its default VFS,
// which it never frees or changes; the rest is an atomic count and a lock.
unsafe impl Send for Table {}
// SAFETY: as for `Send`.
unsafe impl Sync for Table {}

impl Shared {
    /// Names `file` among those opened, once.
    fn opened(&self, file: &Path) {
        let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        if !files.iter().any(|opened| opened == file) {
            files.push(file.to_owned());
        }
    }

    /// The table in front of `original`, made on the first file that has it.
    fn table(&self, original: *const ffi::sqlite3_io_methods) -> &'static Table {
        let mut tables = self.tables.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(table) = tables
            .iter()
            .find(|table| ptr::eq(table.original, original))
        {
            return table;
        }
        // SAFETY: SQLite gave `original`, one of its static method tables.
        let mut methods = unsafe { *original };
        methods.xSync = Some(sync);
        let table: &'static Table = Box::leak(Box::new(Table {
            methods,
            original,
            syncs: Arc::clone(&self.syncs),
        }));
        tables.push(table);
        table
    }
}

/// The default VFS's `xOpen`, then this VFS's table in front of the file's.
unsafe extern "C" fn open(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: `register` set `pAppData` to a leaked `Shared` and `xOpen` to
    // this function, and SQLite calls it with its own valid arguments, sized
    // by the `szOsFile` copied from the default VFS; a `name` that is not null
    // is the NUL-terminated full path SQLite opens, live for the call.
    unsafe {
        let shared: &Shared = &*(*vfs).pAppData.cast::<Shared>();
        let base_open = (*shared.base)
            .xOpen
            .expect("invariant: the default VFS opens files");
        let code = base_open(shared.base, name, file, flags, out_flags);
        if code == ffi::SQLITE_OK && !(*file).pMethods.is_null() {
            (*file).pMethods = &raw const shared.table((*file).pMethods).methods;
            if flags & ffi::SQLITE_OPEN_MAIN_DB != 0 {
                shared.opens.fetch_add(1, Ordering::Relaxed);
            }
            if !name.is_null() {
                shared.opened(Path::new(&*CStr::from_ptr(name).to_string_lossy()));
            }
        }
        code
    }
}

/// Counts one sync, then does it.
unsafe extern "C" fn sync(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    // SAFETY: `open` set this file's `pMethods` to the first field of a leaked
    // `repr(C)` `Table`, the only way this function is reached, and the
    // original table's `xSync` takes the same file.
    unsafe {
        let table: &Table = &*(*file).pMethods.cast::<Table>();
        table.syncs.fetch_add(1, Ordering::Relaxed);
        let original = (*table.original)
            .xSync
            .expect("invariant: a file's methods include xSync");
        original(file, flags)
    }
}

#[cfg(test)]
mod tests;
