//! Settings (ADR 0028 as amended by ADRs 0083, 0087 and 0098): every engine
//! parameter resolves through five layers, from its built-in default
//! through the Operator policy, user and project configuration and a
//! Recipe to the run, with locks, caps, a Recipe's exposed bounds and each
//! key's authority, and every effective value can say which layer set it
//! and which locked it (`config explain`, the run report). Both Products
//! read one major's Operator policy and user configuration from Goodfetch
//! home, each applying its own keys and listing the other's.
//!
//! Its interface: [`Settings`] reads the machine's layers and resolves a
//! Run's [`RunSettings`]; [`resolve`] is the pure resolver over [`Layers`];
//! [`SettingsError`] is every refusal, with its Error code; the files'
//! reading and rewriting and the environment map's check are its other
//! entries.

mod environment;
mod error;
mod files;
mod hosts;
mod keys;
mod resolve;
#[cfg(test)]
mod tests;
mod values;

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{ProjectSettings, RecipeManifest, RunOrigin, SettingsLayer};
use serde_json::{Map, Value};

pub use environment::{check_environment, deployment_variable};
pub use error::SettingsError;
pub use files::{
    LOWEST_MINOR_KEY, MachineLayers, PROJECT_CONFIGURATION_FILE, SettingsFiles, configuration,
    find_project, fuzz_settings_file, load_project, operator_policy, policy_version,
    project_settings, rewrite, set, user_configuration,
};
pub use hosts::HostEntry;
pub use resolve::{
    Effective, HostDefaults, Layers, Policy, Recipe, Resolved, SettingValue, Unapplied,
    catalog_default, catalog_value, get_or_default, resolve, value_as,
};
pub use values::Values;

use crate::tasks::TaskOwner;

/// The engine's settings: where its machine's layers are, and the Product
/// whose keys it applies.
#[derive(Clone)]
pub struct Settings(Arc<Inner>);

impl std::fmt::Debug for Settings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Settings")
            .field("files", &self.0.files)
            .field("product", &self.0.product)
            .finish_non_exhaustive()
    }
}

struct Inner {
    files: SettingsFiles,
    product: Product,
    owner: TaskOwner,
    work: CountedWork,
    host: Host,
}

/// How the engine finds its host's defaults, and what it found: found once,
/// on first use, since finding them may open the OS keychain.
struct Host {
    find: Box<dyn Fn() -> Finding + Send + Sync>,
    found: tokio::sync::OnceCell<HostDefaults>,
}

/// A search for the host's defaults.
pub type Finding = std::pin::Pin<Box<dyn std::future::Future<Output = HostDefaults> + Send>>;

/// What a scope that opens gives the resolver beside the machine's layers.
#[derive(Clone, Debug, Default)]
pub struct Opening {
    /// The Product the Run needs.
    pub run: Option<Product>,
    /// The project configuration the Client sent.
    pub project: Option<ProjectSettings>,
    /// The Recipe, when the Run is one's.
    pub recipe: Option<RecipeManifest>,
    /// The run layer's values, nested by key.
    pub settings: Map<String, Value>,
    /// Where the run layer's values came from.
    pub origin: Option<RunOrigin>,
    /// Whether a Client acting for a model gave the run layer's values and
    /// the project configuration, so that a key no model sets is refused
    /// (ADR 0092).
    pub acting_for_model: bool,
}

impl Settings {
    /// The settings of an engine of `product` whose layers are `files`,
    /// reading them on `owner`'s blocking policy, with the catalog's
    /// defaults.
    pub fn new(files: SettingsFiles, product: Product, owner: TaskOwner) -> Self {
        Self::with_host(files, product, owner, CountedWork::default(), || {
            Box::pin(std::future::ready(HostDefaults::default()))
        })
    }

    /// The same, with the defaults `find` gives for this host, which it
    /// calls once, on first use, each read of its layers counted on `work`.
    pub fn with_host(
        files: SettingsFiles,
        product: Product,
        owner: TaskOwner,
        work: CountedWork,
        find: impl Fn() -> Finding + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Inner {
            files,
            product,
            owner,
            work,
            host: Host {
                find: Box::new(find),
                found: tokio::sync::OnceCell::new(),
            },
        }))
    }

    /// Where its files are.
    pub fn files(&self) -> &SettingsFiles {
        &self.0.files
    }

    /// The Product whose keys it applies.
    pub fn product(&self) -> Product {
        self.0.product
    }

    /// Reads the machine's layers now.
    ///
    /// # Errors
    ///
    /// As [`SettingsFiles::load`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the read to finish on its own.
    pub async fn machine(&self) -> Result<MachineLayers, SettingsError> {
        let settings = self.clone();
        self.0
            .owner
            .blocking(move || settings.machine_blocking())
            .await
    }

    /// Reads the machine's layers now, on the calling thread: for work on
    /// the blocking policy already, such as an edit of the settings files
    /// under their lock. Blocking.
    ///
    /// # Errors
    ///
    /// As [`SettingsFiles::load`].
    pub(crate) fn machine_blocking(&self) -> Result<MachineLayers, SettingsError> {
        self.0.work.count(Row::ConfigurationReads);
        self.0.files.load(self.0.product)
    }

    /// The defaults this host gives in place of the catalog's, found on the
    /// first call.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the search to the next call.
    pub async fn host_defaults(&self) -> HostDefaults {
        let host = &self.0.host;
        host.found.get_or_init(|| (host.find)()).await.clone()
    }

    /// Resolves the settings of a scope that opens with `opening`, against
    /// the machine's layers as they are now; refused before the scope opens
    /// (ADR 0083), a setting whose feature the engine does not build yet
    /// included, when a layer sets it away from its default.
    ///
    /// # Errors
    ///
    /// Every refusal of [`SettingsError`], the first found.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future resolves nothing.
    pub async fn open(
        &self,
        opening: Opening,
    ) -> Result<(RunSettings, MachineLayers), SettingsError> {
        let machine = self.machine().await?;
        let engine = self.0.product;
        let mut project = match &opening.project {
            Some(project) => Values::read(
                SettingsLayer::ProjectConfiguration,
                &project.settings,
                engine,
            )?,
            None => Values::default(),
        };
        let recipe = opening
            .recipe
            .as_ref()
            .map(|manifest| Recipe::read(manifest, engine))
            .transpose()?;
        let mut run = Values::read(SettingsLayer::Run, &opening.settings, engine)?;
        if let Some(origin) = opening.origin {
            run = run.from_origin(origin);
        }
        if opening.acting_for_model {
            // The Client sends the project configuration it found, so a
            // model gives it as much as the run layer (ADR 0092).
            project = project.from_model();
            run = run.from_model();
        }
        let layers = Layers {
            host: self.host_defaults().await,
            operator_policy: machine.operator_policy.clone(),
            user_configuration: machine.user_configuration.clone(),
            project_configuration: project,
            recipe,
            run,
        };
        let resolved = resolve(&layers, engine, opening.run.unwrap_or(engine))
            .and_then(|resolved| resolved.refuse_unbuilt().map(|()| resolved))
            .map_err(|error| self.0.files.named_in(error))?;
        Ok((
            RunSettings {
                resolved,
                project: opening.project,
            },
            machine,
        ))
    }
}

/// One Run's resolved settings, resolved once, as it opens: a Run reads the
/// machine's layers no more, so a change to them, a tightened Operator
/// policy cap included, reaches the next Run (ADR 0138).
#[derive(Clone)]
pub struct RunSettings {
    resolved: Resolved,
    project: Option<ProjectSettings>,
}

impl RunSettings {
    /// What the Run opened with.
    pub fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    /// The path of the project configuration the Run opened with, when it
    /// sets `key`, nested as its file nests it.
    pub fn project_setting(&self, key: &str) -> Option<&str> {
        let project = self.project.as_ref()?;
        nested(&project.settings, key).map(|_| project.path.as_str())
    }
}

/// The catalog's setting named `name`, for a name read from a layer's
/// values or a request; code that knows its key names the constant.
pub fn setting(name: &str) -> Option<&'static goodfetch_protocol::catalog::Setting> {
    keys::find(name).map(|found| found.setting)
}

/// The protocol version that added the setting `key`, when one has it.
pub fn arrived_in(key: &str) -> Option<goodfetch_protocol::catalog::ProtocolVersion> {
    keys::find(key).map(|found| found.setting.arrived_in)
}

/// The value of `key` in `settings`, nested as a file nests it, or written
/// whole as one dotted key.
pub fn nested<'a>(settings: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    if let Some(value) = settings.get(key) {
        return Some(value);
    }
    let mut segments = key.split('.');
    let mut value = settings.get(segments.next()?)?;
    for segment in segments {
        value = value.get(segment)?;
    }
    Some(value)
}
