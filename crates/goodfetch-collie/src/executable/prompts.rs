//! The one place a person-only prompt asks (ADRs 0011, 0031, 0041, 0052
//! and 0069). A prompt is offered at a terminal only, and its answerer is
//! the OS user: under `--json`, or when stdin is no terminal, as under a
//! pipe, a script, an agent or CI, it is refused with `person_required`, a
//! refusal that is not retryable and exits `4`, before any engine call,
//! its `fix` naming the interactive command and wherever else a person
//! gives the answer. A prompt asks its question once: a prompt that widens
//! what the person takes on prints its warning once before it, and no
//! confirmation loop follows (ADR 0052). Each person-only command adds a
//! call to [`Prompt::ask`], never a loop of its own.

use std::process::ExitCode;

use super::catalog_commands::command_words;
use super::exit_codes;
use super::output::{self, Human};
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::cli::{CommandError, CommandFailed};
use goodfetch_protocol::connection::PERSON_REQUIRED;
use serde_json::{Map, Value};

/// A person-only prompt: the operation its answer calls, and where else a
/// person gives that answer.
pub(super) struct Prompt {
    /// The operation the person's yes calls.
    pub(super) operation: &'static Operation,
    /// Where else a person gives the answer, as a `fix` adds it after the
    /// command, such as "raise the Limit in the Kennel"; none where only
    /// the command asks it.
    pub(super) elsewhere: Option<&'static str>,
}

impl Prompt {
    /// Refuses the prompt with `person_required` when no person answers:
    /// under `--json`, or when stdin is no terminal. A command calls it
    /// before any engine call.
    pub(super) fn person_present(&self, json: bool, human: &Human) -> Result<(), ExitCode> {
        if json || !human.can_ask() {
            return Err(self.refused(json, human));
        }
        Ok(())
    }

    /// Asks the person `question` once, after `warning` when the answer
    /// widens what they take on; whether they answered yes. Refused as
    /// [`Self::person_present`] refuses it when no person answers.
    pub(super) fn ask(
        &self,
        question: &str,
        warning: Option<&str>,
        human: &Human,
    ) -> Result<bool, ExitCode> {
        self.person_present(false, human)?;
        if let Some(warning) = warning {
            human.warning(warning);
        }
        let answer = human
            .ask(&format!("{question} [y/N]"))
            .ok_or_else(|| self.refused(false, human))?;
        Ok(yes(&answer))
    }

    /// The `person_required` refusal: one `CommandFailed` on stdout under
    /// `--json`, else on stderr.
    fn refused(&self, json: bool, human: &Human) -> ExitCode {
        let command = command_words(self.operation.name);
        let elsewhere = self
            .elsewhere
            .map(|elsewhere| format!(", or {elsewhere}"))
            .unwrap_or_default();
        let mut details = Map::new();
        details.insert(
            "fix".to_owned(),
            Value::from(format!(
                "Run `{command}` in a terminal, without `--json`{elsewhere}."
            )),
        );
        let failed = CommandFailed {
            error: CommandError {
                code: PERSON_REQUIRED.name.to_owned(),
                group: PERSON_REQUIRED.group,
                retryable: PERSON_REQUIRED.retryable,
                message: format!("`{command}` is a person's answer, asked at a terminal"),
                operation: Some(self.operation.name.to_owned()),
                details,
            },
        };
        output::failed(&failed, json, human);
        ExitCode::from(exit_codes::of_error(
            PERSON_REQUIRED.name,
            PERSON_REQUIRED.group,
        ))
    }
}

/// Whether `answer` is a yes: `y` or `yes`, in any case. Anything else,
/// an empty line too, is the prompt's default, no.
fn yes(answer: &str) -> bool {
    matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes")
}
