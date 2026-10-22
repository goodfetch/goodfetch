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
use crate::process::{Process, ProcessTracker, ProcessesAlive};
use crate::runtime_file::major_folder;
use crate::variables::{
    COVERAGE_VARIABLE, SeedError, Suite, SuiteError, Variables, WINDOWS_VARIABLES, current_test,
};

/// Every test's start time, 2026-01-05T09:00:00Z: a Monday outside any DST
/// change in the shared time zones. A test that needs another passes its own.
pub const START_TIME: &str = "2026-01-05T09:00:00Z";

/// [`START_TIME`] in seconds since the Unix epoch.
const START_SECONDS: u64 = 1_767_603_600;

/// Where an engine started with the map writes its logs: JSON lines in the
/// Goodfetch home's log folder (ADR 0122), standing in for `log.output:
/// file` until the engine reads its configuration.
const LOG_OUTPUT: (&str, &str) = ("GOODFETCH_LOG_OUTPUT", "file");

/// Which logs it writes: `rpc`'s dispatch records and the engine's outcome
/// records, which ADR 0072's exercise audit reads, beside the default level.
fn log_level() -> String {
    format!("info,{DISPATCH_TARGET}=trace,{OUTCOME_TARGET}=trace")
}

/// How long a folder name keeps the test's name.
const NAME_LENGTH: usize = 64;

/// The settings every test's user configuration starts with: the Console
/// of every Daemon the test starts listens on a port the OS chooses, never
/// on its major's fixed port (ADR 0059).
pub const TEST_SETTINGS: &str = "console.address = \"127.0.0.1:0\"\n";

/// The user configuration's file in the per-major folder.
const USER_CONFIGURATION: &str = "configuration.toml";

/// One test's isolated setup. Its folder is removed when it drops, unless
/// the test failed: then it is kept and listed on stderr. Dropping it also
/// fails the test if a process it started is still alive, or if its
/// crash-dump folder holds a file. Its map turns on the JSON log of every
/// engine the test starts, whose lines it copies into the test's exercise
/// record when it drops, under `just test`.
pub struct TestEnv {
    folder: tempfile::TempDir,
    home: PathBuf,
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

    /// The setup of the test with this full name, from these variables.
    /// Prints `goodfetch-test-seed=<n>` to stderr once the seed is known,
    /// before anything else, so a test killed on a timeout still shows it.
    pub fn create(test: &str, variables: &Variables) -> Result<Self, TestEnvError> {
        let seed = variables.seed(test)?;
        #[expect(
            clippy::print_stderr,
            reason = "the seed is printed when the test starts (ADR 0060)"
        )]
        {
            eprintln!("goodfetch-test-seed={seed}");
        }
        let suite = variables.suite()?;
        let folder = tempfile::Builder::new()
            .prefix(&format!("{} é ", folder_name(test)))
            .tempdir()?;
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
        restrict_to_owner(&root.join("runtime"))?;
        write_user_configuration(&home, "")?;
        env.insert("TZ".to_owned(), "UTC".to_owned());
        env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
        env.insert(LOG_OUTPUT.0.to_owned(), LOG_OUTPUT.1.to_owned());
        env.insert("GOODFETCH_LOG".to_owned(), log_level());
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
            crash_dumps,
            env,
            seed,
            suite,
            names: Cell::new(0),
            processes: ProcessTracker::default(),
            record,
        })
    }

    /// The same setup with `LANG` for this locale tag (`tr-TR` becomes
    /// `tr_TR.UTF-8`).
    #[must_use]
    pub fn with_locale(mut self, tag: &str) -> Self {
        self.env.insert("LANG".to_owned(), posix_locale(tag));
        self
    }

    /// The same setup with `TZ` for this time zone.
    #[must_use]
    pub fn with_time_zone(mut self, zone: &str) -> Self {
        self.env.insert("TZ".to_owned(), zone.to_owned());
        self
    }

    /// The same setup with one more variable in the map every child gets,
    /// such as `GOODFETCH_LOG=info` for a run on a pseudo-terminal whose
    /// transcript two runs compare (the trace lines carry their instant).
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
    /// under `[settings]`, after [`TEST_SETTINGS`], which every test's home
    /// starts with, unless `settings` sets `console.address` itself.
    ///
    /// # Panics
    ///
    /// If the file cannot be written.
    pub fn user_configuration(&self, settings: &str) {
        write_user_configuration(&self.home, settings).unwrap_or_else(|error| {
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

/// An `assert_cmd` command with the environment cleared and the test's map
/// applied, run in the test's folder.
pub fn command(env: &TestEnv, program: impl AsRef<OsStr>) -> assert_cmd::Command {
    assert_cmd::Command::from_std(env.process_command(program))
}

/// The `LANG` value of a BCP 47 locale tag: `tr-TR` is `tr_TR.UTF-8`.
pub(crate) fn posix_locale(tag: &str) -> String {
    format!("{}.UTF-8", tag.replace('-', "_"))
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
fn write_user_configuration(home: &Path, settings: &str) -> io::Result<()> {
    let folder = home.join(major_folder(env!("CARGO_PKG_VERSION")));
    std::fs::create_dir_all(&folder)?;
    let defaults = if settings.contains("console.address") {
        ""
    } else {
        TEST_SETTINGS
    };
    std::fs::write(
        folder.join(USER_CONFIGURATION),
        format!("version = \"0.1\"\n\n[settings]\n{defaults}{settings}"),
    )
}

#[cfg(unix)]
fn restrict_to_owner(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_to_owner(_folder: &Path) -> io::Result<()> {
    Ok(())
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
