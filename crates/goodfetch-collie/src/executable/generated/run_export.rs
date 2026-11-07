//! `run export <run> <dataset> --format <f> [--compression gzip|zstd] --out
//! <path|->` (ADRs 0103 and 0116; spec #611's CLI): it calls `run.export`
//! for the Dataset its second positional argument names, then, on the same
//! Connection, `export.stream` of the Export it answered, and writes the
//! bytes that arrive to `--out`'s path or, for `-`, to standard output. A
//! path is written whole or not at all, under ADR 0116's rule, whichever
//! engine runs the call: through a hidden temporary file beside it, renamed
//! over it once every byte arrived, so a refusal or a dropped Connection
//! leaves it as it was. A path that cannot be written exits `13`,
//! `path_not_writable`, and storage that fails `14`, `file_write_failed`
//! of side `program`. Once written, it prints the Export, as JSON under
//! `--json`; standard output holding the bytes, `--out -` takes no `--json`.
//! A standard output that takes no more bytes stops `export.stream` at
//! once: a pipe whose reader left ends the command quietly, and any other
//! failure exits `14`, `file_write_failed` with no path.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Arg, ArgMatches, Command};
use goodfetch_engine::run_files::{SaveError, Saving};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::{ErrorCodeDefinition, Operation};
use goodfetch_protocol::checkpoint::{
    FILE_WRITE_FAILED, FileWriteFailedDetails, PATH_NOT_WRITABLE, PathNotWritableDetails, WriteSide,
};
use goodfetch_protocol::cli;
use goodfetch_protocol::entries::operations_of;
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::page::CHUNK;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::Call;
use crate::executable::engine_choice::{self, Answer, CommandCall, Daemons, Next};
use crate::executable::exit_codes;
use crate::executable::output::{self, Human};

/// The operation whose command this is.
pub(super) const OPERATION: &str = "run.export";

/// The params its own flags give.
pub(super) const REPLACES: &[&str] = &[DATASET];

/// The operation that sends an Export's bytes.
const EXPORT_STREAM: &str = "export.stream";

/// The param, and positional argument, that names the Dataset.
const DATASET: &str = "dataset";

/// The flag that says where the Export's file goes.
const OUT: &str = "out";

/// `command` with the Dataset as its second positional argument, after the
/// Run ID, and `--out`; `operation`'s params give the Dataset's help.
pub(super) fn args(command: Command, operation: &Operation) -> Command {
    let help = super::parameters(operation)
        .into_iter()
        .find(|parameter| parameter.name == DATASET)
        .map(|parameter| parameter.help)
        .unwrap_or_default();
    command
        .arg(
            Arg::new(DATASET)
                .value_name("DATASET")
                .required(true)
                .help(help),
        )
        .arg(
            Arg::new(OUT)
                .long(OUT)
                .value_name("PATH|-")
                .required(true)
                .value_parser(out)
                .help("Writes the Export's file to PATH, whole or not at all, or its bytes to standard output for `-`"),
        )
}

/// Where the Export's file goes.
#[derive(Clone, Debug)]
enum Out {
    /// Standard output, `-`.
    Stdout,
    /// A path, absolute.
    Path(PathBuf),
}

/// Reads `--out`: `-`, or a path, made absolute against the working
/// folder.
fn out(text: &str) -> Result<Out, String> {
    if text == "-" {
        return Ok(Out::Stdout);
    }
    std::path::absolute(text)
        .map(Out::Path)
        .map_err(|error| format!("`{text}` is no path: {error}"))
}

/// What `run export`'s own flags gave beside its params.
pub(super) struct Flags {
    /// `--out`.
    out: Out,
}

impl Flags {
    /// Reads the parse `matches`, adding the Dataset it names to `params`.
    pub(super) fn read(matches: &ArgMatches, params: &mut Map<String, Value>) -> Self {
        if let Some(dataset) = matches.get_one::<String>(DATASET) {
            params.insert(DATASET.to_owned(), Value::from(dataset.as_str()));
        }
        Self {
            out: matches.get_one::<Out>(OUT).cloned().unwrap_or(Out::Stdout),
        }
    }
}

/// Where the bytes that arrive go.
enum Destination {
    Stdout,
    File(Saving),
}

/// Runs `call`, the command `run export`, whose own flags gave `flags`, on
/// the engine its command line chooses, and answers its exit code.
pub(super) fn run(
    call: &Call,
    flags: &Flags,
    engine: &Engine,
    dependencies: Dependencies,
    daemons: &dyn Daemons,
    human: &Human,
) -> ExitCode {
    let mut destination = match &flags.out {
        Out::Stdout if call.json => {
            human.failure(
                "`--json` prints the Export on standard output, which `--out -` gives to its bytes: give `--out` a path",
                &[],
            );
            return ExitCode::from(cli::USAGE.code);
        }
        Out::Stdout => Destination::Stdout,
        Out::Path(path) => match Saving::create(path) {
            Ok(saving) => Destination::File(saving),
            Err(error) => return call.fail(save_failed(error, path), human),
        },
    };
    let stream = operations_of(call.product).find(|operation| operation.name == EXPORT_STREAM);
    let mut export: Option<Value> = None;
    let mut next = |answer: &Value| -> Option<Next> {
        if export.is_some() {
            return None;
        }
        export = Some(answer.clone());
        Some(Next {
            method: stream.map_or(EXPORT_STREAM, |operation| operation.name),
            params: json!({ "export_id": answer["export_id"] }),
            cancellable: stream.is_some_and(|operation| operation.cancellable),
            followed: true,
        })
    };
    let mut failed: Option<SaveError> = None;
    // Whether stdout's reader left, a closed pipe, which ends the command
    // quietly once the call stopped.
    let mut reader_left = false;
    let mut write = |method: &str, sent: &Value| -> Option<u32> {
        if method != CHUNK {
            return Some(0);
        }
        let bytes = sent["data"]
            .as_str()
            .and_then(|data| data_encoding::BASE64.decode(data.as_bytes()).ok())
            .unwrap_or_default();
        match &mut destination {
            // A stdout that takes no more stops the call at once.
            Destination::Stdout => match output::bytes(&bytes) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                    reader_left = true;
                    return None;
                }
                Err(error) => {
                    failed = Some(SaveError::of_storage(&error, "write standard output"));
                    return None;
                }
            },
            Destination::File(saving) if failed.is_none() => {
                failed = saving.write(&bytes).err();
            }
            // Once a write failed, the bytes left are read and dropped, so
            // the engine ends the call.
            Destination::File(_) => {}
        }
        Some(1)
    };
    let command_call = CommandCall {
        method: call.operation.name,
        params: Value::Object(call.params.clone()),
        cancellable: call.operation.cancellable,
        follower: Some(&mut write),
        pager: Some(&mut next),
        input: None,
        stopper: None,
    };
    let progress = human.progress("run export");
    let warn = |text: &str| human.warning(text);
    let reach = call.reach(daemons, &warn);
    let answer = engine_choice::call(engine, dependencies, call.product, &reach, command_call);
    drop(progress);
    if let (Out::Stdout, false) = (&flags.out, matches!(answer, Answer::Interrupted(_))) {
        // The call stopped because stdout took no more: what the engine
        // answered then is no news.
        if let Some(error) = failed.take() {
            return call.fail(stdout_failed(error), human);
        }
        if reader_left {
            return ExitCode::from(exit_codes::SUCCESS);
        }
    }
    match answer {
        Answer::Result(_) => finished(
            call,
            &flags.out,
            (failed, destination),
            export.as_ref(),
            human,
        ),
        Answer::Error(error) => call.fail(error, human),
        Answer::Refused(error) => call.fail_exiting(error, exit_codes::DAEMON_REFUSED, human),
        Answer::Unanswered(error) => output::fail(error, None, call.json, human),
        Answer::Interrupted(interrupt) => ExitCode::from(interrupt.exit_code()),
    }
}

/// The exit code of `call` once `export.stream` ended: for a path, the
/// file `destination` wrote finished, unless a write `failed`, and the Export
/// `export` printed.
fn finished(
    call: &Call,
    out: &Out,
    (failed, destination): (Option<SaveError>, Destination),
    export: Option<&Value>,
    human: &Human,
) -> ExitCode {
    if let Out::Path(path) = out {
        let written = match (failed, destination) {
            (Some(error), _) => Err(error),
            (None, Destination::File(saving)) => saving.finish(),
            (None, Destination::Stdout) => Ok(()),
        };
        if let Err(error) = written {
            return call.fail(save_failed(error, path), human);
        }
        if let Some(export) = export {
            call.print(export, human);
        }
    }
    ExitCode::from(exit_codes::SUCCESS)
}

/// The failure of a write of the Export's bytes to standard output that
/// failed with `error` (ADR 0116): `file_write_failed` of side `program`,
/// which names no path.
fn stdout_failed(error: SaveError) -> ErrorObject {
    let (reason, os_code, message) = match error {
        SaveError::Path {
            reason,
            os_code,
            message,
        }
        | SaveError::Storage {
            reason,
            os_code,
            message,
        }
        | SaveError::Source {
            reason,
            os_code,
            message,
        } => (reason, os_code, message),
        SaveError::Cancelled => ("other", None, "the write stopped".to_owned()),
    };
    failure(
        &FILE_WRITE_FAILED,
        format!(
            "The Export's bytes could not be written to standard output ({message}); what it holds is incomplete. Free space where it goes, then run the command again."
        ),
        &FileWriteFailedDetails {
            reason: reason.to_owned(),
            os_code: os_code.map(str::to_owned),
            path: None,
            side: WriteSide::Program,
        },
    )
}

/// The failure of a write of `path` that failed with `error` (ADR 0116):
/// `path_not_writable` for a reason the path causes, `file_write_failed`
/// of side `program` for the storage's.
fn save_failed(error: SaveError, path: &Path) -> ErrorObject {
    let shown = path.display().to_string();
    match error {
        SaveError::Path {
            reason,
            os_code,
            message,
        } => failure(
            &PATH_NOT_WRITABLE,
            format!("The path `{shown}` cannot be written ({message}); nothing was written there."),
            &PathNotWritableDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: shown,
                fix: "Give `--out` a path in a folder this user can write, whose parents are folders.".to_owned(),
            },
        ),
        SaveError::Storage {
            reason,
            os_code,
            message,
        }
        | SaveError::Source {
            reason,
            os_code,
            message,
        } => failure(
            &FILE_WRITE_FAILED,
            format!(
                "The Export could not be written at `{shown}` ({message}); the path holds what it held before. Free space there, then run the command again."
            ),
            &FileWriteFailedDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: Some(shown),
                side: WriteSide::Program,
            },
        ),
        SaveError::Cancelled => failure(
            &FILE_WRITE_FAILED,
            format!("The Export's write at `{shown}` stopped; the path holds what it held before."),
            &FileWriteFailedDetails {
                reason: "other".to_owned(),
                os_code: None,
                path: Some(shown),
                side: WriteSide::Program,
            },
        ),
    }
}

/// The failure of Error code `definition` with `message` and `details`.
fn failure(
    definition: &ErrorCodeDefinition,
    message: String,
    details: &impl Serialize,
) -> ErrorObject {
    let details = match serde_json::to_value(details) {
        Ok(Value::Object(details)) => details,
        _ => Map::new(),
    };
    ErrorObject {
        code: definition.group.json_rpc_class(),
        message,
        data: ErrorData {
            code: definition.name.to_owned(),
            retryable: definition.retryable,
            details,
        },
    }
}
