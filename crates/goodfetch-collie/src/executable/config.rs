//! `config explain` (ADRs 0028 and 0083), a command of both Products'
//! executables: each effective setting with the layer that set it and the
//! layer that locked it, from the Goodfetch home's Operator policy and user
//! configuration, the project configuration found from the working folder
//! upward (or the one `--project` names) and the run values of the
//! settings flags, `--set` and `--settings`; keys only the other Product honours are listed as that
//! Product's. Its `--json` output is `goodfetch-protocol`'s type. The
//! executable resolves in process, as its Attached engine does, or under
//! `--daemon` asks the major's Daemon's `config.explain`, starting the
//! Daemon if none runs, with the project configuration and flags it read.
//!
//! The human output is one line per key, as `git config --show-origin`
//! shows each value's origin: `key = value  (layer, locked by layer)`, a
//! run value's layer with its origin, `run from flag`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use goodfetch_engine::settings::{
    HostDefaults, Layers, SettingsError, SettingsFiles, Values, find_project, load_project,
    project_settings, resolve,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::settings::{
    ConfigExplainParams, ExplainedSetting, ProjectSettings, RunOrigin, SettingsExplanation,
    SettingsFile, SettingsLayer,
};
use serde_json::{Map, Value};

use super::daemon::{Failure, Machine};
use goodfetch_cli::output;
use goodfetch_engine::home::Home;
use goodfetch_engine::secrets::Keychain;

/// What `config explain` was asked.
pub(super) struct Explain {
    /// The keys or prefixes to show; every key when empty.
    pub(super) keys: Vec<String>,
    /// The run values the settings flags give.
    pub(super) settings: Map<String, Value>,
    /// The project configuration to read instead of the one found.
    pub(super) project: Option<PathBuf>,
    /// The folder the project configuration is found from.
    pub(super) folder: PathBuf,
    /// Whether to print JSON.
    pub(super) json: bool,
}

/// `config explain`.
pub(super) fn explain(
    product: Product,
    version: &ProductVersion,
    home: &Home,
    environment: &BTreeMap<OsString, OsString>,
    request: &Explain,
) -> ExitCode {
    match explanation(product, version, home, environment, request) {
        Ok(explanation) => {
            print(&explanation, request.json);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(code = error.code(), "{error}");
            ExitCode::FAILURE
        }
    }
}

/// `config explain --daemon`: the same, from the Daemon of the major.
pub(super) fn explain_through(machine: &Machine<'_>, request: &Explain) -> ExitCode {
    let params = match daemon_params(request) {
        Ok(params) => params,
        Err(error) => {
            tracing::error!(code = error.code(), "{error}");
            return ExitCode::FAILURE;
        }
    };
    let explained = machine
        .connect(true, machine.identity.product)
        .and_then(|connected| {
            let mut connected = connected.ok_or_else(|| {
                Failure::Unavailable("the Daemon stopped as soon as it started".to_owned())
            })?;
            let explanation = connected.call::<SettingsExplanation>("config.explain", &params);
            connected.close();
            explanation
        });
    match explained {
        Ok(explanation) => {
            print(&explanation, request.json);
            ExitCode::SUCCESS
        }
        Err(failure) => failure.report(),
    }
}

/// What `config.explain` takes for `request`: the project configuration
/// and flags this process read.
fn daemon_params(request: &Explain) -> Result<ConfigExplainParams, SettingsError> {
    let project_path = request
        .project
        .clone()
        .or_else(|| find_project(&request.folder));
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

fn explanation(
    product: Product,
    version: &ProductVersion,
    home: &Home,
    environment: &BTreeMap<OsString, OsString>,
    request: &Explain,
) -> Result<SettingsExplanation, SettingsError> {
    let settings_files = SettingsFiles::of(home, version);
    let machine = settings_files.load(product)?;
    let project_path = request
        .project
        .clone()
        .or_else(|| find_project(&request.folder));
    let project = match &project_path {
        Some(path) => load_project(path, product)?,
        None => Values::default(),
    };
    let run =
        Values::read(SettingsLayer::Run, &request.settings, product)?.from_origin(RunOrigin::Flag);
    let layers = Layers {
        host: host_defaults(environment),
        operator_policy: machine.operator_policy,
        user_configuration: machine.user_configuration,
        project_configuration: project,
        recipe: None,
        run,
    };
    let mut files: Vec<SettingsFile> = machine.files;
    if let Some(path) = &project_path {
        files.push(SettingsFile {
            layer: SettingsLayer::ProjectConfiguration,
            path: path.display().to_string(),
            found: Path::new(path).is_file(),
        });
    }
    let resolved =
        resolve(&layers, product, product).map_err(|error| settings_files.named_in(error))?;
    Ok(resolved.explain(&request.keys, files))
}

/// The defaults this host gives in place of the catalog's, found as the
/// engine finds them: the OS keychain opens over its own runtime, and
/// Chromium's sandbox is probed in `environment`.
fn host_defaults(environment: &BTreeMap<OsString, OsString>) -> HostDefaults {
    #[expect(
        clippy::disallowed_methods,
        reason = "main builds the runtime (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its thread");
    goodfetch_engine::sandbox_probe::host_defaults(
        runtime.block_on(goodfetch_engine::secrets::host_defaults(&Keychain::os())),
        goodfetch_engine::sandbox_probe::probed_in(environment),
    )
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
