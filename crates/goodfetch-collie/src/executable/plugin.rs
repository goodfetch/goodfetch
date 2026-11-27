//! `plugin add` (ADRs 0033 and 0099): installing a Plugin and pinning it in
//! the Plugin lockfile is a person's answer, so the CLI offers it as a
//! prompt only, never as a command generated from the catalog. It asks at
//! the terminal and calls `plugin.add` only when the person answers yes,
//! with the project configuration found from the working folder upward,
//! beside which the lockfile goes. It then prints, once, each permission
//! the Plugin's manifest declares with its limits, and asks the person to
//! approve the Plugin and those permissions; a yes calls `plugin.add` again
//! with `approve`, which writes them in the Operator policy, or prints the
//! entry to add to a policy managed elsewhere (issue 900). Under `--json`, or when stdin is no
//! terminal, as under a script, an agent or CI, it is refused with
//! `person_required` before any engine call.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use clap::Subcommand;
use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{SettingsError, find_project, project_settings};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::plugin::add::{PLUGIN_ADD, PluginAddParams, PluginAdded, PluginApproval};
use goodfetch_protocol::settings::ProjectSettings;

use super::certs::forked;
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
/// and pin it, and calls `plugin.add` on `engine` when they answer yes;
/// then states each permission the Plugin's manifest declares, with its
/// limits, once, and asks the person to approve the Plugin and those
/// permissions, which a second `plugin.add` writes in the Operator policy.
fn add(
    engine: &Engine,
    mut dependencies: Dependencies,
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
    let mut params = PluginAddParams {
        reference,
        project,
        approve: false,
    };
    let added = match plugin_add(engine, forked(&mut dependencies), reach, &params, human) {
        Ok(added) => added,
        Err(exit) => return exit,
    };
    output::line(&format!(
        "Installed the Plugin {} {}, pinned in {}.",
        added.name, added.version, added.lockfile
    ));
    let yes = match ADD.ask(&approval_question(&added), Some(&stated(&added)), human) {
        Ok(yes) => yes,
        Err(exit) => return exit,
    };
    if !yes {
        output::line(&format!(
            "The Plugin {} was not approved: a Run refuses it with `plugin_not_allowed` until it is.",
            added.name
        ));
        return ExitCode::from(exit_codes::SUCCESS);
    }
    params.approve = true;
    let approved = match plugin_add(engine, dependencies, reach, &params, human) {
        Ok(approved) => approved,
        Err(exit) => return exit,
    };
    match approved.approval {
        Some(PluginApproval::Written { policy }) => output::line(&format!(
            "Approved the Plugin {} in the Operator policy {policy}.",
            approved.name
        )),
        Some(PluginApproval::Managed { entry }) => {
            human.warning(
                "The Operator policy is managed elsewhere, so nothing was written: add this entry there.",
            );
            output::line(&entry);
        }
        None => {}
    }
    ExitCode::from(exit_codes::SUCCESS)
}

/// Calls `plugin.add` with `params` and reads its answer. A person-only
/// command takes no `--json`: its failure prints on stderr, as every
/// command's does.
fn plugin_add(
    engine: &Engine,
    dependencies: Dependencies,
    reach: &EngineChoice<'_>,
    params: &PluginAddParams,
    human: &Human,
) -> Result<PluginAdded, ExitCode> {
    let params = serde_json::to_value(params).expect("invariant: the params serialize");
    let result = run_operation(
        engine,
        dependencies,
        reach,
        &PLUGIN_ADD,
        params,
        false,
        human,
    )?;
    serde_json::from_value(result).map_err(|error| {
        tracing::error!(%error, "the engine's `plugin.add` answer does not read");
        ExitCode::FAILURE
    })
}

/// The question that approves the Plugin `added` in the Operator policy.
fn approval_question(added: &PluginAdded) -> String {
    let permissions = if added.permissions.is_empty() {
        String::new()
    } else {
        " and each permission above".to_owned()
    };
    format!(
        "Approve the Plugin {} at {}{permissions} in the Operator policy?",
        added.name, added.digest
    )
}

/// Everything the Plugin `added` grants — its kinds, network level,
/// environment variables, files, processes, browsers and each permission
/// its manifest declares with what it lets the Plugin do and what the
/// engine cannot check — as the prompt's warning prints them once before
/// its approval question (ADR 0033, issues 900 and 907).
fn stated(added: &PluginAdded) -> String {
    let mut text = format!("The Plugin {} {} grants:", added.name, added.version);
    if let Some(signer) = &added.signer {
        writeln!(text, "\n  Signer: {signer}").expect("invariant: writing into a String");
    }
    let kinds: Vec<&str> = added
        .kinds
        .iter()
        .map(goodfetch_protocol::plugin::PluginKindName::as_str)
        .collect();
    let sep = if added.signer.is_some() { "" } else { "\n" };
    writeln!(text, "{sep}  Kinds: {}", joined(&kinds)).expect("invariant: writing into a String");
    writeln!(text, "  Network level: {}", added.network_level.as_str())
        .expect("invariant: writing into a String");
    let env: Vec<String> = added
        .env
        .iter()
        .map(|(variable, setting)| format!("{variable} <- {setting}"))
        .collect();
    writeln!(text, "  Environment variables: {}", joined(&env))
        .expect("invariant: writing into a String");
    writeln!(text, "  Files: {}", joined(&added.files)).expect("invariant: writing into a String");
    writeln!(text, "  Processes: {}", joined(&added.processes))
        .expect("invariant: writing into a String");
    if added.browsers.is_empty() {
        writeln!(text, "  Browsers: none").expect("invariant: writing into a String");
    } else {
        writeln!(text, "  Browsers:").expect("invariant: writing into a String");
        for browser in &added.browsers {
            let platforms: Vec<String> = browser
                .platforms
                .iter()
                .map(|(platform, download)| format!("{platform}: {}", download.digest))
                .collect();
            writeln!(text, "    - {} ({})", browser.name, joined(&platforms))
                .expect("invariant: writing into a String");
        }
    }
    if added.permissions.is_empty() {
        write!(text, "  Permissions: none (it may run, and nothing more)")
            .expect("invariant: writing into a String");
    } else {
        write!(
            text,
            "  Permissions ({}, each approved on its own):",
            added.permissions.len()
        )
        .expect("invariant: writing into a String");
        for stated in &added.permissions {
            write!(
                text,
                "\n    - it may {}.\n      The engine cannot check {}.",
                stated.lets, stated.cannot_check
            )
            .expect("invariant: writing into a String");
        }
    }
    text
}

/// The items of `items` separated by `, `, or `none` when empty.
fn joined<S: AsRef<str>>(items: &[S]) -> String {
    if items.is_empty() {
        "none".to_owned()
    } else {
        items
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(", ")
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
