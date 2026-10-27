//! `usage raise` (ADRs 0041 and 0069): raising a Limit is a person's
//! answer, so the CLI offers it as a prompt only, never as a command
//! generated from the catalog. It asks at the terminal and raises the
//! Limit through `usage.raise` only when the person answers yes; when stdin
//! is no terminal, as under a script, an agent or CI, it is refused with
//! `person_required` before any engine call. It takes no `--json`.

use std::process::ExitCode;

use goodfetch_cli::engine_choice::{self, Answer, CommandCall};
use goodfetch_cli::exit_codes;
use goodfetch_cli::output::{self, Human};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::connection::PERSON_REQUIRED;
use goodfetch_protocol::run::Period;
use goodfetch_protocol::usage::{USAGE_RAISE, UsageRaiseParams};
use serde_json::Value;

/// Asks the person at the terminal to raise the Limit `params` names, and
/// raises it on `engine` when they answer yes.
pub(super) fn raise(
    engine: &Engine,
    dependencies: Dependencies,
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
    let call = CommandCall {
        method: USAGE_RAISE.name,
        params: serde_json::to_value(params).expect("invariant: the params serialize"),
        cancellable: USAGE_RAISE.cancellable,
        follower: None,
        pager: None,
        input: None,
    };
    let product = engine.identity().product;
    match engine_choice::call_attached(engine, dependencies, product, call) {
        Answer::Result(Value::Object(fields)) => {
            for (name, value) in fields {
                human.field(&name, &text(&value));
            }
            ExitCode::from(exit_codes::SUCCESS)
        }
        Answer::Result(value) => {
            output::line(&text(&value));
            ExitCode::from(exit_codes::SUCCESS)
        }
        Answer::Error(error) => {
            let group = goodfetch_cli::generated::group(&error);
            human.failure(
                &error.message,
                &[
                    ("code", error.data.code.clone()),
                    ("retryable", error.data.retryable.to_string()),
                ],
            );
            ExitCode::from(exit_codes::of_error(&error.data.code, group))
        }
        Answer::Unanswered(problem) => {
            tracing::error!("{problem}");
            ExitCode::from(exit_codes::unanswered())
        }
        Answer::Interrupted(interrupt) => ExitCode::from(interrupt.exit_code()),
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

/// A value as human output shows it: text as it is, anything else as JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
    }
}
