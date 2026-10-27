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
//! (`browser install` is `browser.install`); arguments that hold none are
//! recorded as `unmatched`, which the audit reports.
//!
//! `json(schema)` checks a `--json` run: stdout holds exactly one document,
//! valid against that schema of the committed JSON Schema bundle, read as a
//! file; `json_lines(schema)` checks that every line of stdout is one
//! document of that event type. The caller names the type, so test support
//! stays on values. `mode(Mode::Attached)` runs the command with
//! `--attached`, on an Attached engine in the executable's own process, and
//! `mode(Mode::Daemon)` as a Client of a Daemon of the test's home, which
//! the run starts and stops, so one rstest case per mode shares one
//! snapshot; with no mode, the command runs with the arguments the test
//! gave, on the engine the executable chooses.
#![expect(
    clippy::disallowed_methods,
    reason = "test support runs commands and writes the command record outside any async worker (ADR 0062)"
)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::daemon::{DAEMON_DEADLINE, Daemon, DaemonError};
use crate::env::TestEnv;
use crate::exercise::{COMMAND_TARGET, command_record};
use crate::locales::{table_lang, table_zone};
use crate::snapshot::{SnapshotValues, snapshot_settings};

/// The Feature inventory, whose `command:` Features name the commands.
const INVENTORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../features/inventory.json");

/// The JSON Schema bundle `xtask codegen` commits, whose `$defs` hold each
/// `--json` output's and each event's type.
const BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../goodfetch-protocol/generated/bundle.json"
);

/// The flag that makes a command a Client of the Daemon (ADR 0024).
const DAEMON_FLAG: &str = "--daemon";

/// The flag that runs a command on an Attached engine in the executable's
/// own process (ADR 0024).
const ATTACHED_FLAG: &str = "--attached";

/// The byte every ANSI escape sequence starts with.
const ESCAPE: u8 = 0x1B;

/// A command line of a Product's executable as a test builds it: the
/// command, and the arguments its command record names. [`cli`] and
/// [`pty`](crate::pty) build theirs through it, so each records exactly the
/// arguments its child got, the ones they add themselves (`--daemon`,
/// `--color always`) among them.
pub(crate) struct Invocation {
    command: std::process::Command,
    arguments: Vec<String>,
}

impl Invocation {
    /// The command line of `program` in the test's folder, with its cleared
    /// environment and these variables over the test's map.
    pub(crate) fn of(
        env: &TestEnv,
        program: impl AsRef<OsStr>,
        variables: &[(&str, &str)],
    ) -> Self {
        let mut command = env.process_command(program);
        command.envs(variables.iter().copied());
        Self {
            command,
            arguments: Vec::new(),
        }
    }

    /// Adds one argument.
    pub(crate) fn arg(&mut self, argument: &str) {
        self.command.arg(argument);
        self.arguments.push(argument.to_owned());
    }

    /// Sets one more variable.
    pub(crate) fn env(&mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        self.command.env(name, value);
    }

    /// The command to start, and the arguments to record once it ended.
    pub(crate) fn into_parts(self) -> (std::process::Command, Vec<String>) {
        (self.command, self.arguments)
    }
}

/// One run of a Product's executable, which [`Cli::run`] starts.
pub struct Cli<'env> {
    env: &'env TestEnv,
    invocation: Invocation,
    stdin: Option<Vec<u8>>,
    refused: Option<CliError>,
    executable: PathBuf,
    mode: Option<Mode>,
}

/// How a command reaches the engine (ADRs 0024 and 0069): every test of a
/// command that runs the engine has one rstest case per mode, which share
/// one snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// In the executable's own process: the run adds `--attached` after
    /// the test's arguments, so even a command whose value outlives it,
    /// which reaches the Daemon by default, starts none.
    Attached,
    /// As a Client of the Daemon of the test's home: the run starts the
    /// executable's Daemon, adds `--daemon` after the test's arguments,
    /// and stops the Daemon once the command ended, failing when it still
    /// runs or left its runtime file.
    Daemon,
}

/// A run of `executable`, the path of the Product's executable the test
/// gives (`env!("CARGO_BIN_EXE_goodfetch-collie")` in its own `tests/it`),
/// in the test's folder with its cleared environment, `NO_COLOR=1`,
/// `COLUMNS=100` and `TERM=dumb`.
pub fn cli(env: &TestEnv, executable: impl AsRef<OsStr>) -> Cli<'_> {
    Cli {
        env,
        invocation: Invocation::of(
            env,
            &executable,
            &[("NO_COLOR", "1"), ("COLUMNS", "100"), ("TERM", "dumb")],
        ),
        stdin: None,
        refused: None,
        executable: PathBuf::from(executable.as_ref()),
        mode: None,
    }
}

impl Cli<'_> {
    /// Adds one argument.
    #[must_use]
    pub fn arg(mut self, argument: impl AsRef<str>) -> Self {
        self.invocation.arg(argument.as_ref());
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

    /// Gives the executable `input` on its standard input, which is empty
    /// otherwise, as `--settings -` reads it.
    #[must_use]
    pub fn stdin(mut self, input: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(input.into());
        self
    }

    /// Sets one more variable, over the test's map and the helper's own.
    #[must_use]
    pub fn env(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.invocation.env(name, value);
        self
    }

    /// Sets the child's `LANG` to a locale of the shared table (ADR 0073),
    /// given by its BCP 47 tag: `tr-TR` sets `LANG=tr_TR.UTF-8`. The test's
    /// map and the test's own process keep theirs. A tag outside the table
    /// fails the run with [`CliError::LocaleUnknown`].
    #[must_use]
    pub fn with_locale(mut self, tag: &str) -> Self {
        match table_lang(tag) {
            Ok(lang) => {
                self.invocation.env("LANG", lang);
            }
            Err(_) => {
                self.refused.get_or_insert(CliError::LocaleUnknown {
                    tag: tag.to_owned(),
                });
            }
        }
        self
    }

    /// Sets the child's `TZ` to a time zone of the shared table (ADR 0073),
    /// such as `Asia/Kathmandu`. The test's map and the test's own process
    /// keep theirs. A zone outside the table fails the run with
    /// [`CliError::TimeZoneUnknown`].
    #[must_use]
    pub fn with_time_zone(mut self, zone: &str) -> Self {
        if table_zone(zone).is_ok() {
            self.invocation.env("TZ", zone);
        } else {
            self.refused.get_or_insert(CliError::TimeZoneUnknown {
                zone: zone.to_owned(),
            });
        }
        self
    }

    /// Runs the command in this mode; unless set, with the arguments the
    /// test gave, on the engine the executable chooses.
    #[must_use]
    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Runs the executable as [`Cli::run`] does, then checks that its
    /// stdout holds exactly one JSON document, valid against `schema`, a
    /// type of the committed bundle's `$defs` (`SettingsExplanation` for
    /// `config explain --json`). A schema the bundle lacks fails before
    /// anything starts.
    pub fn json(self, schema: &str) -> Result<CliJson, CliError> {
        let validator = validator(schema)?;
        let output = self.run()?;
        let document = one_document(&output.stdout)?;
        check(&validator, schema, None, &document)?;
        Ok(CliJson::of(output, vec![document]))
    }

    /// Runs the executable as [`Cli::run`] does, then checks that its
    /// stdout holds exactly one JSON array, each item valid against
    /// `schema`, a type of the committed bundle's `$defs`: what a command
    /// of a sequence prints under `--json` once its call ended, such as
    /// `run records` (ADR 0099).
    pub fn json_array(self, schema: &str) -> Result<CliJson, CliError> {
        let validator = validator(schema)?;
        let output = self.run()?;
        let document = one_document(&output.stdout)?;
        let items = match &document {
            Value::Array(items) => items.as_slice(),
            _ => std::slice::from_ref(&document),
        };
        let mut errors: Vec<String> = Vec::new();
        if !document.is_array() {
            errors.push("/: the document is no array".to_owned());
        }
        for (index, item) in items.iter().enumerate() {
            errors.extend(
                validator
                    .iter_errors(item)
                    .map(|error| format!("/{index}{}: {error}", error.instance_path())),
            );
        }
        if !errors.is_empty() {
            return Err(CliError::SchemaViolated {
                schema: schema.to_owned(),
                line: None,
                errors,
                document,
            });
        }
        Ok(CliJson::of(output, vec![document]))
    }

    /// Runs the executable as [`Cli::run`] does, then checks that each
    /// line of its stdout is one JSON document valid against `schema`, the
    /// command's declared event type, and nothing else is: JSON Lines. A
    /// stdout with no line fails too, since a stream command that prints
    /// nothing, its output gone elsewhere, would otherwise pass; a test that
    /// expects no event checks [`Cli::run`]'s empty stdout instead.
    pub fn json_lines(self, schema: &str) -> Result<CliJson, CliError> {
        let validator = validator(schema)?;
        let output = self.run()?;
        if output.stdout.is_empty() {
            return Err(CliError::NotJson {
                line: None,
                reason: "stdout holds no line".to_owned(),
                stdout: output.stdout,
            });
        }
        let mut documents = Vec::new();
        for (index, line) in output.stdout.lines().enumerate() {
            let number = Some(index + 1);
            let document = serde_json::from_str(line).map_err(|error| CliError::NotJson {
                line: number,
                reason: error.to_string(),
                stdout: output.stdout.clone(),
            })?;
            check(&validator, schema, number, &document)?;
            documents.push(document);
        }
        Ok(CliJson::of(output, documents))
    }

    /// Runs the executable to its end, appends the command and its exit
    /// code to the test's command record, and returns its output; in
    /// [`Mode::Daemon`] it starts the executable's Daemon first and stops
    /// it once the command ended, failing with [`CliError::Daemon`] when
    /// either fails. It fails
    /// when stdout or stderr is not UTF-8 or holds an escape byte, and
    /// before starting anything when a locale or time zone is outside the
    /// shared table.
    pub fn run(mut self) -> Result<CliOutput, CliError> {
        if let Some(refused) = self.refused {
            return Err(refused);
        }
        let daemon = match self.mode {
            None => None,
            Some(Mode::Attached) => {
                self = self.arg(ATTACHED_FLAG);
                None
            }
            Some(Mode::Daemon) => {
                self = self.arg(DAEMON_FLAG);
                Some(Daemon::start_program(
                    self.env,
                    &self.executable,
                    DAEMON_DEADLINE,
                )?)
            }
        };
        let (command, arguments) = self.invocation.into_parts();
        let mut command = assert_cmd::Command::from_std(command);
        if let Some(input) = self.stdin {
            command.write_stdin(input);
        }
        let output = command.output().map_err(CliError::Start);
        if let Some(daemon) = daemon {
            daemon.stop(self.env)?;
        }
        let output = output?;
        let exit = output.status.code();
        record_command(self.env, &arguments, exit)?;
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

/// A validator of `schema`, one type of the committed bundle's `$defs`.
fn validator(schema: &str) -> Result<jsonschema::Validator, CliError> {
    let unreadable = |reason: String| CliError::Bundle {
        path: PathBuf::from(BUNDLE),
        reason,
    };
    let text = std::fs::read_to_string(BUNDLE).map_err(|error| unreadable(error.to_string()))?;
    let bundle: Value =
        serde_json::from_str(&text).map_err(|error| unreadable(error.to_string()))?;
    let defs = bundle["$defs"]
        .as_object()
        .ok_or_else(|| unreadable("it holds no `$defs`".to_owned()))?;
    if !defs.contains_key(schema) {
        return Err(CliError::SchemaUnknown {
            schema: schema.to_owned(),
        });
    }
    let root = json!({
        "$schema": bundle["$schema"],
        "$defs": defs,
        "$ref": format!("#/$defs/{schema}"),
    });
    jsonschema::validator_for(&root).map_err(|error| unreadable(error.to_string()))
}

/// The one JSON document `stdout` holds, with nothing but whitespace
/// around it.
fn one_document(stdout: &str) -> Result<Value, CliError> {
    let not_json = |reason: String| CliError::NotJson {
        line: None,
        reason,
        stdout: stdout.to_owned(),
    };
    let mut documents = serde_json::Deserializer::from_str(stdout).into_iter::<Value>();
    let document = match documents.next() {
        Some(Ok(document)) => document,
        Some(Err(error)) => return Err(not_json(error.to_string())),
        None => return Err(not_json("stdout holds no document".to_owned())),
    };
    match documents.next() {
        None => Ok(document),
        Some(Ok(_)) => Err(CliError::SecondDocument {
            stdout: stdout.to_owned(),
        }),
        Some(Err(error)) => Err(not_json(error.to_string())),
    }
}

/// Fails when `document`, stdout's or its line `line`'s, violates `schema`.
fn check(
    validator: &jsonschema::Validator,
    schema: &str,
    line: Option<usize>,
    document: &Value,
) -> Result<(), CliError> {
    let errors: Vec<String> = validator
        .iter_errors(document)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CliError::SchemaViolated {
            schema: schema.to_owned(),
            line,
            errors,
            document: document.clone(),
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
    record_command_in(env.folder(), arguments, exit)
}

/// [`record_command`] for the test whose [`TestEnv::folder`] is `folder`,
/// for a handle that outlives no borrow of its `TestEnv`, such as a
/// foreground Daemon a Journey's steps share.
pub fn record_command_in(
    folder: &Path,
    arguments: &[impl AsRef<str>],
    exit: Option<i32>,
) -> Result<(), CliError> {
    let arguments: Vec<&str> = arguments.iter().map(AsRef::as_ref).collect();
    let fields = match command_path(&commands()?, &arguments) {
        Some(command) => json!({ "command": command, "exit_code": exit }),
        None => json!({ "unmatched": arguments.join(" "), "exit_code": exit }),
    };
    let line = json!({ "target": COMMAND_TARGET, "fields": fields });
    let path = command_record(folder);
    File::options()
        .create(true)
        .append(true)
        .open(&path)
        // One write, its newline included: commands run at once append
        // whole lines.
        .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()))
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

/// A `--json` run that [`Cli::json`] or [`Cli::json_lines`] checked.
#[derive(Clone, Debug)]
pub struct CliJson {
    /// The run's exit code and stderr, with stdout as its documents
    /// pretty-printed one after another, which
    /// [`assert_cli_snapshot!`](crate::assert_cli_snapshot) snapshots.
    pub output: CliOutput,
    /// The documents stdout held: the one document, or each line's.
    pub documents: Vec<Value>,
    /// Stdout as the command printed it. The snapshot's pretty-printed
    /// documents leave out its layout, key order and number spelling, so a
    /// test that runs one command in several modes compares these bytes
    /// across them.
    pub printed: String,
}

impl CliJson {
    fn of(output: CliOutput, documents: Vec<Value>) -> Self {
        let stdout = documents
            .iter()
            .flat_map(|document| [format!("{document:#}"), "\n".to_owned()])
            .collect();
        Self {
            printed: output.stdout.clone(),
            output: CliOutput { stdout, ..output },
            documents,
        }
    }
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
    /// The Daemon of a [`Mode::Daemon`] run did not start, or did not stop
    /// cleanly.
    #[error("the Daemon of the run's mode: {0}")]
    Daemon(#[from] DaemonError),
    /// The committed JSON Schema bundle cannot be read.
    #[error("the JSON Schema bundle {} cannot be read: {reason}", path.display())]
    Bundle {
        /// Its path.
        path: PathBuf,
        /// Why.
        reason: String,
    },
    /// The bundle's `$defs` hold no such type.
    #[error("the JSON Schema bundle holds no type {schema}")]
    SchemaUnknown {
        /// The type's name.
        schema: String,
    },
    /// Stdout, or one of its lines, is not exactly one JSON document: a
    /// stray line, no document at all, or a blank line among JSON Lines.
    #[error("stdout{} is not one JSON document ({reason}): {stdout:?}", line.map(|line| format!("'s line {line}")).unwrap_or_default())]
    NotJson {
        /// The line, among JSON Lines.
        line: Option<usize>,
        /// Why.
        reason: String,
        /// The whole stdout.
        stdout: String,
    },
    /// Stdout holds a second document after the one a `--json` command
    /// prints.
    #[error("stdout holds a second JSON document: {stdout:?}")]
    SecondDocument {
        /// The whole stdout.
        stdout: String,
    },
    /// A document violates the type the test named.
    #[error("stdout{}'s document violates {schema}: {}; {document}", line.map(|line| format!(" line {line}")).unwrap_or_default(), errors.join("; "))]
    SchemaViolated {
        /// The type.
        schema: String,
        /// The line, among JSON Lines.
        line: Option<usize>,
        /// Each violation, at its place in the document.
        errors: Vec<String>,
        /// The document.
        document: Value,
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
