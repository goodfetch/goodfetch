//! One Run's process Plugins (ADR 0033): the instances its calls start,
//! one per Plugin a setting names, each lazily, on the first call that
//! names it, and stopped with the Run's owner.
//!
//! A Plugin starts from its bundle in the Plugin store, confined to its
//! bundle and to its own temporary folder in Goodfetch home's
//! `instances`, which the Plugin's process deletes as it stops. Its
//! settings, and the variables they value, are issue 861's: until then a
//! Plugin starts with an empty environment. A Plugin on a runtime waits for
//! the runtime cache (issue 895), and the approvals in Operator policy for
//! issue 900.

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
use crate::plugin_store::{PluginStore, StoreError};
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
    /// on the first call.
    ///
    /// # Errors
    ///
    /// `extractor_unavailable` when no Plugin of the Extractor kind is
    /// installed under `name`, or its instance cannot be set up; why is
    /// logged at `warn`. A Plugin that cannot launch fails its first call
    /// instead, as its instance answers.
    pub async fn extractor(&self, name: &str) -> Result<ProcessExtractor, CallError> {
        let mut extractors = self.extractors.lock().await;
        if let Some(extractor) = extractors.get(name) {
            return Ok(extractor.clone());
        }
        let store = self.store.clone();
        let folder = self.instances.join(format!("{}-{name}", self.run_id));
        let places = self.places.clone();
        let named = name.to_owned();
        let plugin = self
            .owner
            .blocking(move || set_up(&store, &named, &folder, places.as_deref()))
            .await
            .map_err(|reason| {
                tracing::warn!(extractor = name, %reason, "an Extractor's Plugin cannot start");
                unavailable(name)
            })?;
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

/// What starts one instance, set up on the blocking policy.
struct SetUp {
    launcher: PathBuf,
    launch: Launch,
    manifest: PluginManifest,
}

/// Resolves the Extractor `name` in `store` and sets up its instance's
/// folder `folder`, under `places`. Its error is the log's text, which may
/// name paths.
#[expect(
    clippy::disallowed_methods,
    reason = "called on the tasks module's blocking policy, never on an async worker"
)]
fn set_up(
    store: &PluginStore,
    name: &str,
    folder: &Path,
    places: Option<&Places>,
) -> Result<SetUp, String> {
    let installed = store
        .resolve(name, &PluginKindName::EXTRACT)
        .map_err(|error| error.to_string())?;
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
        env: BTreeMap::new(),
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
