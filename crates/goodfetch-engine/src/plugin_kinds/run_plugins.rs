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
//!
//! Size: one concern — one Run's lazy process-Plugin instance registry,
//! shared startup/admission/confinement path and immediate revocation stop
//! across Extractor, Secret provider and Hook kinds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_confinement::deny_list::Places;
use goodfetch_confinement::launch::Launch;
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{NOT_IN_PRODUCT, NotInProductDetails, ProductVersion};
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::plugin::approvals::{
    Origin, PLUGIN_DIRECT_STARTED, PLUGIN_NOT_ALLOWED, PLUGIN_REDUCED_CONFINEMENT, Permission,
    PluginNotAllowedDetails, PluginRefusal, admit, fix,
};
use goodfetch_protocol::plugin::index::{PluginIndex, SignerIdentity};
use goodfetch_protocol::plugin::settings::{PLUGINS, PluginEntry};
use goodfetch_protocol::plugin::{
    HookStageName, NetworkLevel, PluginKindName, PluginManifest, Runtime,
};
use goodfetch_protocol::run::{PluginConfinement, PluginForm, PluginRun, RunId};
use serde_json::{Value, json};

use super::extractor::{ProcessExtractor, unavailable};
use super::hook::{ProcessHook, hook_failed};
use super::notification::{ProcessNotificationChannel, unavailable as notification_unavailable};
use super::secret::ProcessSecretProvider;
use crate::audit::{AuditLog, Origin as AuditOrigin};
use crate::browser_cache::{BrowserCache, CacheLocation, HeldArtifact};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::home::Home;
use crate::plugin_gate::{
    EffectsGate, Inserting, PluginEffectJournal, PluginFetch, StreamAccountant, permission_denied,
};
use crate::plugin_host::instance::{
    self, InstanceHooks, PluginInstance, QuarantineNotifier, Scope,
};
use crate::plugin_host::settings::{self as plugin_settings, EnvironmentError, PluginSettings};
use crate::plugin_host::spend::{PluginSpend, SpendAccountant};
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

/// Persists each started Plugin into the Run's state so `RunReport::plugins`
/// lists it when the Run ends (ADRs 0016 and 0033).
pub trait PluginRunRecorder: Send + Sync {
    /// Records `plugin` in the Run's state.
    fn record(&self, plugin: PluginRun) -> BoxFuture<'_, ()>;
}

/// One Run's process Plugins, started as its calls name them.
pub struct RunPlugins {
    store: PluginStore,
    index_store: PluginIndexStore,
    audit: Option<AuditLog>,
    run_recorder: Option<Arc<dyn PluginRunRecorder>>,
    quarantine_notifier: Option<Arc<dyn QuarantineNotifier>>,
    spend: Option<Arc<dyn SpendAccountant>>,
    streams: Option<Arc<dyn StreamAccountant>>,
    effects_journal: Option<Arc<dyn PluginEffectJournal>>,
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
    /// The Secret providers, by declared scheme, each set once its instance
    /// started.
    secret_providers:
        tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::OnceCell<ProcessSecretProvider>>>>,
    /// The process-Plugin Hooks, by Plugin name, each set once its instance
    /// started.
    hooks: tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::OnceCell<ProcessHook>>>>,
    /// The Notification channels, by Plugin name, each set once its instance
    /// started.
    notification_channels: tokio::sync::Mutex<
        BTreeMap<String, Arc<tokio::sync::OnceCell<ProcessNotificationChannel>>>,
    >,
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
            run_recorder: None,
            quarantine_notifier: None,
            spend: None,
            streams: None,
            effects_journal: None,
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
            secret_providers: tokio::sync::Mutex::new(BTreeMap::new()),
            hooks: tokio::sync::Mutex::new(BTreeMap::new()),
            notification_channels: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// Attaches the engine's [`AuditLog`] so Plugin revocations enforced
    /// at start or on running instances are recorded in the Audit log.
    #[must_use]
    pub fn with_audit(mut self, audit: AuditLog) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Attaches a [`PluginRunRecorder`] so every started Plugin is recorded in
    /// the run file and listed in `RunReport::plugins` (ADRs 0016 and 0033).
    #[must_use]
    pub fn with_run_recorder(mut self, recorder: Arc<dyn PluginRunRecorder>) -> Self {
        self.run_recorder = Some(recorder);
        self
    }

    /// Attaches a [`QuarantineNotifier`] so quarantining a Plugin sends one
    /// "a Plugin quarantined" Notification per Run and Plugin (ADRs 0027 and
    /// 0033, issue 837).
    #[must_use]
    pub fn with_quarantine_notifier(mut self, notifier: Arc<dyn QuarantineNotifier>) -> Self {
        self.quarantine_notifier = Some(notifier);
        self
    }

    /// Attaches the Run's spend accountant so started Plugin instances
    /// account for declared spend against currency Limits (ADR 0033).
    #[must_use]
    pub fn with_spend(mut self, spend: Arc<dyn SpendAccountant>) -> Self {
        self.spend = Some(spend);
        self
    }

    /// Attaches the Run's stream accountant so `net.connect` and `proxy_tcp`
    /// streams charge `plugin_stream_bytes`/`plugin_stream_time` and write to
    /// the Run trace (ADR 0033, issue 833).
    #[must_use]
    pub fn with_streams(mut self, streams: Arc<dyn StreamAccountant>) -> Self {
        self.streams = Some(streams);
        self
    }

    /// Attaches the Run's Effect journal writer so Plugin Effects toward the
    /// run's sites are authorized, audited and journaled (ADR 0033, issue 845).
    #[must_use]
    pub fn with_effects_journal(mut self, journal: Arc<dyn PluginEffectJournal>) -> Self {
        self.effects_journal = Some(journal);
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

    /// Whether this live Run is currently using the Plugin `name` (either a
    /// running instance is tracked for it or an Extractor cell has been
    /// initialized for it, ADR 0033, issue 928).
    pub async fn is_using(&self, name: &str) -> bool {
        if self.running.lock().await.contains_key(name) {
            return true;
        }
        if self
            .extractors
            .lock()
            .await
            .get(name)
            .is_some_and(|cell| cell.initialized())
        {
            return true;
        }
        if self
            .hooks
            .lock()
            .await
            .get(name)
            .is_some_and(|cell| cell.initialized())
        {
            return true;
        }
        self.notification_channels
            .lock()
            .await
            .get(name)
            .is_some_and(|cell| cell.initialized())
    }

    /// Checks whether asking for Plugin `name` of kind `kind` is supported by
    /// this Run's Product (ADRs 0094 and 0098; issue 928). Refuses with
    /// `not_in_product` when `kind` is not carried by `self.product` (such as
    /// `sink` or `fetch_backend` on Collie) or when `name` is installed in the
    /// shared store with only kinds outside `self.product`.
    ///
    /// # Errors
    ///
    /// `not_in_product` when the requested kind or installed Plugin belongs to
    /// Foxhound and this Run runs on Collie.
    pub async fn check_product_kind(
        &self,
        name: &str,
        kind: &PluginKindName,
        product_version: &ProductVersion,
    ) -> Result<(), CallError> {
        if !kind.in_product(self.product) {
            return Err(plugin_not_in_product(
                self.product,
                product_version,
                name,
                std::slice::from_ref(kind),
            ));
        }
        let store = self.store.clone();
        let named = name.to_owned();
        if let Ok(installed) = self.owner.blocking(move || store.installed(&named)).await
            && !installed
                .manifest
                .kinds
                .iter()
                .any(|declared| declared.in_product(self.product))
        {
            let kinds: Vec<PluginKindName> = installed.manifest.kinds.into_iter().collect();
            return Err(plugin_not_in_product(
                self.product,
                product_version,
                name,
                &kinds,
            ));
        }
        Ok(())
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
            self.hooks.lock().await.remove(&name);
            self.notification_channels.lock().await.remove(&name);
            self.secret_providers.lock().await.retain(|_, cell| {
                cell.get()
                    .is_none_or(|provider| provider.name() != name.as_str())
            });
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

    /// Resolves `name` as an Extractor from the store, translating a
    /// Foxhound-only Plugin (such as a Sink or Fetch backend on Collie) into
    /// `not_in_product` instead of `extractor_unavailable`.
    async fn resolve_extractor(&self, name: &str) -> Result<Installed, StartError> {
        let store = self.store.clone();
        let named = name.to_owned();
        let product = self.product;
        match self
            .owner
            .blocking(
                move || match store.resolve(&named, &PluginKindName::EXTRACT) {
                    Ok(installed) => Ok(installed),
                    Err(StoreError::Kind { .. }) => match store.installed(&named) {
                        Ok(other)
                            if !other
                                .manifest
                                .kinds
                                .iter()
                                .any(|kind| kind.in_product(product)) =>
                        {
                            let kinds: Vec<PluginKindName> =
                                other.manifest.kinds.into_iter().collect();
                            Err(Err(kinds))
                        }
                        _ => Err(Ok(StoreError::Kind {
                            name: named,
                            kind: PluginKindName::EXTRACT,
                        })),
                    },
                    Err(error) => Err(Ok(error)),
                },
            )
            .await
        {
            Ok(installed) => Ok(installed),
            Err(Ok(error)) => {
                tracing::warn!(extractor = name, %error, "an Extractor's Plugin cannot start");
                Err(StartError::Refused(unavailable(name)))
            }
            Err(Err(kinds)) => {
                let version = ProductVersion::parse("0.1.0")
                    .expect("invariant: valid ProductVersion literal");
                Err(StartError::Refused(plugin_not_in_product(
                    self.product,
                    &version,
                    name,
                    &kinds,
                )))
            }
        }
    }

    /// The Secret provider of the installed Plugin claiming `scheme`, its
    /// instance started on the first call for that scheme.
    ///
    /// # Errors
    ///
    /// [`SecretStartError::NoScheme`] when no installed Secret provider declares
    /// `scheme`, [`SecretStartError::SchemeConflict`] when more than one does,
    /// or [`SecretStartError::Start`] for any policy, confinement, settings,
    /// secret or startup refusal.
    pub async fn secret_provider(
        &self,
        scheme: &str,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessSecretProvider, SecretStartError> {
        let started = Arc::clone(
            self.secret_providers
                .lock()
                .await
                .entry(scheme.to_owned())
                .or_default(),
        );
        started
            .get_or_try_init(|| self.start_secret_provider(scheme, settings, secrets, probe))
            .await
            .cloned()
    }

    /// Starts the Secret provider instance for `scheme`.
    async fn start_secret_provider(
        &self,
        scheme: &str,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessSecretProvider, SecretStartError> {
        let store = self.store.clone();
        let wanted = scheme.to_owned();
        let installed = self
            .owner
            .blocking(move || store.resolve_secret_scheme(&wanted))
            .await
            .map_err(|error| match error {
                StoreError::NoScheme(scheme) => SecretStartError::NoScheme(scheme),
                StoreError::SchemeConflict { scheme, existing } => {
                    SecretStartError::SchemeConflict { scheme, existing }
                }
                other => {
                    tracing::warn!(scheme, %other, "a Secret provider Plugin cannot be resolved");
                    SecretStartError::Start(StartError::Refused(secret_plugin_unavailable(scheme)))
                }
            })?;
        let name = installed.manifest.name.clone();
        let (instance, _) = self
            .start_instance(
                installed,
                settings,
                secrets,
                probe,
                secret_plugin_unavailable,
            )
            .await
            .map_err(SecretStartError::Start)?;
        Ok(ProcessSecretProvider::new(name, instance))
    }

    /// The process-Plugin Hook named `plugin_name`, started lazily on first
    /// call for `stage` (ADRs 0033, 0088 and 0104; issue 954).
    ///
    /// # Errors
    ///
    /// [`StartError::Secret`] when resolving a secret setting fails;
    /// [`StartError::Refused`] when the Plugin is missing, does not declare
    /// the `hook` kind or `stage`, is denied by Operator policy, fails
    /// settings validation, or cannot be confined on this host.
    pub async fn hook(
        &self,
        plugin_name: &str,
        hook_name: &str,
        stage: &HookStageName,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessHook, StartError> {
        let started = Arc::clone(
            self.hooks
                .lock()
                .await
                .entry(plugin_name.to_owned())
                .or_default(),
        );
        let hook = started
            .get_or_try_init(|| {
                self.start_hook(plugin_name, hook_name, stage, settings, secrets, probe)
            })
            .await?
            .clone();
        if !hook.implements_stage(stage) {
            return Err(StartError::Refused(hook_failed(
                stage,
                hook_name,
                "HookStageUnsupported",
                &format!(
                    "the Plugin `{plugin_name}` does not declare Hook stage `{}` in its manifest",
                    stage.as_str()
                ),
                None,
            )));
        }
        Ok(hook)
    }

    async fn start_hook(
        &self,
        plugin_name: &str,
        hook_name: &str,
        stage: &HookStageName,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessHook, StartError> {
        let store = self.store.clone();
        let named = plugin_name.to_owned();
        let installed = self
            .owner
            .blocking(move || store.resolve(&named, &PluginKindName::HOOK))
            .await
            .map_err(|error| {
                tracing::warn!(plugin = plugin_name, hook = hook_name, %error, "a Hook Plugin cannot start");
                StartError::Refused(hook_failed(
                    stage,
                    hook_name,
                    "HookPluginUnavailable",
                    &format!("the Hook Plugin `{plugin_name}` is not installed or is not a Hook: {error}"),
                    None,
                ))
            })?;
        let stages = installed.manifest.hook_stages.clone();
        let stage_for_err = *stage;
        let hook_name_owned = hook_name.to_owned();
        let (instance, effects) = self
            .start_instance(installed, settings, secrets, probe, move |name| {
                hook_failed(
                    &stage_for_err,
                    &hook_name_owned,
                    "HookPluginUnavailable",
                    &format!("the Hook Plugin `{name}` could not start"),
                    None,
                )
            })
            .await?;
        Ok(ProcessHook::new(
            plugin_name.to_owned(),
            instance,
            effects,
            stages,
        ))
    }

    /// The Notification channel of the installed Plugin `plugin_name`, serving
    /// `notifications.channels.<channel_name>`, started lazily on first send
    /// (ADRs 0027 and 0033; issue 1023).
    ///
    /// # Errors
    ///
    /// [`StartError::Secret`] when resolving a secret setting fails;
    /// [`StartError::Refused`] when no Notification channel Plugin is
    /// installed under `plugin_name` (`notification_channel_unavailable`),
    /// Operator policy refuses it, settings fail validation, or the host
    /// cannot confine it.
    pub async fn notification_channel(
        &self,
        plugin_name: &str,
        channel_name: &str,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessNotificationChannel, StartError> {
        let started = Arc::clone(
            self.notification_channels
                .lock()
                .await
                .entry(plugin_name.to_owned())
                .or_default(),
        );
        started
            .get_or_try_init(|| {
                self.start_notification_channel(plugin_name, channel_name, settings, secrets, probe)
            })
            .await
            .cloned()
    }

    async fn start_notification_channel(
        &self,
        plugin_name: &str,
        channel_name: &str,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
    ) -> Result<ProcessNotificationChannel, StartError> {
        let store = self.store.clone();
        let named = plugin_name.to_owned();
        let installed = self
            .owner
            .blocking(move || store.resolve(&named, &PluginKindName::NOTIFICATION))
            .await
            .map_err(|error| {
                tracing::warn!(
                    plugin = plugin_name,
                    channel = channel_name,
                    %error,
                    "a Notification channel Plugin cannot start"
                );
                StartError::Refused(notification_unavailable(channel_name))
            })?;
        let channel_for_err = channel_name.to_owned();
        let (instance, _) = self
            .start_instance(installed, settings, secrets, probe, move |_| {
                notification_unavailable(&channel_for_err)
            })
            .await?;
        Ok(ProcessNotificationChannel::new(
            plugin_name.to_owned(),
            instance,
        ))
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
        let installed = self.resolve_extractor(name).await?;
        let (instance, _) = self
            .start_instance(installed, settings, secrets, probe, unavailable)
            .await?;
        Ok(ProcessExtractor::new(name.to_owned(), instance))
    }

    /// Shared instance startup path for any installed process Plugin: checks
    /// the index, admission policy, host confinement, settings/environment,
    /// `net.fetch` gate, language runtime, confinement setup and spend
    /// accounting, then launches and tracks the instance.
    async fn start_instance(
        &self,
        installed: Installed,
        settings: Option<&Resolved>,
        secrets: &RunSecrets,
        probe: &SandboxProbe,
        unavailable_error: impl Fn(&str) -> CallError,
    ) -> Result<(PluginInstance, Option<EffectsGate>), StartError> {
        let name = installed.manifest.name.clone();
        let cannot_start = |reason: &dyn std::fmt::Display| {
            tracing::warn!(plugin = %name, %reason, "a Plugin cannot start");
            StartError::Refused(unavailable_error(&name))
        };
        let defaults;
        let resolved = if let Some(settings) = settings {
            settings
        } else {
            defaults = resolve(&Layers::default(), self.product, self.product)
                .expect("invariant: the built-in defaults resolve");
            &defaults
        };
        let origin = self.check_index(&name, installed.digest.as_deref()).await?;
        let entry = admitted(origin, resolved, &name, &installed, &unavailable_error)?;
        let network_level = installed.manifest.network_level;
        check_direct_approval(&name, network_level, origin, &entry)?;
        let (confinement, confinement_missing, reduced_confinement) = self
            .check_confinement(&name, network_level, origin, &entry, probe)
            .await?;
        let (given, layer) = plugin_settings::given(settings, &name);
        let (fetch, effects, environment) = self
            .prepare_gate_and_environment(
                &name,
                (&installed.manifest, &entry.approved, origin),
                (given, layer, resolved),
                secrets,
                &cannot_start,
            )
            .await?;
        let resolved_runtime = self
            .resolve_runtime(
                &name,
                &installed.manifest.runtime,
                resolved,
                &unavailable_error,
            )
            .await?;
        let folder = self.instances.join(format!("{}-{name}", self.run_id));
        let digest = installed.digest.clone();
        let places = self.places.clone();
        let cache_root = self.cache.root().to_path_buf();
        let helper_launcher = self.sandbox_helper_for(probe, network_level).await;
        let plugin = self
            .owner
            .blocking(move || {
                set_up(
                    installed,
                    environment,
                    resolved_runtime,
                    &folder,
                    places.as_deref(),
                    &cache_root,
                    helper_launcher,
                    reduced_confinement,
                )
            })
            .await
            .map_err(|reason| cannot_start(&reason))?;
        let spend = self
            .spend
            .clone()
            .map(|accountant| PluginSpend::new(accountant).with_overrides(entry.spend));
        let instance = PluginInstance::start_with(
            &self.owner,
            instance::Plugin {
                launcher: plugin.launcher,
                launcher_env: plugin.launcher_env,
                launch: plugin.launch,
                manifest: plugin.manifest,
                product: self.product,
                fetch: Arc::new(fetch),
                clock: self.clock,
                spend,
            },
            Scope::Run(self.run_id.clone()),
            InstanceHooks {
                audit: self.audit.clone(),
                quarantine_notifier: self.quarantine_notifier.clone(),
                ..InstanceHooks::default()
            },
        );
        self.record_started(
            StartedPlugin {
                name,
                network_level,
                origin,
                digest,
                confinement,
                confinement_missing,
                layer,
            },
            instance.clone(),
        )
        .await;
        Ok((instance, effects))
    }

    /// Returns the configured `goodfetch-sandbox` helper path when this host
    /// needs it to confine a process Plugin at `network_level` (ADR 0033,
    /// issue 884).
    async fn sandbox_helper_for(
        &self,
        probe: &SandboxProbe,
        network_level: NetworkLevel,
    ) -> Option<PathBuf> {
        let probe = probe.clone();
        self.owner
            .blocking(move || {
                probe
                    .needs_sandbox_helper(network_level)
                    .then(|| probe.sandbox_helper_path().to_path_buf())
            })
            .await
    }

    /// Checks host confinement for `network_level`, permitting reduced
    /// confinement only for a first-party Plugin whose Operator policy sets
    /// `plugins.<name>.reduced_confinement = true` (ADR 0033).
    async fn check_confinement(
        &self,
        name: &str,
        network_level: NetworkLevel,
        origin: Origin,
        entry: &PluginEntry,
        probe: &SandboxProbe,
    ) -> Result<(PluginConfinement, Vec<String>, bool), StartError> {
        let probe = probe.clone();
        match self
            .owner
            .blocking(move || probe.plugin_confinement(network_level))
            .await
        {
            Ok(()) => Ok((PluginConfinement::Full, Vec::new(), false)),
            Err((missing, refusal)) => {
                if origin == Origin::FirstParty && entry.reduced_confinement {
                    Ok((PluginConfinement::Reduced, missing, true))
                } else {
                    tracing::warn!(plugin = %name, %refusal, "this host cannot confine a Plugin");
                    Err(StartError::Refused(refusal))
                }
            }
        }
    }

    /// Audits a `direct` or reduced-confinement Plugin start, records the
    /// started Plugin for the Run report, and tracks the running instance for
    /// revocation enforcement.
    async fn record_started(&self, started: StartedPlugin, instance: PluginInstance) {
        if started.network_level == NetworkLevel::Direct {
            self.audit_direct_start(
                &started.name,
                started.origin,
                started.digest.as_deref(),
                started.layer,
            )
            .await;
        }
        if started.confinement == PluginConfinement::Reduced {
            self.audit_reduced_confinement(
                &started.name,
                started.network_level,
                started.digest.as_deref(),
                &started.confinement_missing,
                started.layer,
            )
            .await;
        }
        if let Some(recorder) = &self.run_recorder {
            recorder
                .record(PluginRun {
                    name: started.name.clone(),
                    form: PluginForm::Process,
                    origin: Some(started.origin),
                    digest: started.digest.clone(),
                    network_level: started.network_level.as_str().to_owned(),
                    confinement: started.confinement,
                    confinement_missing: started.confinement_missing,
                })
                .await;
        }
        self.track_instance(started.name, started.digest, None, instance)
            .await;
    }

    /// Records an SDK handler used by this Run so `RunReport::plugins` lists
    /// it with form `sdk_handler` and `confinement: unconfined` (ADRs 0016,
    /// 0033 and 0104).
    pub async fn record_sdk_handler(&self, name: &str) {
        if let Some(recorder) = &self.run_recorder {
            recorder
                .record(PluginRun {
                    name: name.to_owned(),
                    form: PluginForm::SdkHandler,
                    origin: None,
                    digest: None,
                    network_level: "unconfined".to_owned(),
                    confinement: PluginConfinement::Unconfined,
                    confinement_missing: Vec::new(),
                })
                .await;
        }
    }

    /// Validates the Plugin's settings, builds its [`PluginFetch`] gate,
    /// verifies configured destinations under ADR 0017 when `direct`, and
    /// resolves its environment variables (including Secrets).
    async fn prepare_gate_and_environment(
        &self,
        name: &str,
        (manifest, approved, origin): (
            &PluginManifest,
            &goodfetch_protocol::plugin::approvals::Approvals,
            Origin,
        ),
        (given, layer, resolved): (
            serde_json::Map<String, Value>,
            Option<goodfetch_protocol::settings::SettingsLayer>,
            &Resolved,
        ),
        secrets: &RunSecrets,
        cannot_start: &impl Fn(&dyn std::fmt::Display) -> StartError,
    ) -> Result<(PluginFetch, Option<EffectsGate>, BTreeMap<String, String>), StartError> {
        let checked_settings = PluginSettings::checked(name, manifest, given, layer)
            .map_err(|invalid| StartError::Refused(invalid.into()))?;
        let (fetch, effects) = self
            .build_fetch(name, manifest, approved, resolved, secrets)
            .map_err(|invalid| cannot_start(&invalid))?;
        if manifest.network_level == NetworkLevel::Direct {
            fetch
                .check_direct_destinations(
                    checked_settings.values(),
                    checked_settings.secret_paths(),
                    origin,
                )
                .await
                .map_err(StartError::Refused)?;
        }
        let environment = checked_settings
            .environment(
                name,
                (manifest, approved, origin),
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
        Ok((fetch, effects, environment))
    }

    /// Records `plugin_direct_started` in the Audit log when a Plugin starts
    /// at network level `direct` (ADR 0033, issue 855).
    async fn audit_direct_start(
        &self,
        name: &str,
        origin: Origin,
        digest: Option<&str>,
        layer: Option<goodfetch_protocol::settings::SettingsLayer>,
    ) {
        let Some(audit) = &self.audit else {
            return;
        };
        let mut details = json!({
            "plugin": name,
            "origin": origin,
            "network_level": NetworkLevel::Direct.as_str(),
        });
        if let Some(digest) = digest {
            details["digest"] = json!(digest);
        }
        let audit_origin = AuditOrigin {
            actor: Actor::Engine,
            surface: None,
            layer,
            run_id: Some(self.run_id.clone()),
        };
        let _ = audit
            .record(
                &audit_origin,
                &PLUGIN_DIRECT_STARTED,
                details,
                self.clock.timestamp(),
            )
            .await;
    }

    /// Records `plugin_reduced_confinement` in the Audit log when a first-party
    /// Plugin starts under Operator-approved reduced confinement (ADR 0033,
    /// issue 948).
    async fn audit_reduced_confinement(
        &self,
        name: &str,
        network_level: NetworkLevel,
        digest: Option<&str>,
        confinement_missing: &[String],
        layer: Option<goodfetch_protocol::settings::SettingsLayer>,
    ) {
        let Some(audit) = &self.audit else {
            return;
        };
        let mut details = json!({
            "plugin": name,
            "origin": Origin::FirstParty,
            "network_level": network_level.as_str(),
            "confinement_missing": confinement_missing,
        });
        if let Some(digest) = digest {
            details["digest"] = json!(digest);
        }
        let audit_origin = AuditOrigin {
            actor: Actor::Engine,
            surface: None,
            layer,
            run_id: Some(self.run_id.clone()),
        };
        let _ = audit
            .record(
                &audit_origin,
                &PLUGIN_REDUCED_CONFINEMENT,
                details,
                self.clock.timestamp(),
            )
            .await;
    }

    /// Builds a Plugin's [`PluginFetch`] wired with Secret insertion and the
    /// Run's [`EffectsGate`] (when an Effect journal is attached).
    fn build_fetch(
        &self,
        name: &str,
        manifest: &PluginManifest,
        approved: &goodfetch_protocol::plugin::approvals::Approvals,
        resolved: &Resolved,
        secrets: &RunSecrets,
    ) -> Result<(PluginFetch, Option<EffectsGate>), crate::egress::InvalidEntry> {
        let mut fetch = PluginFetch::of(resolved, name, self.clock, self.owner.blocking_threads())?
            .inserting(Inserting::new(
                name,
                (manifest, approved),
                resolved,
                secrets.clone(),
                self.run_id.clone(),
            ));
        let mut effects = None;
        if let Some(journal) = &self.effects_journal {
            let page_policy = fetch.page_policy().clone();
            let gate = EffectsGate::new(
                name.to_owned(),
                manifest.clone(),
                approved.clone(),
                page_policy,
                self.audit.clone(),
                self.run_id.clone(),
                self.clock,
                Arc::clone(journal),
            );
            fetch = fetch.with_effects(gate.clone());
            effects = Some(gate);
        }
        if let Some(streams) = &self.streams {
            fetch = fetch.with_streams(Arc::clone(streams));
        }
        Ok((fetch, effects))
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
        unavailable_error: &impl Fn(&str) -> CallError,
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
                plugin = name,
                "a Plugin's declared language runtime is not in the runtime cache; run `plugin add` first",
            );
            Err(StartError::Refused(unavailable_error(name)))
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

/// Why a Plugin's instance does not start.
#[derive(Debug)]
pub enum StartError {
    /// The call's failure: its settings refused, or the Plugin unavailable.
    Refused(CallError),
    /// A secret field's reference gave no value, which the caller answers
    /// as any Secret's failure.
    Secret(SecretError),
}

/// Why a Secret provider Plugin's instance does not start.
#[derive(Debug)]
pub enum SecretStartError {
    /// No installed Secret provider Plugin declares the requested scheme.
    NoScheme(String),
    /// More than one installed Secret provider Plugin claims the scheme.
    SchemeConflict {
        /// The conflicting scheme.
        scheme: String,
        /// The installed Plugins claiming it.
        existing: String,
    },
    /// Any other start refusal or secret-resolution failure.
    Start(StartError),
}

/// Generic unavailable error for a Secret provider Plugin that could not
/// start (translated to `secret_unavailable` by the Secret provider driver).
fn secret_plugin_unavailable(name: &str) -> CallError {
    CallError::new(
        &goodfetch_protocol::secrets::SECRET_UNAVAILABLE,
        format!(
            "The Secret provider Plugin `{name}` could not start; why is in the log at `warn`."
        ),
        &(),
    )
}

/// The Operator policy's entry for the Plugin `name` in `resolved`, a Run's
/// settings, once it lets `installed` load: a Plugin from the store counts
/// as third-party until its origin is recorded (issue 911), so it loads
/// only when the entry allows it, at the digest the entry pins.
///
/// # Errors
///
/// `plugin_not_allowed` naming why.
fn admitted(
    origin: Origin,
    resolved: &Resolved,
    name: &str,
    installed: &Installed,
    unavailable_error: &impl Fn(&str) -> CallError,
) -> Result<PluginEntry, StartError> {
    let entry = match &resolved.value(&PLUGINS)[name] {
        Value::Null => PluginEntry::default(),
        entry => PluginEntry::read(entry).map_err(|reason| {
            tracing::warn!(plugin = name, %reason, "a Plugin's entry does not read");
            StartError::Refused(unavailable_error(name))
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

/// Refuses a non-first-party `direct` Plugin whose Operator policy does not
/// explicitly approve `Permission::Direct` (ADR 0033, issue 855).
fn check_direct_approval(
    name: &str,
    network_level: NetworkLevel,
    origin: Origin,
    entry: &PluginEntry,
) -> Result<(), StartError> {
    if network_level == NetworkLevel::Direct
        && origin != Origin::FirstParty
        && !entry.approved.covers(&Permission::Direct)
    {
        return Err(StartError::Refused(permission_denied(
            name,
            Permission::Direct,
        )));
    }
    Ok(())
}

/// Facts about a process Plugin instance that just started, recorded into the
/// Audit log and `run_plugins`.
struct StartedPlugin {
    name: String,
    network_level: NetworkLevel,
    origin: Origin,
    digest: Option<String>,
    confinement: PluginConfinement,
    confinement_missing: Vec<String>,
    layer: Option<goodfetch_protocol::settings::SettingsLayer>,
}

/// What starts one instance, set up on the blocking policy.
struct SetUp {
    launcher: PathBuf,
    launcher_env: Vec<(String, String)>,
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
#[expect(
    clippy::too_many_arguments,
    reason = "internal blocking setup helper assembling a Plugin launch from resolved values"
)]
fn set_up(
    installed: Installed,
    environment: BTreeMap<String, String>,
    resolved_runtime: Option<(PathBuf, PathBuf)>,
    folder: &Path,
    places: Option<&Places>,
    cache_root: &Path,
    helper_launcher: Option<PathBuf>,
    reduced_confinement: bool,
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
    let launcher_env = if helper_launcher.is_some() {
        vec![
            (
                goodfetch_protocol::settings::HOME_VARIABLE.to_owned(),
                places.goodfetch_home.to_string_lossy().into_owned(),
            ),
            (
                goodfetch_protocol::settings::BROWSERS_PATH_VARIABLE.to_owned(),
                cache_root.to_string_lossy().into_owned(),
            ),
        ]
    } else {
        Vec::new()
    };
    let launcher = match helper_launcher {
        Some(helper) => helper,
        None => std::env::current_exe().map_err(|error| error.to_string())?,
    };
    Ok(SetUp {
        launcher,
        launcher_env,
        launch: Launch {
            policy,
            program,
            args,
            folder,
            proxy_relay: None,
            reduced_confinement,
        },
        manifest: installed.manifest,
    })
}

/// Checks whether an installed Plugin `name` in `store_folder` declares only
/// kinds that `product` lacks (such as `sink` or `fetch_backend` on Collie),
/// returning its unsupported kinds when so (ADRs 0094 and 0098; issue 928).
#[must_use]
pub fn unsupported_only_kinds(
    store_folder: PathBuf,
    name: &str,
    product: Product,
) -> Option<Vec<PluginKindName>> {
    let store = PluginStore::new(store_folder);
    let installed = store.installed(name).ok()?;
    if installed
        .manifest
        .kinds
        .iter()
        .any(|kind| kind.in_product(product))
    {
        return None;
    }
    Some(installed.manifest.kinds.into_iter().collect())
}

/// Constructs a `not_in_product` [`CallError`] when a Collie Run asks for a
/// Foxhound-only Plugin or Plugin kind (ADRs 0094 and 0098; issue 928).
#[must_use]
pub fn plugin_not_in_product(
    product: Product,
    product_version: &ProductVersion,
    name: &str,
    kinds: &[PluginKindName],
) -> CallError {
    let listed = kinds
        .iter()
        .map(PluginKindName::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let requested = format!("plugin `{name}` (kind: {listed})");
    let details = NotInProductDetails {
        product,
        product_version: product_version.clone(),
        needs: Product::Foxhound,
        requested: Some(requested.clone()),
        fix: format!(
            "Run with Foxhound instead of Collie to use {requested}; Collie keeps the Plugin in the shared store untouched."
        ),
        install: None,
        handover: false,
    };
    CallError::new(
        &NOT_IN_PRODUCT,
        format!("Collie does not carry {requested}: Foxhound does."),
        &details,
    )
}

#[cfg(test)]
mod tests;
