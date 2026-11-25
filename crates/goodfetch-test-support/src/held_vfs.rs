//! A SQLite VFS that holds a file's read or write until the test lets it
//! go, and is otherwise the OS's own: it opens each file through SQLite's
//! default VFS, as the counting VFS does, and puts one method table in front
//! of the file's whose `xRead` and `xWrite` wait, when a hold names a
//! beginning of the file's path, for the test to let them go. A test then sees what
//! answers while a disk read or write is slow, such as the Runs' registry's
//! calls on another Run while one Run's start reads its run file (issue
//! 2258). It sits where the crash VFS sits (ADR 0060), named by a value
//! production leaves empty.
#![expect(
    unsafe_code,
    reason = "a SQLite VFS is C function pointers over SQLite's own structs, and SQLite's API offers no other way to hold a read"
)]
#![expect(
    clippy::disallowed_types,
    reason = "SQLite calls a VFS from any connection's thread: Mutexes over the tables an open reads, the files open and the holds, and a Condvar a held read waits on, with no lock order to test under loom"
)]

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use rusqlite::ffi;

/// How long, in real time, a held read or write waits at most: past it, it
/// goes on by itself, so code that waits on it still ends, even on a paused
/// clock, and a test that expected an answer meanwhile fails rather than
/// hangs.
pub const LONGEST_HOLD: Duration = Duration::from_secs(10);

/// A holding VFS, registered for the process's life under a name of its own.
#[derive(Clone, Debug)]
pub struct HeldVfs {
    name: String,
    holds: Holds,
}

/// The files a holding VFS holds, each with the state of its hold.
type Holds = Arc<Mutex<Vec<(PathBuf, Arc<HeldState>)>>>;

impl HeldVfs {
    /// Registers a new one in front of SQLite's default VFS.
    ///
    /// # Panics
    ///
    /// When SQLite has no default VFS or refuses the new one.
    #[must_use]
    pub fn register() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!("held-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let c_name = CString::new(name.clone()).expect("invariant: a name with no NUL");
        let holds = Arc::new(Mutex::new(Vec::new()));
        // SAFETY: a null name asks for the default VFS, which SQLite never
        // frees while the process lives.
        let base = unsafe { ffi::sqlite3_vfs_find(ptr::null()) };
        assert!(!base.is_null(), "invariant: SQLite has a default VFS");
        let shared: &'static Shared = Box::leak(Box::new(Shared {
            base,
            holds: Arc::clone(&holds),
            files: Mutex::new(BTreeMap::new()),
            tables: Mutex::new(Vec::new()),
        }));
        // SAFETY: `base` is live, and this copy of it differs only in its
        // name, its data and its `xOpen`; the name and the data are leaked, so
        // they outlive the registration, which lasts as long as the process.
        let code = unsafe {
            let vfs = Box::leak(Box::new(*base));
            vfs.pNext = ptr::null_mut();
            vfs.zName = c_name.into_raw();
            vfs.pAppData = ptr::from_ref(shared).cast_mut().cast::<c_void>();
            vfs.xOpen = Some(open);
            ffi::sqlite3_vfs_register(vfs, 0)
        };
        assert_eq!(code, ffi::SQLITE_OK, "SQLite refused the holding VFS");
        Self { name, holds }
    }

    /// The name the code under test opens its database with.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Holds the next read or write of a file whose path begins with
    /// `prefix`, opened through this VFS, once it is reached, until the
    /// answer lets it go or is dropped, or [`LONGEST_HOLD`] passed; the
    /// reads and writes after it are not held. A run file's path holds the
    /// file and its WAL, a folder's every file in it.
    #[must_use]
    pub fn hold_next(&self, prefix: &Path) -> HeldAccess {
        let state = Arc::new(HeldState::default());
        self.holds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((prefix.to_owned(), Arc::clone(&state)));
        HeldAccess(state)
    }
}

/// A read or write held once it is reached, until the test lets it go:
/// dropping it lets it go too, so a failed test never leaves a thread
/// waiting.
#[derive(Debug)]
pub struct HeldAccess(Arc<HeldState>);

#[derive(Debug, Default)]
struct HeldState {
    marks: Mutex<Marks>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct Marks {
    reached: bool,
    gone: bool,
    /// The held read or write went on, let go or past [`LONGEST_HOLD`].
    left: bool,
}

impl HeldAccess {
    /// Waits until the held read or write is reached, at most
    /// [`LONGEST_HOLD`] of real time, and answers whether it was. It waits
    /// on a blocking thread, so a paused clock stays where it is meanwhile,
    /// whatever thread the read or write comes from.
    #[expect(
        clippy::disallowed_methods,
        reason = "a test's wait on a Condvar, with no owner to track it, awaited before it returns"
    )]
    pub async fn reached(&self) -> bool {
        let state = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || state.wait_until(|marks| marks.reached))
            .await
            .expect("invariant: a wait on a Condvar does not panic")
    }

    /// Whether the read or write is reached and still held: a call that
    /// answers meanwhile did not wait on it.
    #[must_use]
    pub fn holding(&self) -> bool {
        let marks = self.0.marks.lock().unwrap_or_else(PoisonError::into_inner);
        marks.reached && !marks.left
    }

    /// Lets the held read or write go on.
    pub fn let_go(&self) {
        self.0.let_go();
    }
}

impl Drop for HeldAccess {
    fn drop(&mut self) {
        self.0.let_go();
    }
}

impl HeldState {
    fn let_go(&self) {
        self.marks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .gone = true;
        self.changed.notify_all();
    }

    /// Called by the read or write: says it is reached, then waits to be
    /// let go, at most [`LONGEST_HOLD`].
    fn wait(&self) {
        self.marks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reached = true;
        self.changed.notify_all();
        self.wait_until(|marks| marks.gone);
        self.marks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .left = true;
    }

    /// Waits until `done` holds of the marks, at most [`LONGEST_HOLD`],
    /// and answers whether it does.
    fn wait_until(&self, done: impl Fn(&Marks) -> bool) -> bool {
        let marks = self.marks.lock().unwrap_or_else(PoisonError::into_inner);
        let (marks, _) = self
            .changed
            .wait_timeout_while(marks, LONGEST_HOLD, |marks| !done(marks))
            .unwrap_or_else(PoisonError::into_inner);
        done(&marks)
    }
}

/// What `xOpen`, `xRead`, `xWrite` and `xClose` reach through their VFS
/// and their method table.
struct Shared {
    base: *mut ffi::sqlite3_vfs,
    holds: Holds,
    /// The path of each file open through it, by the address of its
    /// `sqlite3_file`.
    files: Mutex<BTreeMap<usize, PathBuf>>,
    /// One table per original method table the default VFS gave: it has a few,
    /// one per locking style.
    tables: Mutex<Vec<&'static Table>>,
}

/// An original method table with its `xRead`, `xWrite` and `xClose`
/// replaced. A file's `pMethods` points at `methods`, its first field, so
/// they find the rest from it.
#[repr(C)]
struct Table {
    methods: ffi::sqlite3_io_methods,
    original: *const ffi::sqlite3_io_methods,
    shared: &'static Shared,
}

// SAFETY: the pointers are SQLite's static method tables and its default VFS,
// which it never frees or changes; the rest is behind locks.
unsafe impl Send for Shared {}
// SAFETY: as for `Send`.
unsafe impl Sync for Shared {}
// SAFETY: as for `Shared`.
unsafe impl Send for Table {}
// SAFETY: as for `Send`.
unsafe impl Sync for Table {}

impl Shared {
    /// The table in front of `original`, made on the first file that has it.
    fn table(&'static self, original: *const ffi::sqlite3_io_methods) -> &'static Table {
        let mut tables = self.tables.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(table) = tables
            .iter()
            .find(|table| ptr::eq(table.original, original))
        {
            return table;
        }
        // SAFETY: SQLite gave `original`, one of its static method tables.
        let mut methods = unsafe { *original };
        methods.xRead = Some(read);
        methods.xWrite = Some(write);
        methods.xClose = Some(close);
        let table: &'static Table = Box::leak(Box::new(Table {
            methods,
            original,
            shared: self,
        }));
        tables.push(table);
        table
    }

    /// Waits on the hold whose prefix begins the path of the file at
    /// `address`, if any, taking it, so the next access goes on.
    fn wait_on_hold(&self, address: usize) {
        let Some(file) = self
            .files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&address)
            .cloned()
        else {
            return;
        };
        let held = {
            let mut holds = self.holds.lock().unwrap_or_else(PoisonError::into_inner);
            holds
                .iter()
                .position(|(prefix, _)| {
                    file.as_os_str()
                        .as_encoded_bytes()
                        .starts_with(prefix.as_os_str().as_encoded_bytes())
                })
                .map(|position| holds.remove(position).1)
        };
        if let Some(held) = held {
            held.wait();
        }
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
        let shared: &'static Shared = &*(*vfs).pAppData.cast::<Shared>();
        let base_open = (*shared.base)
            .xOpen
            .expect("invariant: the default VFS opens files");
        let code = base_open(shared.base, name, file, flags, out_flags);
        if code == ffi::SQLITE_OK && !(*file).pMethods.is_null() {
            (*file).pMethods = &raw const shared.table((*file).pMethods).methods;
            if !name.is_null() {
                let path = PathBuf::from(&*CStr::from_ptr(name).to_string_lossy());
                shared
                    .files
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(file.addr(), path);
            }
        }
        code
    }
}

/// Waits on the hold that names a beginning of the file's path, if any,
/// then reads.
unsafe extern "C" fn read(
    file: *mut ffi::sqlite3_file,
    buffer: *mut c_void,
    amount: c_int,
    offset: ffi::sqlite3_int64,
) -> c_int {
    // SAFETY: `open` set this file's `pMethods` to the first field of a leaked
    // `repr(C)` `Table`, the only way this function is reached, and the
    // original table's `xRead` takes the same arguments.
    unsafe {
        let table: &Table = &*(*file).pMethods.cast::<Table>();
        table.shared.wait_on_hold(file.addr());
        let original = (*table.original)
            .xRead
            .expect("invariant: a file's methods include xRead");
        original(file, buffer, amount, offset)
    }
}

/// Waits on the hold that names a beginning of the file's path, if any,
/// then writes.
unsafe extern "C" fn write(
    file: *mut ffi::sqlite3_file,
    buffer: *const c_void,
    amount: c_int,
    offset: ffi::sqlite3_int64,
) -> c_int {
    // SAFETY: as for `read`; the original `xWrite` takes the same arguments.
    unsafe {
        let table: &Table = &*(*file).pMethods.cast::<Table>();
        table.shared.wait_on_hold(file.addr());
        let original = (*table.original)
            .xWrite
            .expect("invariant: a file's methods include xWrite");
        original(file, buffer, amount, offset)
    }
}

/// Forgets the file's path, then closes it.
unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> c_int {
    // SAFETY: as for `read`; the original `xClose` takes the same file.
    unsafe {
        let table: &Table = &*(*file).pMethods.cast::<Table>();
        table
            .shared
            .files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&file.addr());
        let original = (*table.original)
            .xClose
            .expect("invariant: a file's methods include xClose");
        original(file)
    }
}

#[cfg(test)]
mod tests;
