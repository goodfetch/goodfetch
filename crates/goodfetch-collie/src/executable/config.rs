//! `config explain` (ADRs 0028 and 0083), a command of both Products'
//! executables: each effective setting with the layer that set it and the
//! layer that locked it, from the Goodfetch home's Operator policy and user
//! configuration, the project configuration found from the working folder
//! upward (or the one `--project` names) and the run values of the
//! settings flags, `--set` and `--settings`; keys only the other Product honours are listed as that
//! Product's. Its `--json` output is `goodfetch-protocol`'s type. It asks
//! `config.explain`, with the project configuration and flags it read, of
//! an Attached engine in this process, or under `--daemon` of the Daemon:
//! the remote one `GOODFETCH_URL` names when it is set, else the major's,
//! started if none runs. Both resolve as the engine's operation does.
//!
//! A setting it cannot take, such as a key no Product knows in a file,
//! prints as every command's failure does and exits with its Error code's
//! row of the exit-code table (ADR 0069).
//!
//! The human output is one line per key, as `git config --show-origin`
//! shows each value's origin: `key = value  (layer, locked by layer)`, a
//! run value's layer with its origin, `run from flag`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use goodfetch_engine::run::local_settings_error;
use goodfetch_engine::settings::{SettingsError, find_project, project_settings};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::settings::{
    CONFIG_EXPLAIN, ConfigExplainParams, ExplainedSetting, ProjectSettings, RunOrigin,
    SettingsExplanation,
};
use serde_json::{Map, Value};

use super::daemon::Machine;
use super::engine_choice::{Choice, EngineChoice};
use super::exit_codes;
use super::operation::run_operation;
use super::output::{self, Human};

/// What `config explain` was asked.
pub(super) struct Explain {
    /// The keys or prefixes to show; every key when empty.
    pub(super) keys: Vec<String>,
    /// The run values the settings flags give.
    pub(super) settings: Map<String, Value>,
    /// The project configuration to read instead of the one found from
    /// the working folder upward.
    pub(super) project: Option<PathBuf>,
    /// Whether to print JSON.
    pub(super) json: bool,
}

/// `config explain`: `config.explain` on the engine `choice` chooses, an
/// Attached engine in this process, `engine` on `dependencies`, or the
/// Daemon `machine` reaches under `--daemon`.
pub(super) fn explain(
    (engine, dependencies): (&Engine, Dependencies),
    machine: &Machine<'_>,
    choice: Choice,
    request: &Explain,
    human: &Human,
) -> ExitCode {
    let json = request.json;
    let Ok(folder) = std::env::current_dir() else {
        tracing::error!("the working folder cannot be read");
        return ExitCode::FAILURE;
    };
    let params = match params(request, &folder) {
        Ok(params) => params,
        Err(error) => {
            return output::fail(
                local_settings_error(&error, engine.identity()).to_error_object(),
                None,
                json,
                human,
            );
        }
    };
    let params = serde_json::to_value(params).expect("invariant: the params serialize");
    let warn = |text: &str| human.warning(text);
    let reach = EngineChoice::passing(choice, machine, &warn);
    let explained = match run_operation(
        engine,
        dependencies,
        &reach,
        &CONFIG_EXPLAIN,
        params,
        json,
        human,
    ) {
        Ok(explained) => explained,
        Err(exit) => return exit,
    };
    match serde_json::from_value::<SettingsExplanation>(explained) {
        Ok(explanation) => {
            print(&explanation, json);
            ExitCode::SUCCESS
        }
        Err(problem) => output::fail(
            exit_codes::internal(format!(
                "The engine answered `{}` with no explanation: {problem}.",
                CONFIG_EXPLAIN.name
            )),
            Some(CONFIG_EXPLAIN.name),
            json,
            human,
        ),
    }
}

/// What `config.explain` takes for `request`: the project configuration
/// this process read, found from `folder` upward unless named, and the
/// flags.
fn params(request: &Explain, folder: &Path) -> Result<ConfigExplainParams, SettingsError> {
    let project_path = request.project.clone().or_else(|| find_project(folder));
    let project = match project_path {
        Some(path) => Some(ProjectSettings {
            settings: project_settings(&path)?.unwrap_or_default(),
            path: path.display().to_string(),
        }),
        None => None,
    };
    Ok(ConfigExplainParams {
        keys: request.keys.clone(),
        project,
        settings: request.settings.clone(),
        origin: Some(RunOrigin::Flag),
    })
}

fn print(explanation: &SettingsExplanation, json: bool) {
    if json {
        output::json(explanation);
    } else {
        for setting in &explanation.settings {
            output::line(&describe(setting));
        }
    }
}

fn describe(setting: &ExplainedSetting) -> String {
    let mut origin = vec![match setting.origin {
        Some(run_origin) => format!("{} from {}", setting.layer.as_str(), run_origin.as_str()),
        None => setting.layer.as_str().to_owned(),
    }];
    if let Some(locked_by) = setting.locked_by {
        origin.push(format!("locked by {}", locked_by.as_str()));
    }
    if let Some(cap) = &setting.cap {
        origin.push(format!("capped at {cap}"));
    }
    if setting.weakens {
        origin.push("weakens a guarantee".to_owned());
    }
    if !setting.applied {
        let products: Vec<&str> = setting
            .products
            .iter()
            .map(|product| product.name())
            .collect();
        origin.push(format!("{}'s, not applied", products.join(" and ")));
    }
    format!(
        "{} = {}  ({})",
        setting.key,
        setting.value,
        origin.join(", ")
    )
}
