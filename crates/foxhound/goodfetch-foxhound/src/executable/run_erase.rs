//! `run erase <run> [--value <v>] [--key <k>]` (ADR 0016; spec #565,
//! story 4-120): erasing personal data across a Run's stored Records, Baseline
//! rows, Checkpoint fields and Response cache entries is a person's answer, so
//! the CLI offers it as an interactive prompt only, written by hand since the
//! catalog keeps `run.erase` off every generated surface. Under `--json`, or
//! when stdin is no terminal, as under a pipe, a script, an agent or CI, it is
//! refused with `person_required` before any engine call, and exits `4`.

use std::process::ExitCode;

use clap::{ArgMatches, Command};
use goodfetch_collie::executable::Machine;
use goodfetch_collie::executable::catalog_commands::{Call, OwnCall, OwnCommand};
use goodfetch_collie::executable::engine_choice::{self, CommandCall};
use goodfetch_collie::executable::exit_codes;
use goodfetch_collie::executable::output::{self, Human};
use goodfetch_collie::executable::prompts::Prompt;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::foxhound::erase::RUN_ERASE;
use serde_json::{Map, Value};

/// `run erase`, the only command of `run.erase`.
pub(super) const COMMAND: OwnCommand = OwnCommand {
    operation: RUN_ERASE.name,
    replaces: &[],
    about: Some(
        "Erases matching Records, Baseline rows, Checkpoint fields and Response cache entries of a run no engine runs, once you answer yes",
    ),
    args,
    read: read_flags,
};

/// The erase's prompt: only this command asks it.
const ERASE: Prompt = Prompt {
    operation: &RUN_ERASE,
    elsewhere: Some("erase it in the Kennel"),
};

/// `command` as the catalog's flags make it: the command adds none.
fn args(command: Command, _operation: &Operation) -> Command {
    command
}

/// The erase's call: the catalog's flags have read every param.
fn read_flags(_matches: &ArgMatches, _params: &mut Map<String, Value>) -> Box<dyn OwnCall> {
    Box::new(Erase)
}

/// `run erase`'s call, its params all read.
struct Erase;

impl OwnCall for Erase {
    fn run(
        self: Box<Self>,
        call: &Call,
        engine: &Engine,
        dependencies: Dependencies,
        machine: &Machine<'_>,
        human: &Human,
    ) -> ExitCode {
        let asked = ERASE
            .person_present(call.json(), human)
            .and_then(|()| ERASE.ask(&question(call.params()), Some(WARNING), human));
        match asked {
            Ok(true) => {}
            Ok(false) => {
                output::line("Nothing was erased from the run.");
                return ExitCode::from(exit_codes::SUCCESS);
            }
            Err(exit) => return exit,
        }
        let command_call = CommandCall {
            method: call.operation().name,
            params: Value::Object(call.params().clone()),
            cancellable: call.operation().cancellable,
            follower: None,
            pager: None,
            input: None,
            stopper: None,
        };
        let warn = |text: &str| human.warning(text);
        let reach = call.reach(machine, &warn);
        let answer =
            engine_choice::call(engine, dependencies, call.product(), &reach, command_call);
        match answer.into_result() {
            Ok(result) => {
                call.print(&result, human);
                ExitCode::from(exit_codes::SUCCESS)
            }
            Err(failure) => call.failed(failure, human),
        }
    }
}

/// What the person is warned of before the question.
const WARNING: &str = "Erasing deletes every matching Record, Baseline row, Checkpoint field, kept Export and Response cache entry of the run, and cannot be undone.";

/// The question the prompt asks: which run.
fn question(params: &Map<String, Value>) -> String {
    let run_id = params
        .get("run_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("Erase matching data from the run `{run_id}`?")
}
