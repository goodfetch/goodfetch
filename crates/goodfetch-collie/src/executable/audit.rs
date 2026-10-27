//! `audit export` and `audit verify` (ADRs 0016 and 0067): the user's
//! Audit log leaves the machine as JSONL, one entry with its hash per line,
//! in sequence, which an operator's log stack keeps, and `audit verify`
//! checks either the log itself or such an export, `--file`, which it reads
//! and verifies in this process, so a copy kept anywhere verifies without
//! the log it came from. Each reads the log through `audit.export` and
//! `audit.verify` on an Attached engine in this process; `audit verify`
//! prints what `audit.verify` answers, as JSON under `--json`, and a
//! failure prints as every command's does.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use goodfetch_cli::engine_choice::{self, Answer, CommandCall, Daemons};
use goodfetch_cli::exit_codes;
use goodfetch_cli::generated::{Joined, Joining};
use goodfetch_cli::output::{self, Human};
use goodfetch_engine::audit::verify_exported;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::audit::{AUDIT_EXPORT, AUDIT_VERIFY, AuditVerified};
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::cli::{CommandError, CommandFailed};
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject, ProtocolFailure};
use serde_json::{Map, Value, json};

// The `audit` group's commands: the catalog's `audit` commands this group
// does not write itself, listed first, then its own. No doc comment: clap
// would make it the group's help.
#[derive(Subcommand)]
pub(super) enum AuditCommand {
    #[command(flatten)]
    Generated(Joining<OwnAuditCommand>),
    #[command(flatten)]
    Own(OwnAuditCommand),
}

// The `audit` group's hand-written commands; no doc comment either.
#[derive(Subcommand)]
pub(super) enum OwnAuditCommand {
    /// Writes the user's Audit log to stdout as JSONL: one entry per line,
    /// in sequence, each with the hash that chains it to the line before
    /// it, the anchor first when `audit.retention` pruned its beginning.
    /// `audit verify --file` verifies what it wrote.
    Export {
        /// Prints a failure as JSON on stdout; the entries are JSON either
        /// way.
        #[arg(long)]
        json: bool,
    },
    /// Verifies the user's Audit log, or with `--file` an export of it:
    /// every entry's hash chains it to the one before it, from the first or
    /// from the anchor a pruning left, and the log's head file names its
    /// last. Prints that it is intact, or the first sequence that does not
    /// verify, so an edited, deleted, reordered or truncated entry shows.
    Verify {
        /// A file `audit export` wrote, verified in place of the log; it
        /// has no head file, so its tail is its last line.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

impl Joined for OwnAuditCommand {
    const NAMESPACE: &'static str = "audit";
}

/// Runs the `audit` command `command`.
pub(super) fn run(
    command: AuditCommand,
    engine: &Engine,
    dependencies: Dependencies,
    daemons: &dyn Daemons,
    human: &Human,
) -> ExitCode {
    match command {
        AuditCommand::Generated(joining) => {
            joining.call().run(engine, dependencies, daemons, human)
        }
        AuditCommand::Own(OwnAuditCommand::Export { json }) => {
            export(engine, dependencies, json, human)
        }
        AuditCommand::Own(OwnAuditCommand::Verify { file: None, json }) => {
            match call(engine, dependencies, &AUDIT_VERIFY, json, human) {
                Ok(verified) => print(&verified, json, human),
                Err(exit) => exit,
            }
        }
        AuditCommand::Own(OwnAuditCommand::Verify {
            file: Some(file),
            json,
        }) => verify_file(&file, json, human),
    }
}

/// Writes every entry `audit.export` answers on `engine`, one JSON line
/// each.
fn export(engine: &Engine, dependencies: Dependencies, json: bool, human: &Human) -> ExitCode {
    let exported = match call(engine, dependencies, &AUDIT_EXPORT, json, human) {
        Ok(exported) => exported,
        Err(exit) => return exit,
    };
    for entry in exported["entries"].as_array().into_iter().flatten() {
        output::json(entry);
    }
    ExitCode::from(exit_codes::SUCCESS)
}

/// Verifies the export `file` holds and prints what it found.
fn verify_file(file: &Path, json: bool, human: &Human) -> ExitCode {
    #[expect(
        clippy::disallowed_methods,
        reason = "the command reads the export it verifies before any runtime starts"
    )]
    let read = std::fs::read(file);
    match read {
        Ok(jsonl) => print(&json!(verify_exported(&jsonl)), json, human),
        Err(error) => {
            let failure = ProtocolFailure::InvalidParams;
            let unread = ErrorObject {
                code: failure.code(),
                message: format!("the export `{}` cannot be read: {error}", file.display()),
                data: ErrorData {
                    code: failure.name().to_owned(),
                    retryable: false,
                    details: Map::new(),
                },
            };
            fail(&unread, &AUDIT_VERIFY, json, human)
        }
    }
}

/// Prints what verifying a log found, `verified`: as JSON under `--json`,
/// else each field on its line.
fn print(verified: &Value, json: bool, human: &Human) -> ExitCode {
    if json {
        output::json(verified);
    } else if let Ok(verified) = serde_json::from_value::<AuditVerified>(verified.clone()) {
        human.field("intact", &verified.intact.to_string());
        human.field("entries", &verified.entries.to_string());
        if let Some(sequence) = verified.first_broken {
            human.field("first_broken", &sequence.to_string());
        }
    }
    ExitCode::from(exit_codes::SUCCESS)
}

/// `operation`'s result on an Attached engine in this process, or the exit
/// code of its failure, which it printed.
fn call(
    engine: &Engine,
    dependencies: Dependencies,
    operation: &'static Operation,
    json: bool,
    human: &Human,
) -> Result<Value, ExitCode> {
    let call = CommandCall {
        method: operation.name,
        params: json!({}),
        cancellable: operation.cancellable,
        follower: None,
        pager: None,
        stopper: None,
        input: None,
    };
    let product = engine.identity().product;
    match engine_choice::call_attached(engine, dependencies, product, call) {
        Answer::Result(value) => Ok(value),
        Answer::Error(error) => Err(fail(&error, operation, json, human)),
        Answer::Unanswered(problem) => {
            tracing::error!("{problem}");
            Err(ExitCode::from(exit_codes::unanswered()))
        }
        Answer::Interrupted(interrupt) => Err(ExitCode::from(interrupt.exit_code())),
    }
}

/// Prints `error`, `operation`'s failure, as every command does: as
/// [`CommandFailed`] on stdout under `--json`, else on stderr; answers its
/// exit code.
fn fail(error: &ErrorObject, operation: &Operation, json: bool, human: &Human) -> ExitCode {
    let group = goodfetch_cli::generated::group(error);
    if json {
        output::json(&CommandFailed {
            error: CommandError {
                code: error.data.code.clone(),
                group,
                retryable: error.data.retryable,
                message: error.message.clone(),
                operation: Some(operation.name.to_owned()),
                details: error.data.details.clone(),
            },
        });
    } else {
        human.failure(
            &error.message,
            &[
                ("code", error.data.code.clone()),
                ("retryable", error.data.retryable.to_string()),
            ],
        );
    }
    ExitCode::from(exit_codes::of_error(&error.data.code, group))
}
