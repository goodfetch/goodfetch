//! A folder mounted through LazyFS (ADR 0060, Linux): a FUSE file system
//! whose own cache holds every write until it is synced, so that
//! `clear_cache` after a kill drops what never reached the disk, as a power
//! cut would. The executable is the pinned build `xtask lazyfs build` makes,
//! named by `GOODFETCH_LAZYFS`. A mount that cannot be made is an error, which
//! fails the test; it is never a skip (ADR 0067's rule, applied here). The
//! operating system lets a user mount only in some folders, so a LazyFS
//! test's setup is `TestEnv::for_lazyfs`, whose folder is in one of them, and
//! a mount refused there is [`LazyFsError::MountRefused`], which names them.
//!
//! The folder keeps its path: its files move to a backing folder beside it,
//! `<name>.lazyfs/backing`, which LazyFS mounts back at the folder's path, and
//! return when the mount ends. Ending the mount (`unmount`, or dropping the
//! value) first syncs every file, as a clean shutdown would, so only
//! `clear_cache` loses data. A remount is a second `mount` of the same folder.
//!
//! The kernel refuses an unmount while anything holds the mount: a process
//! using the folder, which fails the end, or a lookup that only passes
//! through it, as `lsof` and `df` make when they stat every mount point on
//! the machine. When no process uses the folder, the end detaches the mount
//! and waits for LazyFS to exit, which it does once such a lookup is done.
#![expect(
    clippy::disallowed_methods,
    reason = "test support mounts, polls and reads files, reads LazyFS's answers on a thread of its own and times its deadlines on the real clock, outside any async worker and any simulated clock (ADR 0062)"
)]

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead as _, BufReader, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::env::TestEnv;
use crate::process::Process;
use crate::variables::{LAZYFS_FOLDER_VARIABLE, LAZYFS_VARIABLE, Variables};

/// How long a mount, an unmount or a command may take.
pub const LAZYFS_DEADLINE: Duration = Duration::from_secs(10);

/// How often a mount and an unmount are checked for.
const POLL: Duration = Duration::from_millis(10);

/// LazyFS's cache: large enough for any test's unsynced writes, since a full
/// cache without eviction refuses writes.
const CACHE_SIZE: &str = "512mb";

/// What `fusermount3` prints, through LazyFS, when the kernel denies it the
/// mount (`EACCES`), as an AppArmor profile does outside its folders.
const MOUNT_DENIED: &str = "fusermount3: mount failed: Permission denied";

/// The command that drops every unsynced write, and LazyFS's answer to it.
const CLEAR_CACHE: &[u8] = b"lazyfs::clear-cache\n";
const CACHE_CLEARED: &str = "finished::clear-cache";

/// A folder mounted through LazyFS until the value drops.
pub struct LazyFs {
    folder: PathBuf,
    state: PathBuf,
    /// LazyFS's process, which the test's process record holds too.
    child: Option<Process>,
    /// LazyFS's answers FIFO, opened for reading and writing before LazyFS
    /// starts, so that LazyFS's own open of it never waits, and so that a
    /// line written here at the end wakes the reader thread.
    answers_fifo: File,
    /// Each line LazyFS answers, read from the FIFO by a thread.
    answers: Option<Receiver<io::Result<String>>>,
}

impl std::fmt::Debug for LazyFs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LazyFs")
            .field("folder", &self.folder)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl LazyFs {
    /// Mounts `folder` through the LazyFS that `GOODFETCH_LAZYFS` names,
    /// creating the folder if its parent exists. LazyFS runs in the test's
    /// cleared environment, and its process joins the test's record.
    pub fn mount(env: &TestEnv, folder: impl AsRef<Path>) -> Result<Self, LazyFsError> {
        let program = Variables::from_process()
            .get(LAZYFS_VARIABLE)
            .map(PathBuf::from)
            .ok_or(LazyFsError::Unset)?;
        Self::mount_program(env, &program, folder)
    }

    /// Mounts `folder` through the LazyFS executable at `program`.
    pub fn mount_program(
        env: &TestEnv,
        program: &Path,
        folder: impl AsRef<Path>,
    ) -> Result<Self, LazyFsError> {
        let folder = folder.as_ref().to_owned();
        let in_folder = |source| LazyFsError::Folder {
            path: folder.clone(),
            source,
        };
        match std::fs::create_dir(&folder) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => {
                return Err(in_folder(error));
            }
            _ => {}
        }
        let mut state_name = folder
            .file_name()
            .map(ToOwned::to_owned)
            .unwrap_or_default();
        state_name.push(".lazyfs");
        let state = folder.with_file_name(state_name);
        std::fs::create_dir(&state).map_err(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists {
                LazyFsError::InUse {
                    state: state.clone(),
                }
            } else {
                in_folder(source)
            }
        })?;
        let backing = state.join("backing");
        let faults = state.join("faults.fifo");
        let answers = state.join("answers.fifo");
        let config = state.join("config.toml");
        let output = state.join("output.txt");
        let prepared = (|| {
            make_fifo(&faults)?;
            make_fifo(&answers)?;
            std::fs::write(&config, configuration(&faults, &answers))?;
            // Linux opens a FIFO for reading and writing at once, without
            // waiting for another end.
            let fifo = OpenOptions::new().read(true).write(true).open(&answers)?;
            let reader = fifo.try_clone()?;
            std::fs::rename(&folder, &backing)?;
            std::fs::create_dir(&folder)?;
            Ok((fifo, reader))
        })();
        let (answers_fifo, reader) = match prepared {
            Ok(ends) => ends,
            Err(source) => {
                restore(&folder, &backing);
                let _ = std::fs::remove_dir_all(&state);
                return Err(in_folder(source));
            }
        };
        // Bounded (STYLE.md): the reader waits while no answer is taken.
        let (sender, receiver) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut mounted = Self {
            folder,
            state,
            child: None,
            answers_fifo,
            answers: Some(receiver),
        };
        let started = File::create(&output).and_then(|out| {
            let err = out.try_clone()?;
            env.spawn_with_streams(
                env.process_command(program)
                    .arg(&mounted.folder)
                    .arg("--config-path")
                    .arg(&config)
                    .args(["-o", "modules=subdir", "-o"])
                    .arg(subdir_option(&backing))
                    .arg("-f")
                    .stdin(Stdio::null())
                    .stdout(out)
                    .stderr(err),
            )
        });
        match started {
            Ok(child) => mounted.child = Some(child),
            Err(source) => {
                mounted.end_unmounted();
                return Err(LazyFsError::Start {
                    program: program.to_owned(),
                    source,
                });
            }
        }
        if let Err(error) = mounted.wait_for_mount() {
            mounted.end_unmounted();
            return Err(error);
        }
        Ok(mounted)
    }

    /// The mounted folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Drops every write LazyFS holds that was never synced, and returns once
    /// LazyFS says it has.
    pub fn clear_cache(&self) -> Result<(), LazyFsError> {
        OpenOptions::new()
            .write(true)
            .open(self.state.join("faults.fifo"))
            .and_then(|mut fifo| fifo.write_all(CLEAR_CACHE))
            .map_err(LazyFsError::Command)?;
        let answer = self.read_answer()?;
        if answer == CACHE_CLEARED {
            Ok(())
        } else {
            Err(LazyFsError::Answer {
                answer,
                output: self.output(),
            })
        }
    }

    /// Ends the mount: syncs every file, unmounts, and moves the files back
    /// into the folder.
    pub fn unmount(mut self) -> Result<(), LazyFsError> {
        self.end()
    }

    fn end(&mut self) -> Result<(), LazyFsError> {
        if self.child.is_none() {
            return Ok(());
        }
        let ended = self.end_mounted();
        if ended.is_err() {
            // Whatever failed, LazyFS stops and the mount goes, so the test's
            // folder is never left mounted.
            let _ = fusermount(&["-uz"], &self.folder);
        }
        self.end_unmounted();
        ended
    }

    /// Syncs every file, unmounts and waits for LazyFS to exit. A refused
    /// unmount with no process using the folder was held by a passing
    /// lookup: the mount is detached instead, and LazyFS exits once the
    /// lookup is done.
    fn end_mounted(&mut self) -> Result<(), LazyFsError> {
        sync_tree(&self.folder).map_err(LazyFsError::Sync)?;
        let unmounted = fusermount(&["-u"], &self.folder)?;
        if !unmounted.status.success() {
            let users = users_of(&self.folder);
            let refused = |output: std::process::Output| LazyFsError::NotUnmounted {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                users: users.clone(),
            };
            if !users.is_empty() {
                return Err(refused(unmounted));
            }
            let detached = fusermount(&["-uz"], &self.folder)?;
            if !detached.status.success() {
                return Err(refused(detached));
            }
        }
        let deadline = Instant::now() + LAZYFS_DEADLINE;
        loop {
            let exited = self
                .child
                .as_ref()
                .expect("invariant: a mounted LazyFS has its process")
                .child()
                .try_wait()
                .map_err(LazyFsError::Unmount)?;
            if exited.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(LazyFsError::Deadline {
                    waiting_for: "LazyFS to exit after its unmount",
                    output: self.output(),
                });
            }
            std::thread::sleep(POLL);
        }
        Ok(())
    }

    /// Stops the reader thread, moves the files back into the folder and
    /// removes the state folder, once nothing is mounted.
    fn end_unmounted(&mut self) {
        if let Some(process) = self.child.take() {
            // Best effort: a LazyFS that never mounted, or failed its end, is
            // stopped either way; one that exited is reaped.
            let mut child = process.child();
            let _ = child.kill();
            let _ = child.wait();
        }
        // The reader thread wakes on this line, finds no receiver and ends.
        if self.answers.take().is_some() {
            let _ = self.answers_fifo.write_all(b"\n");
        }
        let backing = self.state.join("backing");
        restore(&self.folder, &backing);
        if !backing.exists() {
            let _ = std::fs::remove_dir_all(&self.state);
        }
    }

    fn wait_for_mount(&mut self) -> Result<(), LazyFsError> {
        let deadline = Instant::now() + LAZYFS_DEADLINE;
        loop {
            if is_mount_point(&self.folder) {
                return Ok(());
            }
            let exited = self
                .child
                .as_ref()
                .expect("invariant: a starting LazyFS has its process")
                .child()
                .try_wait()
                .map_err(LazyFsError::Command)?;
            if let Some(status) = exited {
                let output = self.output();
                return Err(if output.contains(MOUNT_DENIED) {
                    LazyFsError::MountRefused {
                        folder: self.folder.clone(),
                        output,
                    }
                } else {
                    LazyFsError::NotMounted {
                        status: Some(status),
                        output,
                    }
                });
            }
            if Instant::now() >= deadline {
                return Err(LazyFsError::NotMounted {
                    status: None,
                    output: self.output(),
                });
            }
            std::thread::sleep(POLL);
        }
    }

    /// One line LazyFS answered, waiting at most [`LAZYFS_DEADLINE`].
    fn read_answer(&self) -> Result<String, LazyFsError> {
        let answers = self
            .answers
            .as_ref()
            .expect("invariant: a mounted LazyFS has its reader");
        match answers.recv_timeout(LAZYFS_DEADLINE) {
            Ok(line) => line.map_err(LazyFsError::Command),
            Err(_) => Err(LazyFsError::Deadline {
                waiting_for: "LazyFS's answer to a command",
                output: self.output(),
            }),
        }
    }

    /// What LazyFS printed, for an error.
    fn output(&self) -> String {
        std::fs::read(self.state.join("output.txt"))
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    }
}

impl Drop for LazyFs {
    fn drop(&mut self) {
        if let Err(error) = self.end()
            && !std::thread::panicking()
        {
            panic!(
                "the LazyFS mount of {} cannot end: {error}",
                self.folder.display()
            );
        }
    }
}

/// LazyFS's configuration: its cache without eviction, and its two FIFOs.
/// A JSON string is a TOML basic string, escapes included.
fn configuration(faults: &Path, answers: &Path) -> String {
    let string = |path: &Path| {
        serde_json::to_string(&path.display().to_string())
            .expect("invariant: a string serializes to JSON")
    };
    format!(
        "[faults]\nfifo_path = {}\nfifo_path_completed = {}\n\n\
         [cache]\napply_eviction = false\n\n\
         [cache.simple]\ncustom_size = \"{CACHE_SIZE}\"\nblocks_per_page = 1\n\n\
         [filesystem]\nlog_all_operations = false\nlogfile = \"\"\n",
        string(faults),
        string(answers),
    )
}

/// FUSE's `subdir` option, its `\` and `,` escaped as FUSE's option parser
/// reads them.
fn subdir_option(backing: &Path) -> OsString {
    let path = backing.display().to_string();
    OsString::from(format!(
        "subdir={}",
        path.replace('\\', "\\\\").replace(',', "\\,")
    ))
}

/// Makes a FIFO with coreutils' `mkfifo`, which the standard library lacks.
fn make_fifo(path: &Path) -> io::Result<()> {
    let output = Command::new("mkfifo")
        .args(["-m", "600", "--"])
        .arg(path)
        .stdin(Stdio::null())
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "mkfifo {} failed ({}): {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

/// Whether `path` is the root of another file system than its parent's.
fn is_mount_point(path: &Path) -> bool {
    let (Some(parent), Ok(folder)) = (path.parent(), std::fs::metadata(path)) else {
        return false;
    };
    std::fs::metadata(parent).is_ok_and(|parent| parent.dev() != folder.dev())
}

/// Moves the backing folder's files back to the folder's path, if they are
/// still beside it.
fn restore(folder: &Path, backing: &Path) {
    if backing.exists() {
        // Best effort: the empty mount point goes first, and a failure leaves
        // the files in the backing folder, where the kept test folder shows
        // them.
        let _ = std::fs::remove_dir(folder);
        let _ = std::fs::rename(backing, folder);
    }
}

/// Runs `fusermount3` with `options` on `folder`.
fn fusermount(options: &[&str], folder: &Path) -> Result<std::process::Output, LazyFsError> {
    Command::new("fusermount3")
        .args(options)
        .arg(folder)
        .stdin(Stdio::null())
        .output()
        .map_err(LazyFsError::Unmount)
}

/// The processes using `folder`, as `<pid> <name>: <what>`: a working or
/// root folder, an open file or a mapping in it. Read from `/proc` without
/// looking into the folder, and only for the processes this user may
/// inspect, which every process a test starts is.
fn users_of(folder: &Path) -> Vec<String> {
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let inside = |target: &Path| target.starts_with(folder);
    let mut users = Vec::new();
    for process in processes.flatten() {
        let proc = process.path();
        if !process
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let mut found = Vec::new();
        for link in ["cwd", "root"] {
            if std::fs::read_link(proc.join(link)).is_ok_and(|target| inside(&target)) {
                found.push(link.to_owned());
            }
        }
        if let Ok(files) = std::fs::read_dir(proc.join("fd")) {
            found.extend(files.flatten().filter_map(|file| {
                std::fs::read_link(file.path())
                    .ok()
                    .filter(|target| inside(target))
                    .map(|target| format!("open {}", target.display()))
            }));
        }
        if let Ok(maps) = std::fs::read_to_string(proc.join("maps")) {
            found.extend(
                maps.lines()
                    .filter_map(|line| line.find('/').and_then(|start| line.get(start..)))
                    .filter(|target| inside(Path::new(target)))
                    .map(|target| format!("mapped {target}")),
            );
        }
        if !found.is_empty() {
            let name = std::fs::read_to_string(proc.join("comm")).unwrap_or_default();
            users.push(format!(
                "{} {}: {}",
                process.file_name().to_string_lossy(),
                name.trim_end(),
                found.join(", ")
            ));
        }
    }
    users
}

/// Syncs every regular file under `folder`, as a clean shutdown would.
fn sync_tree(folder: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let synced = entry.file_type().and_then(|kind| {
            if kind.is_dir() {
                sync_tree(&entry.path())
            } else if kind.is_file() {
                File::open(entry.path())?.sync_all()
            } else {
                Ok(())
            }
        });
        match synced {
            // Gone since the listing, it needs no sync: FUSE keeps a file
            // unlinked while open, as SQLite's WAL is at its last close, as
            // `.fuse_hidden…` until the kernel sends its release, after the
            // close and so possibly during this walk.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            synced => synced?,
        }
    }
    Ok(())
}

/// Its message, which holds every field: a test's `unwrap()` then panics
/// with the cause in words and LazyFS's output on its own lines.
impl std::fmt::Debug for LazyFsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// Why a folder cannot be mounted through LazyFS, or its mount used or
/// ended. Each fails the test.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum LazyFsError {
    /// `GOODFETCH_LAZYFS` is not set.
    #[error(
        "{LAZYFS_VARIABLE} is not set: run the LazyFS tests with `just lazyfs-test`, which builds the pinned LazyFS and names it"
    )]
    Unset,
    /// The folder, or its state folder beside it, cannot be prepared.
    #[error("the folder {} cannot be prepared for LazyFS: {source}", path.display())]
    Folder {
        /// The folder.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The folder's state folder exists: a mount of it has not ended.
    #[error("{} exists: the folder is mounted through LazyFS already, or a mount of it did not end", state.display())]
    InUse {
        /// The state folder.
        state: PathBuf,
    },
    /// LazyFS cannot be started.
    #[error("LazyFS ({}) cannot start: {source}", program.display())]
    Start {
        /// The executable.
        program: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// LazyFS exited, or did not mount the folder within [`LAZYFS_DEADLINE`].
    #[error("LazyFS did not mount the folder ({}); it printed:\n{output}", status.map_or_else(|| "still running at the deadline".to_owned(), |status| status.to_string()))]
    NotMounted {
        /// How it exited, if it did.
        status: Option<ExitStatus>,
        /// What it printed.
        output: String,
    },
    /// The operating system refused the mount: `fusermount3` was denied it,
    /// as Ubuntu's AppArmor profile of `fusermount3` denies a mount outside
    /// the folders it allows.
    #[error(
        "the operating system refused LazyFS's mount of {}: fusermount3 was denied it, as Ubuntu's AppArmor profile of fusermount3 (/etc/apparmor.d/fusermount3) denies a FUSE mount outside $HOME, /mnt, /run/user/<uid>, /media and /tmp. Make the test's setup with `TestEnv::for_lazyfs()`, whose folder is in /tmp, or in the folder {LAZYFS_FOLDER_VARIABLE} names, which must be one of those; LazyFS printed:\n{output}",
        folder.display()
    )]
    MountRefused {
        /// The folder.
        folder: PathBuf,
        /// What it printed.
        output: String,
    },
    /// A command could not be sent to LazyFS, or its answer read.
    #[error("a command to LazyFS failed: {0}")]
    Command(io::Error),
    /// LazyFS answered something other than the command's answer.
    #[error("LazyFS answered `{answer}`; it printed:\n{output}")]
    Answer {
        /// The answer.
        answer: String,
        /// What it printed.
        output: String,
    },
    /// LazyFS did not answer or exit within [`LAZYFS_DEADLINE`].
    #[error("waited {LAZYFS_DEADLINE:?} for {waiting_for}; it printed:\n{output}")]
    Deadline {
        /// What was awaited.
        waiting_for: &'static str,
        /// What it printed.
        output: String,
    },
    /// The files cannot be synced before the unmount.
    #[error("the mounted files cannot be synced before the unmount: {0}")]
    Sync(io::Error),
    /// `fusermount3 -u` cannot run, or LazyFS's exit cannot be awaited.
    #[error("the unmount cannot run: {0}")]
    Unmount(io::Error),
    /// `fusermount3` refused the unmount while a process still uses the
    /// folder, or refused to detach it.
    #[error(
        "fusermount3 refused the unmount ({status}): {stderr}; processes using the folder: {users:?}"
    )]
    NotUnmounted {
        /// Its exit status.
        status: ExitStatus,
        /// What it printed.
        stderr: String,
        /// The processes using the folder, as `<pid> <name>: <what>`.
        users: Vec<String>,
    },
}
