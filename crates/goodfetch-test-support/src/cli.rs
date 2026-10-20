//! The CLI helper (ADRs 0069, 0072 and 0073): `cli(&env, executable)` runs
//! the Product's executable a test gives, by path, under `assert_cmd` with the
//! test's cleared environment, `NO_COLOR=1`, `COLUMNS=100` and `TERM=dumb`
//! (the map already fixes the locale and time zone, `C.UTF-8` and `UTC`
//! unless the test named others); it returns one invocation's stdout, stderr
//! and exit code as one value, which `assert_cli_snapshot!` snapshots with
//! the shared filters; it fails when piped stdout or stderr holds an escape
//! byte; and it appends `{command, exit_code}` to the test's command record,
//! a JSON-lines file in the test's folder that `TestEnv` copies into the
//! test's exercise record, which `xtask features audit` reads.
//!
//! The command is the longest run of the test's arguments whose names,
//! joined by dots, are a `command:` Feature of `features/inventory.json`
//! (`browsers install` is `browsers.install`); arguments that hold none are
//! recorded as `unmatched`, which the audit reports.
#![expect(
    clippy::disallowed_methods,
    reason = "test support runs commands and writes the command record outside any async worker (ADR 0062)"
)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::env::{TestEnv, command, posix_locale};
use crate::exercise::{COMMAND_TARGET, command_record};
use crate::locales::{locales, time_zones};
use crate::snapshot::{SnapshotValues, snapshot_settings};

/// The Feature inventory, whose `command:` Features name the commands.
const INVENTORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../features/inventory.json");

/// The byte every ANSI escape sequence starts with.
const ESCAPE: u8 = 0x1B;

/// One run of a Product's executable, which [`Cli::run`] starts.
pub struct Cli<'env> {
    env: &'env TestEnv,
    command: assert_cmd::Command,
    arguments: Vec<String>,
    refused: Option<CliError>,
}

/// A run of `executable`, the path of the Product's executable the test
/// gives (`env!("CARGO_BIN_EXE_goodfetch-collie")` in its own `tests/it`),
/// in the test's folder with its cleared environment, `NO_COLOR=1`,
/// `COLUMNS=100` and `TERM=dumb`.
pub fn cli(env: &TestEnv, executable: impl AsRef<OsStr>) -> Cli<'_> {
    let mut command = command(env, executable);
    command
        .env("NO_COLOR", "1")
        .env("COLUMNS", "100")
        .env("TERM", "dumb");
    Cli {
        env,
        command,
        arguments: Vec::new(),
        refused: None,
    }
}

impl Cli<'_> {
    /// Adds one argument.
    #[must_use]
    pub fn arg(mut self, argument: impl AsRef<str>) -> Self {
        let argument = argument.as_ref();
        self.command.arg(argument);
        self.arguments.push(argument.to_owned());
        self
    }

    /// Adds these arguments.
    #[must_use]
    pub fn args<I>(self, arguments: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        arguments.into_iter().fold(self, Cli::arg)
    }

    /// Sets one more variable, over the test's map and the helper's own.
    #[must_use]
    pub fn env(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.command.env(name, value);
        self
    }

    /// Sets the child's `LANG` to a locale of the shared table (ADR 0073),
    /// given by its BCP 47 tag: `tr-TR` sets `LANG=tr_TR.UTF-8`. The test's
    /// map and the test's own process keep theirs. A tag outside the table
    /// fails the run with [`CliError::LocaleUnknown`].
    #[must_use]
    pub fn with_locale(mut self, tag: &str) -> Self {
        if locales().iter().any(|locale| locale == tag) {
            self.command.env("LANG", posix_locale(tag));
        } else {
            self.refused.get_or_insert(CliError::LocaleUnknown {
                tag: tag.to_owned(),
            });
        }
        self
    }

    /// Sets the child's `TZ` to a time zone of the shared table (ADR 0073),
    /// such as `Asia/Kathmandu`. The test's map and the test's own process
    /// keep theirs. A zone outside the table fails the run with
    /// [`CliError::TimeZoneUnknown`].
    #[must_use]
    pub fn with_time_zone(mut self, zone: &str) -> Self {
        if time_zones().iter().any(|case| case.name == zone) {
            self.command.env("TZ", zone);
        } else {
            self.refused.get_or_insert(CliError::TimeZoneUnknown {
                zone: zone.to_owned(),
            });
        }
        self
    }

    /// Runs the executable to its end, appends the command and its exit
    /// code to the test's command record, and returns its output; it fails
    /// when stdout or stderr is not UTF-8 or holds an escape byte, and
    /// before starting anything when a locale or time zone is outside the
    /// shared table.
    pub fn run(mut self) -> Result<CliOutput, CliError> {
        if let Some(refused) = self.refused {
            return Err(refused);
        }
        let output = self.command.output().map_err(CliError::Start)?;
        let exit = output.status.code();
        record_command(self.env, &self.arguments, exit)?;
        let stdout = text(Stream::Stdout, output.stdout)?;
        let stderr = text(Stream::Stderr, output.stderr)?;
        Ok(CliOutput {
            exit,
            stdout,
            stderr,
            values: SnapshotValues::of(self.env),
        })
    }
}

fn text(stream: Stream, bytes: Vec<u8>) -> Result<String, CliError> {
    if let Some(offset) = bytes.iter().position(|byte| *byte == ESCAPE) {
        return Err(CliError::EscapeByte {
            stream,
            offset,
            output: String::from_utf8_lossy(&bytes).escape_debug().to_string(),
        });
    }
    String::from_utf8(bytes).map_err(|_| CliError::NotUtf8 { stream })
}

/// Appends a command the test ran, with these arguments and this exit code
/// (none when a signal ended it), to the test's command record; the CLI
/// helper calls it for each run, and a test that starts the executable
/// itself, such as an Attached engine's, calls it once the process exited.
pub fn record_command(
    env: &TestEnv,
    arguments: &[impl AsRef<str>],
    exit: Option<i32>,
) -> Result<(), CliError> {
    let arguments: Vec<&str> = arguments.iter().map(AsRef::as_ref).collect();
    let fields = match command_path(&commands()?, &arguments) {
        Some(command) => json!({ "command": command, "exit_code": exit }),
        None => json!({ "unmatched": arguments.join(" "), "exit_code": exit }),
    };
    let line = json!({ "target": COMMAND_TARGET, "fields": fields });
    let path = command_record(env.folder());
    File::options()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|error| CliError::Record { path, error })
}

/// The `command:` Features' paths, from the inventory.
fn commands() -> Result<BTreeSet<String>, CliError> {
    let unreadable = |reason: String| CliError::Inventory {
        path: PathBuf::from(INVENTORY),
        reason,
    };
    let text = std::fs::read_to_string(INVENTORY).map_err(|error| unreadable(error.to_string()))?;
    let inventory: Value =
        serde_json::from_str(&text).map_err(|error| unreadable(error.to_string()))?;
    let features = inventory["features"]
        .as_array()
        .ok_or_else(|| unreadable("it holds no `features` list".to_owned()))?;
    Ok(features
        .iter()
        .filter_map(|feature| feature["id"].as_str()?.strip_prefix("command:"))
        .map(ToOwned::to_owned)
        .collect())
}

/// The longest run of `arguments`, joined by dots, that is one of
/// `commands`, from the first argument that starts one.
fn command_path(commands: &BTreeSet<String>, arguments: &[&str]) -> Option<String> {
    (0..arguments.len()).find_map(|start| {
        (start + 1..=arguments.len())
            .rev()
            .map(|end| arguments[start..end].join("."))
            .find(|path| commands.contains(path))
    })
}

/// One invocation's exit code, stdout and stderr, which
/// [`assert_cli_snapshot!`](crate::assert_cli_snapshot) snapshots as one
/// value.
#[derive(Clone, Debug)]
pub struct CliOutput {
    /// The exit code; none when a signal ended the process.
    pub exit: Option<i32>,
    /// Its stdout.
    pub stdout: String,
    /// Its stderr.
    pub stderr: String,
    values: SnapshotValues,
}

impl CliOutput {
    /// insta settings with the shared filters for the test's folder.
    pub fn settings(&self) -> insta::Settings {
        snapshot_settings(&self.values)
    }
}

impl fmt::Display for CliOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.exit {
            Some(code) => writeln!(formatter, "exit: {code}")?,
            None => writeln!(formatter, "exit: none, ended by a signal")?,
        }
        writeln!(formatter, "----- stdout -----")?;
        write_stream(formatter, &self.stdout)?;
        writeln!(formatter, "----- stderr -----")?;
        write_stream(formatter, &self.stderr)
    }
}

fn write_stream(formatter: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    formatter.write_str(text)?;
    if !text.is_empty() && !text.ends_with('\n') {
        writeln!(formatter)?;
    }
    Ok(())
}

/// Snapshots a [`CliOutput`] (its exit code, stdout and stderr as one value)
/// under the shared snapshot filters, as `insta::assert_snapshot!` does,
/// with an optional snapshot name first or an inline snapshot after.
#[macro_export]
macro_rules! assert_cli_snapshot {
    ($output:expr, @$snapshot:literal $(,)?) => {{
        let output: &$crate::CliOutput = &$output;
        output
            .settings()
            .bind(|| $crate::__insta::assert_snapshot!(output.to_string(), @$snapshot));
    }};
    ($output:expr $(,)?) => {{
        let output: &$crate::CliOutput = &$output;
        output
            .settings()
            .bind(|| $crate::__insta::assert_snapshot!(output.to_string()));
    }};
    ($name:expr, $output:expr $(,)?) => {{
        let output: &$crate::CliOutput = &$output;
        output
            .settings()
            .bind(|| $crate::__insta::assert_snapshot!($name, output.to_string()));
    }};
}

/// Which of a command's output streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// Its stdout.
    Stdout,
    /// Its stderr.
    Stderr,
}

impl fmt::Display for Stream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        })
    }
}

/// Why a CLI run fails.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    /// The executable cannot be started.
    #[error("the executable cannot be started: {0}")]
    Start(io::Error),
    /// Piped output holds an escape byte: colour or another terminal
    /// sequence leaks into a pipe (ADR 0073).
    #[error("the piped {stream} holds an escape byte at byte {offset}: {output}")]
    EscapeByte {
        /// The stream that holds it.
        stream: Stream,
        /// The byte's offset in it.
        offset: usize,
        /// The stream, escaped so the byte shows.
        output: String,
    },
    /// A stream is not UTF-8.
    #[error("the {stream} is not UTF-8")]
    NotUtf8 {
        /// The stream.
        stream: Stream,
    },
    /// The test's command record cannot be written.
    #[error("the command record {} cannot be written: {error}", path.display())]
    Record {
        /// The record's path.
        path: PathBuf,
        /// Why.
        error: io::Error,
    },
    /// The locale is not one of the shared table's (ADR 0073).
    #[error("the locale {tag} is not in the shared locale table")]
    LocaleUnknown {
        /// Its tag.
        tag: String,
    },
    /// The time zone is not one of the shared table's (ADR 0073).
    #[error("the time zone {zone} is not in the shared locale table")]
    TimeZoneUnknown {
        /// Its name.
        zone: String,
    },
    /// The Feature inventory cannot be read.
    #[error("the Feature inventory {} cannot be read: {reason}", path.display())]
    Inventory {
        /// Its path.
        path: PathBuf,
        /// Why.
        reason: String,
    },
}
