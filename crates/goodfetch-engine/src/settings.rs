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
//! Run's [`RunSettings`], and keeps the engine's snapshot of the machine's
//! settings, [`MachineSettings`], for the work that reads them outside any
//! Run, such as the Resource governor and the Audit log; [`resolve`] is the
//! pure resolver over [`Layers`];
//! [`SettingsError`] is every refusal, with its Error code; the files'
//! reading and rewriting and the environment map's check are its other
//! entries.

mod environment;
mod error;
mod files;
mod hosts;
mod keys;
mod resolve;
mod scopes;
#[cfg(test)]
mod tests;
mod values;

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{ProjectSettings, RecipeManifest, RunOrigin, SettingsLayer};
use serde_json::{Map, Value};

pub use environment::{check_environment, deployment_variable, taken_value};
pub use error::SettingsError;
pub use files::{
    LOWEST_MINOR_KEY, MachineLayers, SettingsFiles, configuration, find_project,
    fuzz_settings_file, operator_policy, policy_version, project_settings, rewrite, set,
    user_configuration,
};
pub use hosts::HostEntry;
pub(crate) use keys::fixed_keys;
pub use resolve::{
    Effective, HostDefaults, Layers, Policy, Recipe, Resolved, SettingValue, Unapplied,
    catalog_default, catalog_value, get_or_default, resolve, value_as,
};
pub use scopes::{beneath_connection, check_client_bounds, check_scope};
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
    snapshot: Snapshot,
}

/// The engine's snapshot of the machine's settings (ADR 0140): read when
/// first asked for, as the engine starts, and again at each read of the
/// machine's layers, a Run's admission included, and after each of the
/// engine's own writes of its files.
struct Snapshot {
    /// The first read, for an engine that kept none before it was asked.
    first: tokio::sync::OnceCell<()>,
    /// The reads begun and the one kept.
    kept: tokio::sync::watch::Sender<Kept>,
}

/// What [`Snapshot`] holds: a read begun earlier than the one kept never
/// replaces it, so a slow read cannot bring back a value a later one
/// changed.
#[derive(Default)]
struct Kept {
    /// The reads begun, each numbered in turn.
    begun: u64,
    /// The number of the read kept, 0 for none.
    read: u64,
    /// What it read.
    machine: Option<Arc<MachineSettings>>,
}

/// The machine's settings as one read of them found: its Operator policy
/// and user configuration, and what they resolve to on the engine's
/// Product before any Run, on the catalog's defaults.
#[derive(Debug)]
pub struct MachineSettings {
    layers: Result<MachineLayers, SettingsError>,
    resolved: Result<Resolved, SettingsError>,
}

impl MachineSettings {
    fn of(layers: Result<MachineLayers, SettingsError>, engine: Product) -> Self {
        let resolved = layers.clone().and_then(|machine| machine.resolve(engine));
        Self { layers, resolved }
    }

    /// The layers read.
    ///
    /// # Errors
    ///
    /// As [`SettingsFiles::load`].
    pub fn layers(&self) -> Result<&MachineLayers, &SettingsError> {
        self.layers.as_ref()
    }

    /// What they resolve to.
    ///
    /// # Errors
    ///
    /// As [`SettingsFiles::load`] and [`MachineLayers::resolve`].
    pub fn resolved(&self) -> Result<&Resolved, &SettingsError> {
        self.resolved.as_ref()
    }
}

/// How the engine finds its host's defaults, and what it found of those
/// every opening reads: found once, on first use, since finding them waits
/// for the sandbox probe.
struct Host {
    search: HostSearch,
    found: tokio::sync::OnceCell<HostDefaults>,
}

/// A search for the host's defaults.
pub type Finding = std::pin::Pin<Box<dyn std::future::Future<Output = HostDefaults> + Send>>;

/// How an engine finds its host's defaults, in two parts: those every
/// opening of a Run's settings reads, and those only asked for where their
/// value is needed, since asking may open the OS keychain, on Linux a
/// Secret Service session over D-Bus (issue 2204).
pub struct HostSearch {
    /// Finds those every opening reads, such as the sandbox probe's;
    /// called once, on first use.
    opening: Box<dyn Fn() -> Finding + Send + Sync>,
    /// Finds those only asked for, such as `run_state.key_reference`'s;
    /// called at each ask.
    asked: Box<dyn Fn() -> Finding + Send + Sync>,
    /// Those of `asked` already found, without finding anything.
    known: Box<dyn Fn() -> Option<HostDefaults> + Send + Sync>,
}

impl HostSearch {
    /// A search whose every default `opening` finds, none asked for.
    pub fn opening(find: impl Fn() -> Finding + Send + Sync + 'static) -> Self {
        Self {
            opening: Box::new(find),
            asked: Box::new(|| Box::pin(std::future::ready(HostDefaults::default()))),
            known: Box::new(|| Some(HostDefaults::default())),
        }
    }

    /// The same, with the defaults `asked` finds only where their value is
    /// needed, and `known` giving what it found, without finding anything:
    /// none before it first found them, and after, what it found.
    #[must_use]
    pub fn asking(
        self,
        asked: impl Fn() -> Finding + Send + Sync + 'static,
        known: impl Fn() -> Option<HostDefaults> + Send + Sync + 'static,
    ) -> Self {
        Self {
            asked: Box::new(asked),
            known: Box::new(known),
            ..self
        }
    }
}

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
    /// Whether the Client reached an Exposed Daemon, where the project
    /// configuration it sent keeps its rank with only the run layer's
    /// authority, so it widens nothing reserved to configuration (ADR
    /// 0083).
    pub exposed: bool,
}

impl Settings {
    /// The settings of an engine of `product` whose layers are `files`,
    /// reading them on `owner`'s blocking policy, with the catalog's
    /// defaults.
    pub fn new(files: SettingsFiles, product: Product, owner: TaskOwner) -> Self {
        Self::with_host(
            files,
            product,
            owner,
            CountedWork::default(),
            HostSearch::opening(|| Box::pin(std::future::ready(HostDefaults::default()))),
        )
    }

    /// The same, with the defaults `search` finds for this host, each read
    /// of its layers counted on `work`.
    pub fn with_host(
        files: SettingsFiles,
        product: Product,
        owner: TaskOwner,
        work: CountedWork,
        search: HostSearch,
    ) -> Self {
        Self(Arc::new(Inner {
            files,
            product,
            owner,
            work,
            host: Host {
                search,
                found: tokio::sync::OnceCell::new(),
            },
            snapshot: Snapshot {
                first: tokio::sync::OnceCell::new(),
                kept: tokio::sync::watch::Sender::new(Kept::default()),
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

    /// Reads the machine's layers now, and keeps them as the engine's
    /// snapshot.
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

    /// Reads the machine's layers now, on the calling thread, and keeps
    /// them as the engine's snapshot: for work on the blocking policy
    /// already, such as an edit of the settings files under their lock.
    /// Blocking.
    ///
    /// # Errors
    ///
    /// As [`SettingsFiles::load`].
    pub(crate) fn machine_blocking(&self) -> Result<MachineLayers, SettingsError> {
        self.read_blocking().layers.clone()
    }

    /// The engine's snapshot of the machine's settings: read on the first
    /// call, as the engine starts, unless a read of the machine's layers
    /// kept one before; the one kept after. It reads no file once kept
    /// (ADR 0140): a hand edit of either file is seen at the engine's next
    /// read of them, a Run's admission or its own write of them at the
    /// latest.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves a read begun to finish on its own.
    pub async fn snapshot(&self) -> Arc<MachineSettings> {
        let snapshot = &self.0.snapshot;
        snapshot
            .first
            .get_or_init(|| async {
                if snapshot.kept.borrow().machine.is_none() {
                    self.refresh().await;
                }
            })
            .await;
        snapshot
            .kept
            .borrow()
            .machine
            .clone()
            .expect("invariant: the first call keeps a read before it answers")
    }

    /// Reads the machine's layers again into the snapshot: what the engine
    /// does after it wrote its own settings files.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the read to finish on its own.
    pub async fn refresh(&self) {
        let settings = self.clone();
        self.0
            .owner
            .blocking(move || {
                settings.read_blocking();
            })
            .await;
    }

    /// [`Settings::refresh`], on the calling thread, for work on the
    /// blocking policy already. Blocking.
    pub(crate) fn refresh_blocking(&self) {
        self.read_blocking();
    }

    /// Reads the machine's layers and keeps them unless a read begun after
    /// this one was kept first, each read counted on
    /// `F2.configuration_reads` whoever asked (ADR 0140). Blocking.
    fn read_blocking(&self) -> Arc<MachineSettings> {
        self.0.work.count(Row::ConfigurationReads);
        let kept = &self.0.snapshot.kept;
        let mut ticket = 0;
        kept.send_modify(|kept| {
            kept.begun += 1;
            ticket = kept.begun;
        });
        let machine = Arc::new(MachineSettings::of(
            self.0.files.load(self.0.product),
            self.0.product,
        ));
        kept.send_if_modified(|kept| {
            let later = ticket > kept.read;
            if later {
                kept.read = ticket;
                kept.machine = Some(Arc::clone(&machine));
            }
            later
        });
        machine
    }

    /// The defaults this host gives in place of the catalog's, those only
    /// asked for included: it opens the OS keychain if no one has.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the search to the next call.
    pub async fn host_defaults(&self) -> HostDefaults {
        let asked = (self.0.host.search.asked)().await;
        self.opening_defaults().await.and(asked)
    }

    /// Those of the defaults only asked for that this host already found,
    /// without opening anything: none before the OS keychain was first
    /// asked.
    pub fn known_host_defaults(&self) -> Option<HostDefaults> {
        (self.0.host.search.known)()
    }

    /// The defaults this host gives that every opening reads, found on the
    /// first call.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the search to the next call.
    async fn opening_defaults(&self) -> HostDefaults {
        let host = &self.0.host;
        host.found
            .get_or_init(|| (host.search.opening)())
            .await
            .clone()
    }

    /// [`Settings::open`], under every default this host gives, those only
    /// asked for included, so it opens the OS keychain if no one has: what
    /// `config explain` shows.
    ///
    /// # Errors
    ///
    /// As [`Settings::open`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future resolves nothing.
    pub async fn open_asked(
        &self,
        opening: Opening,
    ) -> Result<(RunSettings, MachineLayers), SettingsError> {
        let host = self.host_defaults().await;
        self.open_under(opening, host).await
    }

    /// Resolves the settings of a scope that opens with `opening`, against
    /// the machine's layers as they are now; refused before the scope opens
    /// (ADR 0083), a setting whose feature the engine does not build yet
    /// included, when a layer sets it away from its default. A default
    /// only asked for of the host is the catalog's here: it opens no OS
    /// keychain (issue 2204).
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
        let host = self.opening_defaults().await;
        self.open_under(opening, host).await
    }

    async fn open_under(
        &self,
        opening: Opening,
        host: HostDefaults,
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
        if opening.exposed {
            project = project.with_run_authority();
        }
        let layers = Layers {
            host,
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
