//! `dataset erase --run-id <run> --dataset <dataset>` (ADR 0031; spec
//! #1937, story 214): erasing a Dataset is a person's answer, so the CLI
//! offers it as a prompt only, written by hand since the catalog keeps
//! `dataset.erase` off every generated surface. Its flags are those the
//! catalog gives its params, as for the `dataset` group's other commands.
//! It asks once at the terminal, after a warning that the erase cannot be
//! undone, and calls `dataset.erase` only when the person answers yes;
//! under `--json`, or when stdin is no terminal, as under a pipe, a script,
//! an agent or CI, it is refused with `person_required` before any engine
//! call, and exits `4`. It erases on the engine its command line chooses,
//! which refuses a run it runs.

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
use goodfetch_protocol::foxhound::erase::DATASET_ERASE;
use serde_json::{Map, Value};

/// `dataset erase`, the only command of `dataset.erase`.
pub(super) const COMMAND: OwnCommand = OwnCommand {
    operation: DATASET_ERASE.name,
    replaces: &[],
    about: Some("Erases every Record of a Dataset of a run no engine runs, once you answer yes"),
    args,
    read: read_flags,
};

/// The erase's prompt: only this command asks it.
const ERASE: Prompt = Prompt {
    operation: &DATASET_ERASE,
    elsewhere: None,
};

/// `command` as the catalog's flags make it: the command adds none.
fn args(command: Command, _operation: &Operation) -> Command {
    command
}

/// The erase's call: the catalog's flags have read every param.
fn read_flags(_matches: &ArgMatches, _params: &mut Map<String, Value>) -> Box<dyn OwnCall> {
    Box::new(Erase)
}

/// `dataset erase`'s call, its params all read.
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
                output::line("The Dataset was not erased.");
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
const WARNING: &str = "Erasing deletes every committed and rejected Record of the Dataset from the run, and every Export kept of the run, and cannot be undone.";

/// The question the prompt asks: which Dataset of which run.
fn question(params: &Map<String, Value>) -> String {
    let named = |name: &str| params.get(name).and_then(Value::as_str).unwrap_or_default();
    format!(
        "Erase the Dataset `{}` of the run `{}`?",
        named("dataset"),
        named("run_id")
    )
}
