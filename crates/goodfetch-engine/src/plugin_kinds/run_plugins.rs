//! One Run's process Plugins (ADR 0033): the instances its calls start,
//! one per Plugin a setting names, each lazily, on the first call that
//! names it, and stopped with the Run's owner.
//!
//! A Plugin starts from its bundle in the Plugin store, confined to its
//! bundle and to its own temporary folder in Goodfetch home's
//! `instances`, which the Plugin's process deletes as it stops. Its
//! settings in the Run's `plugins` are checked against its manifest's
//! schema as it starts, and its environment holds only the variables its
//! manifest declares, valued from them ([`plugin_settings`]). A Plugin on
//! a runtime waits for the runtime cache (issue 895), and the approvals in
//! Operator policy for issue 900.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_confinement::deny_list::Places;
use goodfetch_confinement::launch::Launch;
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::{PluginKindName, PluginManifest, Runtime};
use goodfetch_protocol::run::RunId;

use super::extractor::{ProcessExtractor, unavailable};
use crate::call_error::CallError;
use crate::home::Home;
use crate::plugin_host::instance::{self, PluginInstance, Scope};
use crate::plugin_host::settings::{self as plugin_settings, EnvironmentError, PluginSettings};
use crate::plugin_store::{Installed, PluginStore, StoreError};
use crate::sandbox_probe::SandboxProbe;
use crate::secrets::{RunSecrets, SecretError};
use crate::settings::Resolved;
use crate::tasks::TaskOwner;

/// One Run's process Plugins, started as its calls name them.
pub struct RunPlugins {
    store: PluginStore,
    instances: PathBuf,
    places: Option<Arc<Places>>,
    product: Product,
    run_id: RunId,
    owner: Arc<TaskOwner>,
    /// The Extractors started, by name: only those that started, so a
    /// Plugin installed after a failed start is found on the next call.
    extractors: tokio::sync::Mutex<BTreeMap<String, ProcessExtractor>>,
}

impl std::fmt::Debug for RunPlugins {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunPlugins")
            .field("run_id", &self.run_id)
            .finish_non_exhaustive()
    }
}

impl RunPlugins {
    /// The Plugins of the Run `run_id` of `product`'s engine at `home`,
    /// confined away from `places`, none of which starts without them, and
    /// started as tasks of the Run's `owner`.
    pub fn new(
        home: &Home,
        places: Option<Arc<Places>>,
        product: Product,
        run_id: RunId,
        owner: Arc<TaskOwner>,
    ) -> Self {
        Self {
            store: PluginStore::new(home.plugins()),
            instances: home.plugin_instances(),
            places,
            product,
            run_id,
            owner,
            extractors: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// The Extractor of the installed Plugin `name`, its instance started
    /// on the first call with its settings in the Run's `settings` and its
    /// environment valued from them, a secret field's reference resolved
    /// by the Run's `secrets`, once the engine's `probe` finds that this
    /// host confines a Plugin (ADR 0049).
    ///
    /// # Errors
    ///
    /// [`StartError::Secret`] for a secret field whose reference gives no
    /// value; otherwise [`StartError::Refused`]: `setting_invalid` naming
    /// the field for settings the Plugin's schema refuses,
    /// `confinement_unavailable` naming the environment and its fix where
    /// this host cannot confine the instance, and
    /// `extractor_unavailable` when no Plugin of the Extractor kind is
    /// installed under `name`, or its instance cannot be set up; why is
    /// logged at `warn`. A Plugin that cannot launch fails its first call
    /// instead, as its instance answers.
    pub async fn extractor(
        &self,
        name: &str,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessExtractor, StartError> {
        let mut extractors = self.extractors.lock().await;
        if let Some(extractor) = extractors.get(name) {
            return Ok(extractor.clone());
        }
        let cannot_start = |reason: &dyn std::fmt::Display| {
            tracing::warn!(extractor = name, %reason, "an Extractor's Plugin cannot start");
            StartError::Refused(unavailable(name))
        };
        let store = self.store.clone();
        let named = name.to_owned();
        let installed = self
            .owner
            .blocking(move || store.resolve(&named, &PluginKindName::EXTRACT))
            .await
            .map_err(|error| cannot_start(&error))?;
        let (given, layer) = plugin_settings::given(settings, name);
        let environment = PluginSettings::checked(name, &installed.manifest, given, layer)
            .map_err(|invalid| StartError::Refused(invalid.into()))?
            .environment(name, &installed.manifest, secrets, (&self.run_id, layer))
            .await
            .map_err(|error| match error {
                EnvironmentError::Secret(error) => StartError::Secret(error),
                ungranted @ EnvironmentError::Ungranted { .. } => cannot_start(&ungranted),
            })?;
        let probe = probe.clone();
        if let Some(refusal) = self.owner.blocking(move || probe.plugin_refusal()).await {
            tracing::warn!(extractor = name, %refusal, "this host cannot confine an Extractor's Plugin");
            return Err(StartError::Refused(refusal));
        }
        let folder = self.instances.join(format!("{}-{name}", self.run_id));
        let places = self.places.clone();
        let plugin = self
            .owner
            .blocking(move || set_up(installed, environment, &folder, places.as_deref()))
            .await
            .map_err(|reason| cannot_start(&reason))?;
        let instance = PluginInstance::start(
            &self.owner,
            instance::Plugin {
                launcher: plugin.launcher,
                launch: plugin.launch,
                manifest: plugin.manifest,
                product: self.product,
            },
            Scope::Run(self.run_id.clone()),
        );
        let extractor = ProcessExtractor::new(name.to_owned(), instance);
        extractors.insert(name.to_owned(), extractor.clone());
        Ok(extractor)
    }

    /// Checks that a Plugin of the Extractor kind is installed under
    /// `name`, as a setting that names it is set: nothing starts.
    ///
    /// # Errors
    ///
    /// Why the store holds no such Plugin, which names it and no path.
    pub async fn check_extractor(&self, name: &str) -> Result<(), StoreError> {
        let store = self.store.clone();
        let name = name.to_owned();
        self.owner
            .blocking(move || store.resolve(&name, &PluginKindName::EXTRACT).map(drop))
            .await
    }
}

/// Why an Extractor's instance does not start.
#[derive(Debug)]
pub enum StartError {
    /// The call's failure: its settings refused, or the Plugin unavailable.
    Refused(CallError),
    /// A secret field's reference gave no value, which the caller answers
    /// as any Secret's failure.
    Secret(SecretError),
}

/// What starts one instance, set up on the blocking policy.
struct SetUp {
    launcher: PathBuf,
    launch: Launch,
    manifest: PluginManifest,
}

/// Sets up the instance of `installed`, an Extractor, with `environment`,
/// its folder `folder`, under `places`. Its error is the log's text, which
/// may name paths.
#[expect(
    clippy::disallowed_methods,
    reason = "called on the tasks module's blocking policy, never on an async worker"
)]
fn set_up(
    installed: Installed,
    environment: BTreeMap<String, String>,
    folder: &Path,
    places: Option<&Places>,
) -> Result<SetUp, String> {
    let places =
        places.ok_or("the engine's environment names no home folder to confine it from")?;
    if installed.manifest.runtime != Runtime::None {
        return Err("its runtime is not yet installed by the runtime cache (issue 895)".to_owned());
    }
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let canonical = |path: &Path| std::fs::canonicalize(path).map_err(|error| error.to_string());
    let bundle = canonical(&installed.bundle)?;
    let folder = canonical(folder)?;
    let instance = Instance {
        bundle: bundle.clone(),
        runtime: None,
        folders: vec![folder.clone()],
        env: environment,
    };
    let policy =
        Policy::of(&installed.manifest, &instance, places).map_err(|error| error.to_string())?;
    let launcher = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(SetUp {
        launcher,
        launch: Launch {
            policy,
            program: bundle.join(&installed.manifest.entrypoint),
            args: Vec::new(),
            folder,
        },
        manifest: installed.manifest,
    })
}
