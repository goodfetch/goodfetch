//! One Run's process Plugins (ADR 0033): the instances its calls start,
//! one per Plugin a setting names, each lazily, on the first call that
//! names it, and stopped with the Run's owner.
//!
//! A Plugin starts from its bundle in the Plugin store, confined to its
//! bundle and to its own temporary folder in Goodfetch home's
//! `instances`, which the Plugin's process deletes as it stops. Its
//! settings in the Run's `plugins` are checked against its manifest's
//! schema as it starts, and its environment holds only the variables its
//! manifest declares, valued from them ([`plugin_settings`]). It starts
//! only once the Operator policy lets it load and approves each Secret
//! value its environment holds (issue 900). A Plugin declaring a language
//! runtime runs from the shared runtime cache (issue 895).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_confinement::deny_list::Places;
use goodfetch_confinement::launch::Launch;
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::plugin::approvals::{
    Origin, PLUGIN_NOT_ALLOWED, PluginNotAllowedDetails, PluginRefusal, admit, fix,
};
use goodfetch_protocol::plugin::index::{PluginIndex, SignerIdentity};
use goodfetch_protocol::plugin::settings::{PLUGINS, PluginEntry};
use goodfetch_protocol::plugin::{PluginKindName, PluginManifest, Runtime};
use goodfetch_protocol::run::RunId;
use serde_json::Value;

use super::extractor::{ProcessExtractor, unavailable};
use crate::audit::AuditLog;
use crate::browser_cache::{BrowserCache, CacheLocation, HeldArtifact};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::home::Home;
use crate::plugin_gate::{Inserting, PluginFetch, permission_denied};
use crate::plugin_host::instance::{self, PluginInstance, Scope};
use crate::plugin_host::settings::{self as plugin_settings, EnvironmentError, PluginSettings};
use crate::plugin_index::{self, LoadRefusal, PluginIndexStore};
use crate::plugin_runtime::{self, RuntimeCatalog};
use crate::plugin_store::{Installed, PluginStore, StoreError};
use crate::sandbox_probe::SandboxProbe;
use crate::secrets::{RunSecrets, SecretError};
use crate::settings::{Layers, Resolved, resolve};
use crate::tasks::TaskOwner;

/// A running Plugin instance tracked by [`RunPlugins`] so revocations can
/// stop it immediately.
struct RunningInstance {
    digest: Option<String>,
    signer: Option<SignerIdentity>,
    instance: PluginInstance,
}

/// One Run's process Plugins, started as its calls name them.
pub struct RunPlugins {
    store: PluginStore,
    index_store: PluginIndexStore,
    audit: Option<AuditLog>,
    cache: CacheLocation,
    instances: PathBuf,
    places: Option<Arc<Places>>,
    product: Product,
    run_id: RunId,
    owner: Arc<TaskOwner>,
    /// What its Plugins' `net.fetch` waits on.
    clock: Clock,
    /// Shared cache locks held for each started Plugin's runtime until the
    /// Run drops.
    held_runtimes: tokio::sync::Mutex<Vec<HeldArtifact>>,
    /// Running Plugin instances, by name, tracked so revocations stop them
    /// immediately.
    running: tokio::sync::Mutex<BTreeMap<String, RunningInstance>>,
    /// The Extractors, by name, each set once its instance started: one
    /// whose start failed stays unset, so a Plugin installed after a failed
    /// start is found on the next call. The lock is held only to find a
    /// name's cell, never across a start.
    extractors: tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::OnceCell<ProcessExtractor>>>>,
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
    /// confined away from `places`, none of which starts without them,
    /// started as tasks of the Run's `owner`, their `net.fetch` waiting on
    /// `clock`.
    pub fn new(
        home: &Home,
        places: Option<Arc<Places>>,
        product: Product,
        run_id: RunId,
        (owner, clock): (Arc<TaskOwner>, Clock),
    ) -> Self {
        Self {
            store: PluginStore::new(home.plugins()),
            index_store: PluginIndexStore::new(home.plugin_index()),
            audit: None,
            cache: CacheLocation::new(home.root(), home.browsers_path().map(Path::to_path_buf)),
            instances: home.plugin_instances(),
            places,
            product,
            run_id,
            owner,
            clock,
            held_runtimes: tokio::sync::Mutex::new(Vec::new()),
            running: tokio::sync::Mutex::new(BTreeMap::new()),
            extractors: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// Attaches the engine's [`AuditLog`] so Plugin revocations enforced
    /// at start or on running instances are recorded in the Audit log.
    #[must_use]
    pub fn with_audit(mut self, audit: AuditLog) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Tracks a running [`PluginInstance`] under `name`, `digest` and
    /// `signer` so [`Self::enforce_revocations`] stops it at once if a
    /// revocation arrives.
    pub async fn track_instance(
        &self,
        name: String,
        digest: Option<String>,
        signer: Option<SignerIdentity>,
        instance: PluginInstance,
    ) {
        self.running.lock().await.insert(
            name,
            RunningInstance {
                digest,
                signer,
                instance,
            },
        );
    }

    /// Stops every currently running instance whose digest or signer
    /// matches a revocation in `index`, evicts it from the Run's cache,
    /// and records `plugin_revoked` in the Audit log. Returns the names of
    /// the stopped Plugins.
    pub async fn enforce_revocations(&self, index: &PluginIndex) -> Vec<String> {
        let mut to_stop = Vec::new();
        {
            let mut running = self.running.lock().await;
            let matching: Vec<String> = running
                .iter()
                .filter(|(_, record)| {
                    index
                        .revocation_for(record.digest.as_deref(), record.signer.as_ref())
                        .is_some()
                })
                .map(|(name, _)| name.clone())
                .collect();
            for name in matching {
                if let Some(record) = running.remove(&name)
                    && let Some(revocation) =
                        index.revocation_for(record.digest.as_deref(), record.signer.as_ref())
                {
                    to_stop.push((name, record, revocation.clone()));
                }
            }
        }
        let mut stopped = Vec::with_capacity(to_stop.len());
        let now = self.clock.timestamp();
        for (name, record, revocation) in to_stop {
            self.extractors.lock().await.remove(&name);
            let error = plugin_index::revoked_error(&name, record.digest.as_deref(), &revocation);
            record.instance.stop_with(error).await;
            if let Some(audit) = &self.audit {
                plugin_index::audit_revocation(
                    audit,
                    &name,
                    record.digest.as_deref(),
                    &revocation,
                    Some(self.run_id.clone()),
                    true,
                    now,
                )
                .await;
            }
            stopped.push(name);
        }
        stopped
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
    /// value; otherwise [`StartError::Refused`]: `plugin_not_allowed` when
    /// the Operator policy does not let the Plugin load,
    /// `plugin_permission_denied` for a permission it needs as it starts
    /// that the policy has not approved, `setting_invalid` naming
    /// the field for settings the Plugin's schema refuses,
    /// `confinement_unavailable` naming the environment and its fix where
    /// this host cannot confine the instance, and
    /// `extractor_unavailable` when no Plugin of the Extractor kind is
    /// installed under `name`, or its instance cannot be set up; why is
    /// logged at `warn`. A Plugin that cannot launch fails its first call
    /// instead, as its instance answers.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe. Calls naming one Plugin wait on one start; dropping
    /// the call that starts it leaves the Plugin unstarted, and the next
    /// call starts it, as after a failed start.
    pub async fn extractor(
        &self,
        name: &str,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessExtractor, StartError> {
        let started = Arc::clone(
            self.extractors
                .lock()
                .await
                .entry(name.to_owned())
                .or_default(),
        );
        started
            .get_or_try_init(|| self.start(name, settings, secrets, probe))
            .await
            .cloned()
    }

    /// Starts the instance of the installed Plugin `name`, as
    /// [`Self::extractor`] tells: the host's confinement is checked before
    /// any setting is read, so a host that cannot confine the Plugin
    /// resolves none of its secrets.
    async fn start(
        &self,
        name: &str,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessExtractor, StartError> {
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
        let defaults;
        let resolved = if let Some(settings) = settings {
            settings
        } else {
            defaults = resolve(&Layers::default(), self.product, self.product)
                .expect("invariant: the built-in defaults resolve");
            &defaults
        };
        let origin = self.check_index(name, installed.digest.as_deref()).await?;
        let entry = admitted(origin, resolved, name, &installed)?;
        let probe = probe.clone();
        if let Some(refusal) = self.owner.blocking(move || probe.plugin_refusal()).await {
            tracing::warn!(extractor = name, %refusal, "this host cannot confine an Extractor's Plugin");
            return Err(StartError::Refused(refusal));
        }
        let (given, layer) = plugin_settings::given(settings, name);
        let environment = PluginSettings::checked(name, &installed.manifest, given, layer)
            .map_err(|invalid| StartError::Refused(invalid.into()))?
            .environment(
                name,
                (&installed.manifest, &entry.approved),
                secrets,
                (&self.run_id, layer),
            )
            .await
            .map_err(|error| match error {
                EnvironmentError::Secret(error) => StartError::Secret(error),
                EnvironmentError::Denied(permission) => {
                    StartError::Refused(permission_denied(name, permission))
                }
                ungranted @ EnvironmentError::Ungranted { .. } => cannot_start(&ungranted),
            })?;
        let fetch = PluginFetch::of(resolved, name, self.clock, self.owner.blocking_threads())
            .map_err(|invalid| cannot_start(&invalid))?
            .inserting(Inserting::new(
                name,
                (&installed.manifest, &entry.approved),
                resolved,
                secrets.clone(),
                self.run_id.clone(),
            ));
        let resolved_runtime = self
            .resolve_runtime(name, &installed.manifest.runtime, resolved)
            .await?;
        let folder = self.instances.join(format!("{}-{name}", self.run_id));
        let digest = installed.digest.clone();
        let places = self.places.clone();
        let plugin = self
            .owner
            .blocking(move || {
                set_up(
                    installed,
                    environment,
                    resolved_runtime,
                    &folder,
                    places.as_deref(),
                )
            })
            .await
            .map_err(|reason| cannot_start(&reason))?;
        let instance = PluginInstance::start(
            &self.owner,
            instance::Plugin {
                launcher: plugin.launcher,
                launch: plugin.launch,
                manifest: plugin.manifest,
                product: self.product,
                fetch: Arc::new(fetch),
                clock: self.clock,
            },
            Scope::Run(self.run_id.clone()),
        );
        self.track_instance(name.to_owned(), digest, None, instance.clone())
            .await;
        Ok(ProcessExtractor::new(name.to_owned(), instance))
    }

    /// Checks the cached Plugin index for revocations and expiry, auditing
    /// any revocation refusal, and returns the Plugin's [`Origin`].
    async fn check_index(&self, name: &str, digest: Option<&str>) -> Result<Origin, StartError> {
        let index_store = self.index_store.clone();
        let now = self.clock.timestamp();
        let status = self
            .owner
            .blocking(move || index_store.status(now))
            .await
            .map_err(|error| {
                tracing::warn!(extractor = name, %error, "the Plugin index could not be read");
                StartError::Refused(unavailable(name))
            })?;
        match plugin_index::check_load(&status, name, digest, None) {
            Ok(origin) => Ok(origin),
            Err(LoadRefusal::Revoked { revocation, error }) => {
                if let Some(audit) = &self.audit {
                    plugin_index::audit_revocation(
                        audit,
                        name,
                        digest,
                        &revocation,
                        Some(self.run_id.clone()),
                        false,
                        now,
                    )
                    .await;
                }
                Err(StartError::Refused(*error))
            }
            Err(LoadRefusal::ExpiredIndex(error)) => Err(StartError::Refused(*error)),
        }
    }

    /// Resolves and locks a Plugin's declared language runtime from the shared
    /// cache, refusing with `artifact_unavailable_offline` under
    /// `egress.offline` when absent without attempting a download.
    async fn resolve_runtime(
        &self,
        name: &str,
        runtime: &Runtime,
        resolved: &Resolved,
    ) -> Result<Option<(PathBuf, PathBuf)>, StartError> {
        if *runtime == Runtime::None {
            return Ok(None);
        }
        let offline = resolved
            .effective(&EGRESS_OFFLINE)
            .filter(|effective| effective.value == Value::Bool(true))
            .map(|effective| effective.layer);
        let cache = BrowserCache::new(&self.cache, self.owner.blocking_threads());
        let catalog = RuntimeCatalog::built_in();
        let resolved_rt = plugin_runtime::resolve_for_start(
            runtime,
            &self.store,
            &cache,
            &catalog,
            offline,
            &self.owner.blocking_threads(),
            self.clock.timestamp(),
        )
        .await
        .map_err(StartError::Refused)?;
        if let Some((held, executable)) = resolved_rt {
            let files = held.files();
            self.held_runtimes.lock().await.push(held);
            Ok(Some((files, executable)))
        } else {
            tracing::warn!(
                extractor = name,
                "a Plugin's declared language runtime is not in the runtime cache; run `plugin add` first",
            );
            Err(StartError::Refused(unavailable(name)))
        }
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

/// The Operator policy's entry for the Plugin `name` in `resolved`, a Run's
/// settings, once it lets `installed` load: a Plugin from the store counts
/// as third-party until its origin is recorded (issue 911), so it loads
/// only when the entry allows it, at the digest the entry pins. An
/// Extractor's network level is always `none`, so it never needs `direct`.
///
/// # Errors
///
/// `plugin_not_allowed` naming why.
fn admitted(
    origin: Origin,
    resolved: &Resolved,
    name: &str,
    installed: &Installed,
) -> Result<PluginEntry, StartError> {
    let entry = match &resolved.value(&PLUGINS)[name] {
        Value::Null => PluginEntry::default(),
        entry => PluginEntry::read(entry).map_err(|reason| {
            tracing::warn!(extractor = name, %reason, "an Extractor's Plugin entry does not read");
            StartError::Refused(unavailable(name))
        })?,
    };
    let found = installed.digest.as_deref();
    admit(origin, entry.allow, entry.digest.as_deref(), found).map_err(|refusal| {
        let (pinned, found, field) = match &refusal {
            PluginRefusal::Digest { pinned } => {
                (Some(pinned.clone()), found.map(str::to_owned), "digest")
            }
            PluginRefusal::NotAllowed | PluginRefusal::Denied => (None, None, "allow"),
        };
        let details = PluginNotAllowedDetails {
            plugin: name.to_owned(),
            reason: refusal.reason(),
            pinned,
            found,
            fix: fix(name, field),
        };
        StartError::Refused(CallError::new(
            &PLUGIN_NOT_ALLOWED,
            format!("the Plugin `{name}` cannot load: {refusal}"),
            &details,
        ))
    })?;
    Ok(entry)
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
    resolved_runtime: Option<(PathBuf, PathBuf)>,
    folder: &Path,
    places: Option<&Places>,
) -> Result<SetUp, String> {
    let places =
        places.ok_or("the engine's environment names no home folder to confine it from")?;
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let canonical = |path: &Path| std::fs::canonicalize(path).map_err(|error| error.to_string());
    let bundle = canonical(&installed.bundle)?;
    let folder = canonical(folder)?;
    let (runtime_folder, program, args) = match resolved_runtime {
        Some((files, executable)) => {
            let runtime_dir = canonical(&files)?;
            let entrypoint = bundle.join(&installed.manifest.entrypoint);
            (
                Some(runtime_dir),
                canonical(&executable)?,
                vec![entrypoint.to_string_lossy().into_owned()],
            )
        }
        None => (
            None,
            bundle.join(&installed.manifest.entrypoint),
            Vec::new(),
        ),
    };
    let instance = Instance {
        bundle,
        runtime: runtime_folder,
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
            program,
            args,
            folder,
        },
        manifest: installed.manifest,
    })
}

#[cfg(test)]
mod tests;
