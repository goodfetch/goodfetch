//! `usage raise` (ADRs 0041 and 0069): raising a Limit is a person's
//! answer, so the CLI offers it as a prompt only, never as a command
//! generated from the catalog. It asks at the terminal and raises the
//! Limit through `usage.raise` only when the person answers yes; under
//! `--json`, or when stdin is no terminal, as under a script, an agent or
//! CI, it is refused with `person_required` before any engine call, its
//! `fix` naming the command and the Kennel ([`prompts`]). It raises the
//! Limit on the Daemon when one runs or `--daemon` is given, else on an
//! Attached engine, as any command whose value does not outlive it (ticket
//! #1183).
//!
//! [`prompts`]: super::prompts

use std::process::ExitCode;

use super::daemon::Machine;
use super::engine_choice::{Choice, EngineChoice};
use super::exit_codes;
use super::operation::run_operation;
use super::output::{self, Human};
use super::prompts::Prompt;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::run::Period;
use goodfetch_protocol::usage::{USAGE_RAISE, UsageRaiseParams};

/// The Limit raise's prompt: a person also raises a Limit in the Kennel.
const RAISE: Prompt = Prompt {
    operation: &USAGE_RAISE,
    elsewhere: Some("raise the Limit in the Kennel"),
};

/// Asks the person at the terminal to raise the Limit `params` names, and
/// raises it when they answer yes: on the engine that `choice` and the
/// Daemon `machine` reaches choose for a command whose value does not
/// outlive it ([`engine_choice`]), which is the Daemon or an Attached
/// engine, `engine` on `dependencies`. Under `--json`, `json`, it is
/// refused.
pub(super) fn raise(
    engine: &Engine,
    dependencies: Dependencies,
    (machine, choice): (&Machine<'_>, Choice),
    (params, json): (&UsageRaiseParams, bool),
    human: &Human,
) -> ExitCode {
    let asked = RAISE
        .person_present(json, human)
        .and_then(|()| RAISE.ask(&question(params), None, human));
    match asked {
        Ok(true) => {}
        Ok(false) => {
            output::line("The Limit was not raised.");
            return ExitCode::from(exit_codes::SUCCESS);
        }
        Err(exit) => return exit,
    }
    let warn = |text: &str| human.warning(text);
    let reach = EngineChoice::passing(choice, machine, &warn);
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
pub(super) fn question(params: &UsageRaiseParams) -> String {
    format!(
        "Raise the {} to {}?",
        limit(params),
        amount(params.amount, params.currency.as_deref())
    )
}

/// The Limit `params` names.
pub(super) fn limit(params: &UsageRaiseParams) -> String {
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
    format!(
        "{}{scope_id} Limit on {}{currency}{period}",
        params.scope, params.usage_kind
    )
}

/// How much a Limit allows: `amount` of its Usage kind, or millionths of
/// its `currency`.
pub(super) fn amount(amount: u64, currency: Option<&str>) -> String {
    match currency {
        Some(currency) => format!("{amount} millionths of {currency}"),
        None => amount.to_string(),
    }
}

/// A period as `--period` takes it: JSON, such as
/// `{"every":"month","time_zone":"UTC"}`.
pub(super) fn period(text: &str) -> Result<Period, String> {
    serde_json::from_str(text).map_err(|error| format!("no period: {error}"))
}
