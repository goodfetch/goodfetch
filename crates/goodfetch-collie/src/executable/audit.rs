//! `audit export` and `audit verify` (ADRs 0016 and 0067): the user's
//! Audit log leaves the machine as JSONL, one entry with its hash per line,
//! in sequence, which an operator's log stack keeps, and `audit verify`
//! checks either the log itself or such an export, `--file`, which it reads
//! and verifies in this process, so a copy kept anywhere verifies without
//! the log it came from. Each reads the log through `audit.export` and
//! `audit.verify` on the Daemon when one runs or `--daemon` is given, else
//! on an Attached engine in this process (ticket #1183); `audit verify`
//! prints what `audit.verify` answers, as JSON under `--json`, and exits
//! `22`, `audit_log_not_intact`, when it does not verify; a failure prints
//! as every command's does.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::engine_choice::{
    self, Answer, CommandCall, Daemons, EngineChoice, EngineFlags, Lifetime,
};
use super::exit_codes;
use super::generated::{Joined, Joining};
use super::output::{self, Human};
use clap::Subcommand;
use goodfetch_engine::audit::verify_exported;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::audit::{AUDIT_EXPORT, AUDIT_VERIFY, AuditVerified};
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::cli::AUDIT_LOG_NOT_INTACT;
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
        #[command(flatten)]
        engine: EngineFlags,
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
        #[command(flatten)]
        engine: EngineFlags,
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
        AuditCommand::Own(OwnAuditCommand::Export {
            json,
            engine: flags,
        }) => {
            let warn = |text: &str| human.warning(text);
            let reach = reach(daemons, flags, &warn);
            export(engine, dependencies, &reach, json, human)
        }
        AuditCommand::Own(OwnAuditCommand::Verify {
            file: None,
            json,
            engine: flags,
        }) => {
            let warn = |text: &str| human.warning(text);
            let reach = reach(daemons, flags, &warn);
            match call(engine, dependencies, &reach, &AUDIT_VERIFY, json, human) {
                Ok(verified) => print(&verified, json, human),
                Err(exit) => exit,
            }
        }
        AuditCommand::Own(OwnAuditCommand::Verify {
            file: Some(file),
            json,
            engine: _,
        }) => verify_file(&file, json, human),
    }
}

/// The engine a command's call runs on: as its `flags` choose, a command
/// whose value does not outlive it, so the Daemon when one runs or
/// `--daemon` is given, else an Attached engine ([`engine_choice`]).
fn reach<'a>(
    daemons: &'a dyn Daemons,
    flags: EngineFlags,
    warn: &'a dyn Fn(&str),
) -> EngineChoice<'a> {
    EngineChoice {
        choice: flags.choice(),
        lifetime: Lifetime::Passing,
        daemons,
        warn,
    }
}

/// Writes every entry `audit.export` answers on the engine `reach`
/// chooses, one JSON line each.
fn export(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    json: bool,
    human: &Human,
) -> ExitCode {
    let exported = match call(engine, dependencies, reach, &AUDIT_EXPORT, json, human) {
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
            output::fail(unread, Some(AUDIT_VERIFY.name), json, human)
        }
    }
}

/// Prints what verifying a log found, `verified`: as JSON under `--json`,
/// else each field on its line; exits `22`, `audit_log_not_intact`, for a
/// log or export that does not verify, as `sha256sum -c` exits non-zero on
/// a mismatch, so a gate that reads only the exit code catches it (ADR
/// 0069).
fn print(verified: &Value, json: bool, human: &Human) -> ExitCode {
    let read = serde_json::from_value::<AuditVerified>(verified.clone());
    if json {
        output::json(verified);
    } else if let Ok(verified) = &read {
        human.field("intact", &verified.intact.to_string());
        human.field("entries", &verified.entries.to_string());
        if let Some(sequence) = verified.first_broken {
            human.field("first_broken", &sequence.to_string());
        }
    }
    match read {
        Ok(verified) if verified.intact => ExitCode::from(exit_codes::SUCCESS),
        Ok(_) => ExitCode::from(AUDIT_LOG_NOT_INTACT.code),
        Err(problem) => output::fail(
            exit_codes::internal(format!(
                "The engine answered `{}` with no verification: {problem}.",
                AUDIT_VERIFY.name
            )),
            Some(AUDIT_VERIFY.name),
            json,
            human,
        ),
    }
}

/// `operation`'s result on the engine `reach` chooses, the Daemon or an
/// Attached engine in this process, `engine` on `dependencies`, or the exit
/// code of its failure, which it printed.
fn call(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
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
    match engine_choice::call(engine, dependencies, product, reach, call) {
        Answer::Result(value) => Ok(value),
        Answer::Error(error) => Err(output::fail(error, Some(operation.name), json, human)),
        Answer::Refused(error) => Err(output::fail_exiting(
            error,
            Some(operation.name),
            exit_codes::DAEMON_REFUSED,
            json,
            human,
        )),
        Answer::Unanswered(error) => Err(output::fail(error, None, json, human)),
        Answer::Interrupted(interrupt) => Err(ExitCode::from(interrupt.exit_code())),
    }
}
