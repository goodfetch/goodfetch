//! A command's call and how its answer ends the command (ADRs 0024 and
//! 0069): [`run_operation`] calls one operation on the engine the command
//! line chooses, as every hand-written command that calls the catalog
//! does, and answers its result; [`failed`] prints a call's failure as
//! every command's failure prints and answers its exit code: the engine's
//! error by its Error code's row, naming the operation; a Daemon's refusal
//! of `hello` by Product or protocol version `11`
//! ([`exit_codes::DAEMON_REFUSED`]), naming it; a call that was never made,
//! naming none; and an interrupt by its signal's row.

use std::process::ExitCode;

use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::Operation;
use serde_json::Value;

use super::engine_choice::{self, CommandCall, EngineChoice, Failed};
use super::exit_codes;
use super::output::{self, Human};

/// Calls `operation` with `params` on the engine `reach` chooses, the
/// Daemon or an Attached engine in this process, `engine` on
/// `dependencies`, and answers its result, or the exit code of its failure,
/// which it printed as [`failed`] does, as JSON under `json`.
pub fn run_operation(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    operation: &'static Operation,
    params: Value,
    json: bool,
    human: &Human,
) -> Result<Value, ExitCode> {
    let call = CommandCall::of(operation, params);
    let product = engine.identity().product;
    engine_choice::call(engine, dependencies, product, reach, call)
        .into_result()
        .map_err(|failure| failed(failure, operation.name, json, human))
}

/// Prints `failure`, the failure of a command's call of `operation`, as
/// every command's failure prints, as JSON under `json`, and answers its
/// exit code.
pub fn failed(failure: Failed, operation: &str, json: bool, human: &Human) -> ExitCode {
    match failure {
        Failed::Error(error) => output::fail(error, Some(operation), json, human),
        Failed::Refused(error) => output::fail_exiting(
            error,
            Some(operation),
            exit_codes::DAEMON_REFUSED,
            json,
            human,
        ),
        Failed::Unanswered(error) => output::fail(error, None, json, human),
        Failed::Interrupted(interrupt) => ExitCode::from(interrupt.exit_code()),
    }
}
