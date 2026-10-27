//! `certs accept` and `certs forget` (ADRs 0011, 0042 and 0069): accepting
//! a pinned host's changed key and removing a pin are a person's answers,
//! so the CLI offers them as prompts only, never as commands generated from
//! the catalog. Each reads the caller's pin through `certs.list`, shows it
//! at the terminal, and calls `certs.accept` or `certs.forget` only when
//! the person answers yes. Under `--json`, or when stdin is no terminal, as
//! under a script, an agent or CI, it is refused with `person_required`
//! before any engine call.

use std::process::ExitCode;

use clap::Subcommand;
use goodfetch_cli::engine_choice::{self, Answer, CommandCall, Daemons};
use goodfetch_cli::exit_codes;
use goodfetch_cli::generated::{Joined, Joining};
use goodfetch_cli::output::{self, Human};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::{ErrorGroup, Operation};
use goodfetch_protocol::cli::{CommandError, CommandFailed};
use goodfetch_protocol::connection::PERSON_REQUIRED;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::tls::{
    CERTS_ACCEPT, CERTS_FORGET, CERTS_LIST, CertificatePin, CertsAcceptParams, CertsForgetParams,
    CertsListed,
};
use serde_json::{Map, Value, json};

// The `certs` group's commands: the catalog's `certs` commands, listed
// first, then its own. No doc comment: clap would make it the group's help.
#[derive(Subcommand)]
pub(super) enum CertsCommand {
    #[command(flatten)]
    Generated(Joining<OwnCertsCommand>),
    #[command(flatten)]
    Own(OwnCertsCommand),
}

// The `certs` group's hand-written commands; no doc comment either.
#[derive(Subcommand)]
pub(super) enum OwnCertsCommand {
    /// Pins the changed key that stopped a host in place of its pin: a
    /// person's answer, asked at the terminal, and refused with
    /// `person_required` under `--json` or when stdin is no terminal.
    Accept {
        /// The pinned host.
        host: String,
        /// Refused: a person answers at a terminal, never under `--json`.
        #[arg(long)]
        json: bool,
    },
    /// Removes a host's pin, so its next connection under trust on first
    /// use pins again: a person's answer, asked at the terminal, and
    /// refused with `person_required` under `--json` or when stdin is no
    /// terminal.
    Forget {
        /// The pinned host.
        host: String,
        /// Refused: a person answers at a terminal, never under `--json`.
        #[arg(long)]
        json: bool,
    },
}

impl Joined for OwnCertsCommand {
    const NAMESPACE: &'static str = "certs";
}

/// Runs the `certs` command `command`.
pub(super) fn run(
    command: CertsCommand,
    engine: &Engine,
    dependencies: Dependencies,
    daemons: &dyn Daemons,
    human: &Human,
) -> ExitCode {
    match command {
        CertsCommand::Generated(joining) => {
            joining.call().run(engine, dependencies, daemons, human)
        }
        CertsCommand::Own(OwnCertsCommand::Accept { host, json }) => {
            accept(engine, dependencies, &host, json, human)
        }
        CertsCommand::Own(OwnCertsCommand::Forget { host, json }) => {
            forget(engine, dependencies, &host, json, human)
        }
    }
}

/// Asks the person at the terminal to pin the changed key that stopped
/// `host` in place of its pin, and pins it on `engine` when they answer
/// yes.
fn accept(
    engine: &Engine,
    mut dependencies: Dependencies,
    host: &str,
    json: bool,
    human: &Human,
) -> ExitCode {
    if json || !human.can_ask() {
        return person_required(&CERTS_ACCEPT, json, human);
    }
    let pin = match pin_of(engine, forked(&mut dependencies), host, human) {
        Ok(pin) => pin,
        Err(exit) => return exit,
    };
    let Some((pin, changed)) = pin.and_then(|pin| {
        let changed = pin.changed_spki_sha256.clone()?;
        Some((pin, changed))
    }) else {
        human.failure(
            &format!(
                "No changed key of {host} waits to be accepted: `certs list` shows the caller's pins"
            ),
            &[("code", ProtocolFailure::InvalidParams.name().to_owned())],
        );
        return ExitCode::from(exit_codes::of_error(
            ProtocolFailure::InvalidParams.name(),
            ErrorGroup::Program,
        ));
    };
    let pinned = pin
        .spki_sha256
        .unwrap_or_else(|| "an unreadable pin".to_owned());
    let question = format!(
        "The key of {} changed from {pinned} to {changed}. Pin {changed} in its place? [y/N]",
        pin.host
    );
    let Some(yes) = asked(&question, human) else {
        return person_required(&CERTS_ACCEPT, false, human);
    };
    if !yes {
        output::line("The pin was not changed.");
        return ExitCode::from(exit_codes::SUCCESS);
    }
    let params = CertsAcceptParams {
        host: pin.host,
        spki_sha256: changed,
    };
    answered(engine, dependencies, &CERTS_ACCEPT, json!(params), human)
}

/// Asks the person at the terminal to remove `host`'s pin, and removes it
/// on `engine` when they answer yes.
fn forget(
    engine: &Engine,
    mut dependencies: Dependencies,
    host: &str,
    json: bool,
    human: &Human,
) -> ExitCode {
    if json || !human.can_ask() {
        return person_required(&CERTS_FORGET, json, human);
    }
    let pin = match pin_of(engine, forked(&mut dependencies), host, human) {
        Ok(pin) => pin,
        Err(exit) => return exit,
    };
    let Some(pin) = pin else {
        output::line(&format!("{host} has no pin."));
        return ExitCode::from(exit_codes::SUCCESS);
    };
    let question = match &pin.spki_sha256 {
        Some(pinned) => format!(
            "Remove the pin {pinned} of {}? Its next connection under trust on first use pins again. [y/N]",
            pin.host
        ),
        None => format!(
            "Remove the unreadable pin of {}? Its next connection under trust on first use pins again. [y/N]",
            pin.host
        ),
    };
    let Some(yes) = asked(&question, human) else {
        return person_required(&CERTS_FORGET, false, human);
    };
    if !yes {
        output::line("The pin was kept.");
        return ExitCode::from(exit_codes::SUCCESS);
    }
    let params = CertsForgetParams { host: pin.host };
    answered(engine, dependencies, &CERTS_FORGET, json!(params), human)
}

/// The person's answer to `question`, yes or not; none when stdin is no
/// terminal, where no person answers.
fn asked(question: &str, human: &Human) -> Option<bool> {
    let answer = human.ask(question)?;
    Some(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
}

/// What a second call on an Attached engine runs on: `dependencies`' own,
/// with entropy forked from theirs, since each call starts its engine.
fn forked(dependencies: &mut Dependencies) -> Dependencies {
    Dependencies {
        home: dependencies.home.clone(),
        clock: dependencies.clock,
        entropy: dependencies.entropy.fork(),
        blocking: dependencies.blocking,
        run_state_location: dependencies.run_state_location.clone(),
        run_state: dependencies.run_state.clone(),
        politeness: dependencies.politeness.clone(),
        environment: dependencies.environment.clone(),
        keychain: dependencies.keychain.clone(),
    }
}

/// The caller's pin of `host`, as `certs.list` answers it; none when it has
/// none. A host is matched as the store keys it: lower case, without a
/// trailing dot.
fn pin_of(
    engine: &Engine,
    dependencies: Dependencies,
    host: &str,
    human: &Human,
) -> Result<Option<CertificatePin>, ExitCode> {
    let listed = call(engine, dependencies, &CERTS_LIST, json!({}), human)?;
    let listed: CertsListed =
        serde_json::from_value(listed).map_err(|_| ExitCode::from(exit_codes::unanswered()))?;
    let wanted = host.to_ascii_lowercase();
    let wanted = wanted.strip_suffix('.').unwrap_or(&wanted);
    Ok(listed.pins.into_iter().find(|pin| pin.host == wanted))
}

/// Calls `operation` with `params` on `engine` and prints its result's
/// fields.
fn answered(
    engine: &Engine,
    dependencies: Dependencies,
    operation: &'static Operation,
    params: Value,
    human: &Human,
) -> ExitCode {
    match call(engine, dependencies, operation, params, human) {
        Ok(Value::Object(fields)) => {
            for (name, value) in fields {
                human.field(&name, &text(&value));
            }
            ExitCode::from(exit_codes::SUCCESS)
        }
        Ok(value) => {
            output::line(&text(&value));
            ExitCode::from(exit_codes::SUCCESS)
        }
        Err(exit) => exit,
    }
}

/// `operation`'s result on `engine`, or the exit code of its failure, which
/// it printed.
fn call(
    engine: &Engine,
    dependencies: Dependencies,
    operation: &'static Operation,
    params: Value,
    human: &Human,
) -> Result<Value, ExitCode> {
    let call = CommandCall {
        method: operation.name,
        params,
        cancellable: operation.cancellable,
        follower: None,
        pager: None,
        stopper: None,
        input: None,
    };
    let product = engine.identity().product;
    match engine_choice::call_attached(engine, dependencies, product, call) {
        Answer::Result(value) => Ok(value),
        Answer::Error(error) => {
            let group = goodfetch_cli::generated::group(&error);
            human.failure(
                &error.message,
                &[
                    ("code", error.data.code.clone()),
                    ("retryable", error.data.retryable.to_string()),
                ],
            );
            Err(ExitCode::from(exit_codes::of_error(
                &error.data.code,
                group,
            )))
        }
        Answer::Unanswered(problem) => {
            tracing::error!("{problem}");
            Err(ExitCode::from(exit_codes::unanswered()))
        }
        Answer::Interrupted(interrupt) => Err(ExitCode::from(interrupt.exit_code())),
    }
}

/// Refuses `operation`, a person's answer, with `person_required`: as JSON
/// under `--json`, else on stderr.
fn person_required(operation: &Operation, json: bool, human: &Human) -> ExitCode {
    let command = operation.name.replace('.', " ");
    let message = format!(
        "`{command}` is a person's answer, asked at a terminal: run it in one without `--json`, or answer it in the Console"
    );
    if json {
        output::json(&CommandFailed {
            error: CommandError {
                code: PERSON_REQUIRED.name.to_owned(),
                group: PERSON_REQUIRED.group,
                retryable: PERSON_REQUIRED.retryable,
                message,
                operation: Some(operation.name.to_owned()),
                details: Map::new(),
            },
        });
    } else {
        human.failure(
            &message,
            &[
                ("code", PERSON_REQUIRED.name.to_owned()),
                ("retryable", PERSON_REQUIRED.retryable.to_string()),
            ],
        );
    }
    ExitCode::from(exit_codes::of_error(
        PERSON_REQUIRED.name,
        PERSON_REQUIRED.group,
    ))
}

/// A value as human output shows it: text as it is, anything else as JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
    }
}
