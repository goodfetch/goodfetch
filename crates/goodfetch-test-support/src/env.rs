//! `TestEnv`, each test's one entry point (ADRs 0059 and 0062): its own
//! folder, Goodfetch home and cleared environment map, its printed seed, the
//! fixed start time, unique names, the commands and processes it starts, the
//! crash-dump folder of the browsers they launch, and the engine's JSON log
//! in its home, which it copies into the test's exercise record (ADR 0072).
#![expect(
    clippy::disallowed_methods,
    reason = "test support creates folders and starts processes outside any async worker (ADR 0062)"
)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use crate::crash_dumps::{self, CRASH_DUMP_VARIABLE, CrashDumps};
use crate::exercise::{DISPATCH_TARGET, ExerciseRecord, OUTCOME_TARGET};
use crate::locales::{LocaleTableError, table_lang, table_zone};
use crate::process::{Process, ProcessTracker, ProcessesAlive};
use crate::runtime_file::{
    DaemonEndpoint, RUNTIME_FILE, RuntimeFile, SOCKET_FOLDER_PREFIX, major_folder,
};
use crate::variables::{
    COVERAGE_VARIABLE, SeedError, Suite, SuiteError, Variables, WINDOWS_VARIABLES, current_test,
};

/// Every test's start time, 2026-01-05T09:00:00Z: a Monday outside any DST
/// change in the shared time zones. A test that needs another passes its own.
pub const START_TIME: &str = "2026-01-05T09:00:00Z";

/// [`START_TIME`] in seconds since the Unix epoch.
const START_SECONDS: u64 = 1_767_603_600;

/// Which logs every engine the test starts writes: `rpc`'s dispatch records
/// and the engine's outcome records, which ADR 0072's exercise audit reads,
/// beside the default level.
fn log_level() -> String {
    format!("info,{DISPATCH_TARGET}=trace,{OUTCOME_TARGET}=trace")
}

/// How long a folder name keeps the test's name.
const NAME_LENGTH: usize = 64;

/// The settings every test's user configuration starts with: the Console
/// of every Daemon the test starts listens on a port the OS chooses, never
/// on its major's fixed port (ADR 0059); and every engine process writes
/// its JSON log into the home's log folder (ADR 0122), at the test's level
/// (`log.level`, [`TestEnv::log_level`]).
pub const TEST_SETTINGS: &str = "console.address = \"127.0.0.1:0\"\nlog.output = \"file\"\n";

/// The user configuration's file in the per-major folder.
const USER_CONFIGURATION: &str = "configuration.toml";

/// One test's isolated setup. Its folder is removed when it drops, unless
/// the test failed: then it is kept and listed on stderr. Dropping it also
/// fails the test if a process it started is still alive, or if its
/// crash-dump folder holds a file. Its user configuration turns on the
/// JSON log of every engine the test starts, whose lines it copies into
/// the test's exercise record when it drops, under `just test`.
pub struct TestEnv {
    folder: tempfile::TempDir,
    home: PathBuf,
    /// The `log.level` of its user configuration.
    log_level: std::cell::RefCell<String>,
    crash_dumps: PathBuf,
    env: BTreeMap<String, String>,
    seed: u64,
    suite: Suite,
    names: Cell<u64>,
    processes: ProcessTracker,
    record: Option<ExerciseRecord>,
}

impl TestEnv {
    /// The running test's setup, from its thread's name and the process's
    /// variables.
    ///
    /// # Panics
    ///
    /// If its folder cannot be made or a variable cannot be read.
    pub fn new() -> Self {
        Self::create(&current_test(), &Variables::from_process())
            .unwrap_or_else(|error| panic!("the test's TestEnv cannot start: {error}"))
    }

    /// The running test's setup as [`TestEnv::new`] makes it, its folder on
    /// a memory file system where the OS keeps one at a known place
    /// (`/dev/shm` on Linux), else in the temporary folder as `new`'s: for
    /// an in-process test of a store, whose synced writes and new files
    /// then wait for no disk, however busy other writers keep it
    /// (fix-queue 123). A test that starts a browser or an engine keeps
    /// [`TestEnv::new`]: its copies would sit in memory.
    ///
    /// # Panics
    ///
    /// As [`TestEnv::new`].
    pub fn in_memory() -> Self {
        Self::create_in(
            &current_test(),
            &Variables::from_process(),
            memory_folder().as_deref(),
        )
        .unwrap_or_else(|error| panic!("the test's TestEnv cannot start: {error}"))
    }

    /// The setup of the test with this full name, from these variables.
    /// Prints `goodfetch-test-seed=<n>` to stderr once the seed is known,
    /// before anything else, so a test killed on a timeout still shows it.
    pub fn create(test: &str, variables: &Variables) -> Result<Self, TestEnvError> {
        Self::create_in(test, variables, None)
    }

    /// As [`TestEnv::create`], its folder in `root`, or in the temporary
    /// folder when none.
    fn create_in(
        test: &str,
        variables: &Variables,
        root: Option<&Path>,
    ) -> Result<Self, TestEnvError> {
        let seed = variables.seed(test)?;
        #[expect(
            clippy::print_stderr,
            reason = "the seed is printed when the test starts (ADR 0060)"
        )]
        {
            eprintln!("goodfetch-test-seed={seed}");
        }
        let suite = variables.suite()?;
        let prefix = format!("{} é ", folder_name(test));
        let mut builder = tempfile::Builder::new();
        builder.prefix(&prefix);
        let folder = match root {
            Some(root) => builder.tempdir_in(root)?,
            None => builder.tempdir()?,
        };
        let root = folder.path();
        let home = root.join("home");
        let crash_dumps = home.join(crash_dumps::FOLDER);
        let user = root.join("user");
        let folders = [
            ("GOODFETCH_HOME", home.clone()),
            (CRASH_DUMP_VARIABLE, crash_dumps.clone()),
            ("HOME", user.clone()),
            ("USERPROFILE", user.clone()),
            ("XDG_CONFIG_HOME", user.join(".config")),
            ("XDG_DATA_HOME", user.join(".local").join("share")),
            ("XDG_CACHE_HOME", user.join(".cache")),
            ("XDG_STATE_HOME", user.join(".local").join("state")),
            ("XDG_RUNTIME_DIR", root.join("runtime")),
        ];
        let mut env = BTreeMap::new();
        for (name, path) in folders {
            std::fs::create_dir_all(&path)?;
            let value = path
                .to_str()
                .ok_or_else(|| TestEnvError::NotUnicode(path.clone()))?;
            env.insert(name.to_owned(), value.to_owned());
        }
        #[cfg(unix)]
        restrict_to_owner(&root.join("runtime"))?;
        write_user_configuration(&home, "", &log_level())?;
        env.insert("TZ".to_owned(), "UTC".to_owned());
        env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
        let record = ExerciseRecord::of(variables);
        if let Some(record) = &record {
            record.open()?;
        }
        if let Some(profile) = variables.get(COVERAGE_VARIABLE) {
            env.insert(COVERAGE_VARIABLE.to_owned(), profile.to_owned());
        }
        if cfg!(windows) {
            for name in WINDOWS_VARIABLES {
                if let Some(value) = variables.get(name) {
                    env.insert(name.to_owned(), value.to_owned());
                }
            }
        }
        Ok(Self {
            folder,
            home,
            log_level: std::cell::RefCell::new(log_level()),
            crash_dumps,
            env,
            seed,
            suite,
            names: Cell::new(0),
            processes: ProcessTracker::default(),
            record,
        })
    }

    /// The same setup with `LANG` for this locale of the shared table
    /// (ADR 0073), given by its BCP 47 tag (`tr-TR` becomes `tr_TR.UTF-8`).
    ///
    /// # Errors
    ///
    /// [`LocaleTableError::LocaleUnknown`] for a tag outside the table.
    pub fn with_locale(mut self, tag: &str) -> Result<Self, LocaleTableError> {
        self.env.insert("LANG".to_owned(), table_lang(tag)?);
        Ok(self)
    }

    /// The same setup with `TZ` for this time zone of the shared table.
    ///
    /// # Errors
    ///
    /// [`LocaleTableError::TimeZoneUnknown`] for a zone outside the table.
    pub fn with_time_zone(mut self, zone: &str) -> Result<Self, LocaleTableError> {
        self.env
            .insert("TZ".to_owned(), table_zone(zone)?.to_owned());
        Ok(self)
    }

    /// The same setup with one more variable in the map every child gets,
    /// such as `TERM` for a run on a pseudo-terminal.
    #[must_use]
    pub fn with_variable(mut self, name: &str, value: &str) -> Self {
        self.env.insert(name.to_owned(), value.to_owned());
        self
    }

    /// The test's own folder.
    pub fn folder(&self) -> &Path {
        self.folder.path()
    }

    /// `<folder>/home`, the `GOODFETCH_HOME` of everything the test starts.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Writes the home's user configuration with `settings`, lines of TOML
    /// under `[settings]`, after [`TEST_SETTINGS`] and the test's
    /// `log.level`, which every test's home starts with, each unless
    /// `settings` sets its key itself.
    ///
    /// # Panics
    ///
    /// If the file cannot be written.
    pub fn user_configuration(&self, settings: &str) {
        write_user_configuration(&self.home, settings, &self.log_level.borrow()).unwrap_or_else(
            |error| panic!("the test's user configuration cannot be written: {error}"),
        );
    }

    /// Sets the `log.level` of every engine the test starts from now on,
    /// a `tracing` filter directive, such as `trace` for a test whose
    /// stdout must hold no log line at any level, or `info` for a
    /// pseudo-terminal's transcript two runs compare (the trace lines carry
    /// their instant): in the user configuration as it is, whoever wrote
    /// it, every other line kept (ADR 0122).
    ///
    /// # Panics
    ///
    /// If the file cannot be read or written.
    pub fn log_level(&self, filter: &str) {
        filter.clone_into(&mut self.log_level.borrow_mut());
        set_log_level(&self.home, filter).unwrap_or_else(|error| {
            panic!("the test's user configuration cannot be written: {error}")
        });
    }

    /// `<home>/crash-dumps`, which the map names in
    /// [`CRASH_DUMP_VARIABLE`] for every browser the test launches; a file
    /// in it at teardown fails the test.
    pub fn crash_dump_folder(&self) -> &Path {
        &self.crash_dumps
    }

    /// The cleared environment of every child the test starts.
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// The test's seed (ADR 0060).
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The running suite.
    pub fn suite(&self) -> Suite {
        self.suite
    }

    /// [`START_TIME`], the same for every test.
    pub fn start_time(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(START_SECONDS)
    }

    /// `<prefix>-<seed>-<counter>`, for keychain services and service-manager
    /// units shared by the whole user.
    pub fn unique_name(&self, prefix: &str) -> String {
        let counter = self.names.get() + 1;
        self.names.set(counter);
        format!("{prefix}-{}-{counter}", self.seed)
    }

    /// A command with the environment cleared and the map applied, run in
    /// the test's folder, for [`TestEnv::spawn`].
    pub fn process_command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        let mut command = std::process::Command::new(program);
        command
            .env_clear()
            .envs(&self.env)
            .current_dir(self.folder());
        command
    }

    /// Starts `command` with the standard streams it sets, and records it,
    /// for a process whose output goes to a file, such as LazyFS's.
    #[cfg(target_os = "linux")]
    pub(crate) fn spawn_with_streams(
        &self,
        command: &mut std::process::Command,
    ) -> io::Result<Process> {
        let program = Path::new(command.get_program())
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let child = command.spawn()?;
        Ok(self.processes.record(program, child))
    }

    /// Starts a command with its standard input and output piped and records
    /// its process, which must have exited when the test's teardown runs.
    pub fn spawn(&self, command: &mut std::process::Command) -> io::Result<Process> {
        let program = Path::new(command.get_program())
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        Ok(self.processes.record(program, child))
    }
}

impl TestEnv {
    /// Keeps the test's folder after the value drops, as a failed test's
    /// is, and lists it in the test's output: `goodfetch-test-env` does so
    /// when the TypeScript or Python test it serves failed.
    pub fn keep(&mut self) {
        self.folder.disable_cleanup(true);
        #[expect(
            clippy::print_stderr,
            reason = "a failed test lists its kept folder in its output (ADR 0062)"
        )]
        {
            eprintln!("goodfetch-test-folder-kept={}", self.folder().display());
        }
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // Children first, so a browser that crashes while it is stopped has
        // finished writing its dump.
        let alive = self.processes.stop_alive();
        remove_socket_folders_outside(&self.home);
        let panicking = std::thread::panicking();
        let recorded = self.record.as_ref().map_or(Ok(()), |record| {
            record.append_logs(self.folder.path(), &self.home)
        });
        if let Err(error) = recorded
            && !panicking
        {
            self.folder.disable_cleanup(true);
            panic!(
                "the test's exercise record cannot take the logs of {}: {error}",
                self.home.display()
            );
        }
        let dumps = match crash_dumps::find(&self.crash_dumps) {
            Ok(dumps) => dumps,
            Err(_) if panicking => Vec::new(),
            Err(error) => {
                self.folder.disable_cleanup(true);
                panic!(
                    "the test's crash-dump folder {} cannot be listed: {error}",
                    self.crash_dumps.display()
                );
            }
        };
        if !panicking && alive.is_empty() && dumps.is_empty() {
            return;
        }
        self.folder.disable_cleanup(true);
        let alive = ProcessesAlive { programs: alive };
        let dumps = CrashDumps { dumps };
        #[expect(
            clippy::print_stderr,
            reason = "a failed test lists its kept folder and the failure's reasons, since a typed panic payload prints none (ADRs 0062 and 0069)"
        )]
        {
            eprintln!("goodfetch-test-folder-kept={}", self.folder().display());
            if !alive.programs.is_empty() {
                eprintln!("{alive}");
            }
            if !dumps.dumps.is_empty() {
                eprintln!("{dumps}");
            }
        }
        if panicking {
            return;
        }
        if alive.programs.is_empty() {
            std::panic::panic_any(dumps);
        }
        std::panic::panic_any(alive);
    }
}

/// Removes each socket folder a Daemon of `home` made under `/tmp`, its
/// socket's path in the home being too long for the OS, and left behind
/// when it was killed: a folder named `SOCKET_FOLDER_PREFIX…` that holds the
/// socket a runtime file of the home names.
fn remove_socket_folders_outside(home: &Path) {
    let Ok(majors) = std::fs::read_dir(home) else {
        return;
    };
    for major in majors.flatten() {
        let Ok(found) = RuntimeFile::read(&major.path().join(RUNTIME_FILE)) else {
            continue;
        };
        if let DaemonEndpoint::Unix(socket) = found.endpoint
            && !socket.starts_with(home)
            && let Some(folder) = socket.parent()
            && folder
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(SOCKET_FOLDER_PREFIX))
        {
            // Best effort: a folder another test removed is fine.
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

/// An `assert_cmd` command with the environment cleared and the test's map
/// applied, run in the test's folder.
pub fn command(env: &TestEnv, program: impl AsRef<OsStr>) -> assert_cmd::Command {
    assert_cmd::Command::from_std(env.process_command(program))
}

/// The memory file system an in-memory setup's folder lies in, where the
/// OS keeps one at a known place: `/dev/shm` on Linux.
fn memory_folder() -> Option<PathBuf> {
    let shm = Path::new("/dev/shm");
    (cfg!(target_os = "linux") && shm.is_dir()).then(|| shm.to_path_buf())
}

/// The test's name as a folder name every OS accepts.
fn folder_name(test: &str) -> String {
    test.replace("::", "-")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "-_.".contains(character) {
                character
            } else {
                '-'
            }
        })
        .take(NAME_LENGTH)
        .collect()
}

/// Writes `home`'s user configuration of the workspace's major: its format
/// version, then [`TEST_SETTINGS`] and `settings` under `[settings]`; when
/// `settings` sets the Console's address itself, its value alone.
/// Sets `log.level` to `filter` in `home`'s user configuration: its line
/// replaced, or one added under `[settings]`, every other line kept.
fn set_log_level(home: &Path, filter: &str) -> io::Result<()> {
    let file = home
        .join(major_folder(env!("CARGO_PKG_VERSION")))
        .join(USER_CONFIGURATION);
    let level = format!("log.level = \"{filter}\"");
    let mut lines: Vec<String> = std::fs::read_to_string(&file)?
        .lines()
        .filter(|line| !line.trim_start().starts_with("log.level"))
        .map(str::to_owned)
        .collect();
    match lines.iter().position(|line| line.trim() == "[settings]") {
        Some(table) => lines.insert(table + 1, level),
        None => lines.extend(["[settings]".to_owned(), level]),
    }
    replace(&file, &(lines.join("\n") + "\n"))
}

/// Writes `home`'s user configuration: [`TEST_SETTINGS`] and `log.level`
/// at `log_level`, each unless `settings` sets its key, then `settings`.
fn write_user_configuration(home: &Path, settings: &str, log_level: &str) -> io::Result<()> {
    let folder = home.join(major_folder(env!("CARGO_PKG_VERSION")));
    std::fs::create_dir_all(&folder)?;
    let level = format!("log.level = \"{log_level}\"\n");
    let defaults: String = TEST_SETTINGS
        .lines()
        .map(|line| format!("{line}\n"))
        .chain(std::iter::once(level))
        .filter(|line| {
            let key = line.split('=').next().unwrap_or_default().trim();
            !settings.contains(key)
        })
        .collect();
    replace(
        &folder.join(USER_CONFIGURATION),
        &format!("version = \"0.1\"\n\n[settings]\n{defaults}{settings}"),
    )
}

/// Replaces `file` with `text` at once: an engine reading it meanwhile, as
/// one the test started before does while the test starts another, reads
/// it before or after, never cut. The text and the rename are synced: a
/// LazyFS test's power cut after a write on its mounted home, such as the
/// `log.level` each engine start sets, leaves the file as written, never
/// empty.
fn replace(file: &Path, text: &str) -> io::Result<()> {
    let folder = file.parent().unwrap_or(Path::new("."));
    let mut replacement = tempfile::NamedTempFile::new_in(folder)?;
    io::Write::write_all(&mut replacement, text.as_bytes())?;
    replacement.as_file().sync_all()?;
    replacement.persist(file).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(folder)?.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn restrict_to_owner(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
}

/// Why a test's setup cannot start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TestEnvError {
    /// `GOODFETCH_TEST_SEED` cannot be read.
    #[error(transparent)]
    Seed(#[from] SeedError),
    /// `GOODFETCH_TEST_SUITE` names no suite.
    #[error(transparent)]
    Suite(#[from] SuiteError),
    /// The folder cannot be made.
    #[error("the test's folder cannot be made: {0}")]
    Folder(#[from] io::Error),
    /// A path of the folder is not Unicode, so no environment map holds it.
    #[error("the test's folder {} is not Unicode", .0.display())]
    NotUnicode(PathBuf),
}

#[cfg(test)]
mod tests;
