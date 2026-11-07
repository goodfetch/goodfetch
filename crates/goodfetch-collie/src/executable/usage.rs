//! `usage raise` (ADRs 0041 and 0069): raising a Limit is a person's
//! answer, so the CLI offers it as a prompt only, never as a command
//! generated from the catalog. It asks at the terminal and raises the
//! Limit through `usage.raise` only when the person answers yes; when stdin
//! is no terminal, as under a script, an agent or CI, it is refused with
//! `person_required` before any engine call. It takes no `--json`. It
//! raises the Limit on the Daemon when one runs or `--daemon` is given,
//! else on an Attached engine, as any command whose value does not outlive
//! it (ticket #1183).

use std::process::ExitCode;

use super::engine_choice::{Choice, Daemons, EngineChoice};
use super::exit_codes;
use super::operation::run_operation;
use super::output::{self, Human};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::connection::PERSON_REQUIRED;
use goodfetch_protocol::run::Period;
use goodfetch_protocol::usage::{USAGE_RAISE, UsageRaiseParams};

/// Asks the person at the terminal to raise the Limit `params` names, and
/// raises it when they answer yes: on the engine `choice` and the
/// executable's Daemon, `daemons`, choose, a command whose value does not
/// outlive it ([`engine_choice`]), or else an Attached engine, `engine` on
/// `dependencies`.
pub(super) fn raise(
    engine: &Engine,
    dependencies: Dependencies,
    (daemons, choice): (&dyn Daemons, Choice),
    params: &UsageRaiseParams,
    human: &Human,
) -> ExitCode {
    let Some(answer) = human.ask(&format!("{}? [y/N]", question(params))) else {
        human.failure(
            "`usage raise` is a person's answer, asked at a terminal: run it in one, or raise the Limit in the Console",
            &[
                ("code", PERSON_REQUIRED.name.to_owned()),
                ("retryable", PERSON_REQUIRED.retryable.to_string()),
            ],
        );
        return ExitCode::from(exit_codes::of_error(
            PERSON_REQUIRED.name,
            PERSON_REQUIRED.group,
        ));
    };
    if !matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
        output::line("The Limit was not raised.");
        return ExitCode::from(exit_codes::SUCCESS);
    }
    let warn = |text: &str| human.warning(text);
    let reach = EngineChoice::passing(choice, daemons, &warn);
    let params = serde_json::to_value(params).expect("invariant: the params serialize");
    // A person-only command takes no `--json`: its failure prints on
    // stderr, as every command's does.
    match run_operation(
        engine,
        dependencies,
        &reach,
        &USAGE_RAISE,
        params,
        false,
        human,
    ) {
        Ok(result) => {
            output::fields(&result, human);
            ExitCode::from(exit_codes::SUCCESS)
        }
        Err(exit) => exit,
    }
}

/// The question the prompt asks: which Limit, raised to what.
fn question(params: &UsageRaiseParams) -> String {
    let scope_id = params
        .scope_id
        .as_ref()
        .map(|id| format!(" `{id}`"))
        .unwrap_or_default();
    let currency = params
        .currency
        .as_ref()
        .map(|currency| format!(" in {currency}"))
        .unwrap_or_default();
    let period = params
        .period
        .as_ref()
        .map(|period| format!(" per {} in {}", period.every.as_str(), period.time_zone))
        .unwrap_or_default();
    let amount = match &params.currency {
        Some(currency) => format!("{} millionths of {currency}", params.amount),
        None => params.amount.to_string(),
    };
    format!(
        "Raise the {}{scope_id} Limit on {}{currency}{period} to {amount}",
        params.scope, params.usage_kind
    )
}

/// A period as `--period` takes it: JSON, such as
/// `{"every":"month","time_zone":"UTC"}`.
pub(super) fn period(text: &str) -> Result<Period, String> {
    serde_json::from_str(text).map_err(|error| format!("no period: {error}"))
}
