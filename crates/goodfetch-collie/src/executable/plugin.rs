//! `plugin add` (ADRs 0033 and 0099): installing a Plugin and pinning it in
//! the Plugin lockfile is a person's answer, so the CLI offers it as a
//! prompt only, never as a command generated from the catalog. It asks at
//! the terminal and calls `plugin.add` only when the person answers yes,
//! with the project configuration found from the working folder upward,
//! beside which the lockfile goes. Under `--json`, or when stdin is no
//! terminal, as under a script, an agent or CI, it is refused with
//! `person_required` before any engine call.

use std::path::Path;
use std::process::ExitCode;

use clap::Subcommand;
use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{SettingsError, find_project, project_settings};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::plugin::add::{PLUGIN_ADD, PluginAddParams};
use goodfetch_protocol::settings::ProjectSettings;

use super::daemon::Machine;
use super::engine_choice::{EngineChoice, EngineFlags};
use super::exit_codes;
use super::operation::run_operation;
use super::output::{self, Human};
use super::prompts::Prompt;

/// The prompt that installs and pins a Plugin.
const ADD: Prompt = Prompt {
    operation: &PLUGIN_ADD,
    elsewhere: None,
};

// The `plugin` group's commands. No doc comment: clap would make it the
// group's help.
#[derive(Subcommand)]
pub(super) enum PluginCommand {
    /// Installs a Plugin from its OCI reference,
    /// `<registry>/<repository>@sha256:<hex>`, and pins its digest in the
    /// Plugin lockfile beside the project's configuration: a person's
    /// answer, asked at the terminal, and refused with `person_required`
    /// under `--json` or when stdin is no terminal.
    Add {
        /// The Plugin's OCI reference, pinned by its image index's digest.
        reference: String,
        /// Refused: a person answers at a terminal, never under `--json`.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineFlags,
    },
}

/// Runs the `plugin` command `command`.
pub(super) fn run(
    command: PluginCommand,
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
    human: &Human,
) -> ExitCode {
    match command {
        PluginCommand::Add {
            reference,
            json,
            engine: flags,
        } => {
            let warn = |text: &str| human.warning(text);
            let reach = EngineChoice::passing(flags.choice(), machine, &warn);
            add(engine, dependencies, &reach, reference, json, human)
        }
    }
}

/// Asks the person at the terminal to install the Plugin `reference` names
/// and pin it, and calls `plugin.add` on `engine` when they answer yes.
fn add(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    reference: String,
    json: bool,
    human: &Human,
) -> ExitCode {
    if let Err(exit) = ADD.person_present(json, human) {
        return exit;
    }
    let Ok(folder) = std::env::current_dir() else {
        tracing::error!("the working folder cannot be read");
        return ExitCode::FAILURE;
    };
    let project = match project(&folder) {
        Ok(project) => project,
        Err(error) => {
            return output::fail(
                local_settings_error(&error, engine.identity()).to_error_object(),
                None,
                false,
                human,
            );
        }
    };
    let yes = match ADD.ask(
        &question(&reference, project.as_ref()),
        Some("A Plugin runs as its own process when a Run uses it."),
        human,
    ) {
        Ok(yes) => yes,
        Err(exit) => return exit,
    };
    if !yes {
        output::line("Nothing was installed.");
        return ExitCode::from(exit_codes::SUCCESS);
    }
    let params = serde_json::to_value(PluginAddParams { reference, project })
        .expect("invariant: the params serialize");
    // A person-only command takes no `--json`: its failure prints on
    // stderr, as every command's does.
    match run_operation(
        engine,
        dependencies,
        reach,
        &PLUGIN_ADD,
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

/// The project configuration found from `folder` upward, none outside a
/// project.
fn project(folder: &Path) -> Result<Option<ProjectSettings>, SettingsError> {
    let Some(path) = find_project(folder) else {
        return Ok(None);
    };
    Ok(project_settings(&path)?.map(|settings| ProjectSettings {
        settings,
        path: path.display().to_string(),
    }))
}

/// The question that installs the Plugin `reference` names and pins it in
/// the lockfile beside `project`, or in the Goodfetch home's when none.
fn question(reference: &str, project: Option<&ProjectSettings>) -> String {
    let beside = project.map_or_else(
        || "the Goodfetch home's configuration".to_owned(),
        |project| project.path.clone(),
    );
    format!(
        "Install the Plugin {reference} and pin its digest in the Plugin lockfile beside {beside}?"
    )
}
