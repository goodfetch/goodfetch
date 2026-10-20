//! The `config.*` operations every Product carries (ADRs 0028 and 0083):
//! `config.explain` answers each setting's effective value, the layer that
//! set it and the layer that locked it, from the engine's own layers and
//! the project configuration and run values a Client gives.

use goodfetch_protocol::settings::{
    CONFIG_EXPLAIN, ConfigExplainParams, RunOrigin, SettingsExplanation,
};

use crate::composition::{CallContext, CallError, Handler};
use crate::run::settings_error;
use crate::settings::Opening;

/// The `config.*` handlers.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&CONFIG_EXPLAIN, explain)]
}

async fn explain(
    params: ConfigExplainParams,
    context: CallContext,
) -> Result<SettingsExplanation, CallError> {
    let settings = context.runs().settings();
    let (run, machine) = settings
        .open(Opening {
            run: None,
            project: params.project.clone(),
            recipe: None,
            settings: params.settings,
            origin: Some(RunOrigin::Settings),
        })
        .await
        .map_err(|error| settings_error(&error, context.identity()))?;
    let mut files = machine.files;
    if let Some(project) = params.project {
        files.push(goodfetch_protocol::settings::SettingsFile {
            layer: goodfetch_protocol::settings::SettingsLayer::ProjectConfiguration,
            path: project.path,
            found: true,
        });
    }
    Ok(run.resolved().explain(&params.keys, files))
}
