//! `plugin.add` (ADR 0033, issue 889): a person's answer (ADR 0099) that
//! installs a Plugin from the OCI reference it is given and pins the digest
//! of its image index in the Plugin lockfile, `goodfetch-plugins.lock`,
//! beside the project's configuration or in the Goodfetch home's
//! configuration folder.
//!
//! The lockfile is read before anything is pulled: a name it pins to
//! another digest is refused with `plugin_digest_mismatch`, a Plugin the
//! store holds that it does not pin with `plugin_installed`, and a Plugin
//! it pins to the same digest that the store holds is answered again
//! without a pull. The lockfile is written only once the Plugin is in the
//! store.
//!
//! The answer states each permission the Plugin's manifest declares. With
//! `approve` (issue 900), given once the person saw them, the Operator
//! policy's `plugins.<name>` entry allows the Plugin, pins it to the
//! digest and approves each permission, recorded in the Audit log as
//! `plugin_approved`; a policy managed elsewhere is left as it is and the
//! answer carries the entry to add there.
#![expect(
    clippy::disallowed_methods,
    reason = "the lockfile is read and written on the engine's blocking policy, never on an async worker"
)]

use std::io;
use std::path::Path;

use goodfetch_protocol::browser_cache::{
    ARTIFACT_DOWNLOAD_FAILED, ARTIFACT_UNAVAILABLE_OFFLINE, ArtifactDownloadFailedDetails,
    ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::plugin::add::{
    AttestationFailureReason, PLUGIN_ADD, PLUGIN_ARTIFACT_REFUSED, PLUGIN_DIGEST_MISMATCH,
    PLUGIN_INSTALLED, PluginAddParams, PluginAdded, PluginApproval, PluginDigestMismatchDetails,
};
use goodfetch_protocol::plugin::approvals::{
    Approvals, Origin, PLUGIN_APPROVED, Permission, stated,
};
use goodfetch_protocol::plugin::index::{PluginIndex, SignerIdentity};
use goodfetch_protocol::plugin::lockfile::{LOCKFILE_NAME, LockedPlugin, PluginLockfile};
use goodfetch_protocol::plugin::settings::{PLUGINS, PluginEntry};
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use serde_json::{Map, Value, json};

use crate::browser_cache::{BrowserCache, CacheLocation};
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::config;
use crate::durable;
use crate::pinned_download::Fetching;
use crate::plugin_index::PluginIndexStore;
use crate::plugin_install::{self, AttestationFetchError, InstallError, PullFailure};
use crate::plugin_lockfile;
use crate::plugin_oci::Reference;
use crate::plugin_provenance::{self, AttestationTrustRoot, AttestationVerdict, ResolveError};
use crate::plugin_runtime::{self, RuntimeCatalog};
use crate::plugin_store::{Installed, PluginStore, StoreError};
use crate::run::settings_error;
use crate::run_files::{os_code, reason_of};
use crate::settings::{Layers, Opening, Policy, Resolved, Values, resolve, rewrite};
use crate::settings_editor::{self, Change, SettingsEditor};
use crate::tasks::BlockingThreads;

/// Provenance and index inputs used by [`add_checked`] to enforce look-alike
/// name protection and provenance attestation verification (ADR 0033,
/// issue 916).
pub struct ProvenancePolicy<'a> {
    /// The cached Plugin index, when available.
    pub index: Option<&'a PluginIndex>,
    /// Resolved settings holding any Operator policy exact-digest allowance.
    pub settings: Option<&'a Resolved>,
    /// Trust root for verifying provenance attestation signatures.
    pub trust: &'a AttestationTrustRoot,
}

/// The `plugin.*` handlers.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&PLUGIN_ADD, add_operation)]
}

/// Dependencies opened for a `plugin.add` or `plugin.update` RPC call.
pub(crate) struct InstallContext {
    pub(crate) lockfile: std::path::PathBuf,
    pub(crate) resolved: Resolved,
    pub(crate) fetching: Fetching,
    pub(crate) blocking: BlockingThreads,
    pub(crate) store: PluginStore,
    pub(crate) cache: BrowserCache,
    pub(crate) catalog: RuntimeCatalog,
    pub(crate) now: jiff::Timestamp,
    pub(crate) cached_index: Option<PluginIndex>,
}

/// Opens the lockfile path, settings, HTTP fetcher, store, runtime cache and
/// cached Plugin index for a `plugin.add` or `plugin.update` call.
pub(crate) async fn open_install_context(
    project: Option<goodfetch_protocol::settings::ProjectSettings>,
    context: &CallContext,
) -> Result<InstallContext, CallError> {
    let engine = context.engine();
    let beside = match &project {
        Some(project) => Path::new(&project.path),
        None => engine.settings().files().user_configuration(),
    };
    let lockfile = beside
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LOCKFILE_NAME);
    let (settings, _) = engine
        .settings()
        .open(Opening {
            project,
            registered: context.registered_settings().clone(),
            acting_for_model: context.surface().acts_for_model(),
            exposed: context.exposed(),
            ..Opening::default()
        })
        .await
        .map_err(|error| settings_error(&error, context))?;
    let fetching = Fetching::of(settings.resolved(), engine.clock(), engine.owner()).await;
    let blocking = engine.owner().blocking_threads();
    let store = PluginStore::new(engine.home().plugins());
    let location = CacheLocation::new(
        engine.home().root(),
        engine.home().browsers_path().map(Path::to_path_buf),
    );
    let cache = BrowserCache::new(&location, blocking.clone());
    let catalog = RuntimeCatalog::built_in();
    let now = engine.clock().timestamp();
    let index_folder = engine.home().plugin_index();
    let cached_index = blocking
        .run(move || {
            PluginIndexStore::new(index_folder)
                .status(now)
                .ok()
                .and_then(|status| status.index().cloned())
        })
        .await;
    Ok(InstallContext {
        lockfile,
        resolved: settings.resolved().clone(),
        fetching,
        blocking,
        store,
        cache,
        catalog,
        now,
        cached_index,
    })
}

async fn add_operation(
    params: PluginAddParams,
    context: CallContext,
) -> Result<PluginAdded, CallError> {
    let ctx = open_install_context(params.project, &context).await?;
    let provenance = ProvenancePolicy {
        index: ctx.cached_index.as_ref(),
        settings: Some(&ctx.resolved),
        trust: context.engine().plugin_attestation_trust_root(),
    };
    let mut added = add_checked(
        &params.reference,
        &provenance,
        &ctx.fetching,
        &ctx.blocking,
        &ctx.store,
        &ctx.lockfile,
        &ctx.cache,
        &ctx.catalog,
        ctx.now,
    )
    .await?;
    if params.approve {
        added.approval = Some(approve(&added, &context).await?);
    }
    Ok(added)
}

/// Writes the person's approval of the Plugin `added` into the Operator
/// policy's `plugins.<name>` entry (ADR 0033): allowed, pinned to its
/// digest and each permission it declares approved, the entry's own
/// `settings` kept, recorded in the Audit log as `plugin_approved`. A
/// policy managed elsewhere is left as it is, and the entry to add there
/// is answered instead.
///
/// # Errors
///
/// The settings editor's refusals: `setting_invalid` for a policy that
/// does not read, and an internal error for a file that cannot be read or
/// written, or an approval written that the Audit log cannot record.
async fn approve(added: &PluginAdded, context: &CallContext) -> Result<PluginApproval, CallError> {
    approve_permissions(
        added.name.clone(),
        added.digest.clone(),
        added.permissions.clone(),
        context,
    )
    .await
}

/// Writes the person's approval of `name` at `digest` with `permissions` into
/// the Operator policy's `plugins.<name>` entry (preserving any existing
/// Plugin `settings`), audited as `plugin_approved` (ADR 0033, issues 900
/// and 924).
pub(crate) async fn approve_permissions(
    name: String,
    digest: String,
    permissions: Vec<goodfetch_protocol::plugin::approvals::StatedPermission>,
    context: &CallContext,
) -> Result<PluginApproval, CallError> {
    let mut approved = Approvals::default();
    for stated in &permissions {
        approved.approve(&stated.permission);
    }
    let details = json!({
        "plugin": name,
        "digest": digest,
        "permissions": permissions,
    });
    let path = context.engine().settings().files().operator_policy();
    let policy = path.display().to_string();
    SettingsEditor::of(context)
        .edit(move |draft| {
            let layer = SettingsLayer::OperatorPolicy;
            let held = draft.operator_policy()?;
            let mut plugins = match held.settings.get(PLUGINS.name) {
                Some(Value::Object(plugins)) => plugins.clone(),
                _ => Map::new(),
            };
            let existing = plugins
                .get(&name)
                .and_then(|entry| PluginEntry::read(entry).ok())
                .unwrap_or_default();
            let entry = PluginEntry {
                settings: existing.settings,
                allow: Some(true),
                digest: Some(digest),
                approved,
                spend: existing.spend,
                reduced_confinement: existing.reduced_confinement,
            };
            let entry = serde_json::to_value(&entry)
                .expect("invariant: a Plugin's entry serializes to JSON");
            if held.managed.is_some() {
                let key = format!("{}.{name}", PLUGINS.name);
                return Ok(Change::Kept(PluginApproval::Managed {
                    entry: rewrite("", "settings", &key, &entry)?,
                }));
            }
            plugins.insert(name, entry);
            draft.set(layer, "settings", PLUGINS.name, &Value::Object(plugins))?;
            Ok::<_, settings_editor::Refused>(Change::Made {
                answer: PluginApproval::Written { policy },
                layer,
                kind: &PLUGIN_APPROVED,
                details,
            })
        })
        .await
        .map_err(|refused| config::edit_error(refused, context))
}

/// Installs the Plugin `reference` pins into `store` and pins it in the
/// lockfile at `lockfile`, through `fetching`, its files read and written
/// on `blocking`; answers what it pinned.
///
/// # Errors
///
/// `plugin_lockfile_invalid` or `format_version_unsupported` for a
/// lockfile the engine does not read, `plugin_digest_mismatch` when it
/// pins the Plugin's name to another digest, `plugin_installed` when the
/// store holds the Plugin and the lockfile does not pin it,
/// `artifact_unavailable_offline` and `artifact_download_failed` for a
/// pull that did not happen or failed, `plugin_artifact_refused` for an
/// artifact the engine refuses, and `file_write_failed` when the store or
/// the lockfile cannot be written; the lockfile is then left as it was.
///
/// # Cancel safety
///
/// Dropping the future before the bundle's unpack leaves the store and
/// the lockfile as they were; past it, the Plugin may be in the store and
/// not pinned, which a later `plugin.add` refuses with `plugin_installed`.
pub async fn add(
    reference: &Reference,
    fetching: &Fetching,
    blocking: &BlockingThreads,
    store: &PluginStore,
    lockfile: &Path,
) -> Result<PluginAdded, CallError> {
    let home = store.folder().parent().unwrap_or_else(|| store.folder());
    let cache = BrowserCache::new(&CacheLocation::new(home, None), blocking.clone());
    let catalog = RuntimeCatalog::built_in();
    add_with_runtime(
        reference,
        fetching,
        blocking,
        store,
        lockfile,
        &cache,
        &catalog,
        fetching.clock.timestamp(),
    )
    .await
}

/// Installs the Plugin `reference` pins into `store`, fetches and verifies
/// its declared language runtime into `cache` through `catalog` before
/// committing the Plugin to `store`, and pins it in `lockfile`, treating the
/// caller-supplied exact digest as allowed by Operator policy.
///
/// # Errors
///
/// As [`add`], plus `artifact_unavailable_offline`,
/// `artifact_download_failed` and `plugin_artifact_refused` for the
/// Plugin's declared language runtime; on any failure neither the store
/// nor the lockfile is modified.
#[expect(
    clippy::too_many_arguments,
    reason = "each dependency (registry, runtime cache, catalog, lockfile, clock timestamp) is injected explicitly for testing (ADR 0059)"
)]
pub async fn add_with_runtime(
    reference: &Reference,
    fetching: &Fetching,
    blocking: &BlockingThreads,
    store: &PluginStore,
    lockfile: &Path,
    cache: &BrowserCache,
    catalog: &RuntimeCatalog,
    now: jiff::Timestamp,
) -> Result<PluginAdded, CallError> {
    let resolved = allow_exact_digest_policy(reference.plugin(), &reference.index().to_string());
    let trust = AttestationTrustRoot::bundled();
    let provenance = ProvenancePolicy {
        index: None,
        settings: Some(&resolved),
        trust: &trust,
    };
    add_checked(
        &reference.to_string(),
        &provenance,
        fetching,
        blocking,
        store,
        lockfile,
        cache,
        catalog,
        now,
    )
    .await
}

/// Resolves `raw_reference` against `provenance.index` (refusing look-alike
/// names without an exact digest with `plugin_name_ambiguous`), verifies the
/// artifact's provenance attestation against the index-recorded signer
/// (unless Operator policy in `provenance.settings` explicitly allows the
/// exact digest), pulls and stages the bundle, fetches any declared language
/// runtime into `cache`, commits to `store`, and pins the Plugin (including
/// its verified signer identity) in `lockfile`.
///
/// # Errors
///
/// `plugin_name_ambiguous` (`ProgramError`) for a look-alike or ambiguous
/// name without an exact digest, `plugin_attestation_invalid` for a missing
/// or mismatching provenance attestation without an Operator policy
/// exact-digest allowance, plus all errors of [`add_with_runtime`].
#[expect(
    clippy::too_many_arguments,
    reason = "each dependency (provenance policy, registry, runtime cache, catalog, lockfile, clock timestamp) is injected explicitly for testing (ADR 0059)"
)]
pub async fn add_checked(
    raw_reference: &str,
    provenance: &ProvenancePolicy<'_>,
    fetching: &Fetching,
    blocking: &BlockingThreads,
    store: &PluginStore,
    lockfile: &Path,
    cache: &BrowserCache,
    catalog: &RuntimeCatalog,
    now: jiff::Timestamp,
) -> Result<PluginAdded, CallError> {
    let resolved_ref = plugin_provenance::resolve_reference(raw_reference, provenance.index)
        .map_err(|error| match error {
            ResolveError::Ambiguous { name, similar } => {
                plugin_provenance::name_ambiguous_error(&name, similar)
            }
            ResolveError::Malformed(err) => {
                CallError::protocol(ProtocolFailure::InvalidParams, format!("{err}"))
            }
        })?;
    let reference = resolved_ref.reference();
    let name = reference.plugin().to_owned();
    let digest = reference.index().to_string();
    let mut pins = {
        let path = lockfile.to_owned();
        blocking.run(move || read_lockfile(&path)).await?
    };
    let pinned_signer = match pins.locked(&name) {
        Some(locked) if locked.digest != digest => {
            return Err(mismatch(&name, &locked.digest, &digest, lockfile));
        }
        Some(locked) => Some(locked.signer.clone()),
        None => None,
    };
    let pinned = pinned_signer.is_some();
    let held = {
        let (store, name) = (store.clone(), name.clone());
        blocking.run(move || store.installed(&name)).await
    };
    match held {
        Ok(held) if pinned => {
            let signer = pinned_signer.flatten().and_then(|recorded| {
                provenance
                    .index
                    .and_then(|idx| idx.entry(&name))
                    .filter(|entry| entry.signer.to_string() == recorded)
                    .map(|entry| entry.signer.clone())
            });
            return Ok(added(held, digest, lockfile, signer));
        }
        Ok(held) => return Err(installed(&name, &held.manifest.version)),
        Err(StoreError::NotInstalled(_) | StoreError::Versions { versions: 0, .. }) => {}
        Err(error) => {
            return Err(CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the Plugin store cannot be read: {error}"),
            ));
        }
    }
    let verified_signer =
        check_provenance(reference, &name, &digest, provenance, fetching, store).await?;
    let staged = plugin_install::stage(reference, fetching, blocking, store)
        .await
        .map_err(|error| refusal(error, reference, store))?;
    {
        let (store_clone, check_name, manifest) =
            (store.clone(), name.clone(), staged.manifest().clone());
        blocking
            .run(move || store_clone.check_scheme_conflict(&check_name, &manifest))
            .await
            .map_err(|error| refusal(InstallError::Store(error), reference, store))?;
    }
    let runtime = staged.manifest().runtime.clone();
    let version = staged.manifest().version.clone();
    let _held_runtime = plugin_runtime::ensure_installed(
        &runtime,
        (&name, &version),
        store,
        plugin_runtime::RuntimeTarget {
            cache,
            catalog,
            now,
        },
        fetching,
        blocking,
    )
    .await?;
    let held = staged
        .commit(blocking, store)
        .await
        .map_err(|error| refusal(error, reference, store))?;
    if !pinned {
        pins.lock(LockedPlugin {
            name: name.clone(),
            version: held.manifest.version.clone(),
            digest: digest.clone(),
            signer: verified_signer.as_ref().map(ToString::to_string),
            approvals: None,
        })
        .map_err(|found| mismatch(&name, &found.locked, &digest, lockfile))?;
        let (path, text) = (lockfile.to_owned(), plugin_lockfile::text(&pins));
        blocking
            .run(move || write_lockfile(&path, &text))
            .await
            .map_err(|error| unwritten(&error, lockfile, &name))?;
    }
    Ok(added(held, digest, lockfile, verified_signer))
}

/// Fetches provenance attestations for `reference`, translating transport and
/// parse errors into `CallError` (or returning `Ok(None)` when Operator policy
/// allows the exact digest and no attestation could be fetched).
pub(crate) async fn fetch_attestations_or_allow(
    reference: &Reference,
    name: &str,
    digest: &str,
    policy_allows: bool,
    expected_signer: Option<&SignerIdentity>,
    fetching: &Fetching,
    store: &PluginStore,
) -> Result<Option<Vec<goodfetch_protocol::plugin::index::ProvenanceAttestation>>, CallError> {
    let artifact = ArtifactId {
        kind: ArtifactKind::Plugin,
        name: name.to_owned(),
        build: digest.to_owned(),
    };
    match plugin_install::fetch_attestations(reference, fetching).await {
        Ok(attestations) => Ok(Some(attestations)),
        Err(AttestationFetchError::Offline(layer)) => Err(offline(artifact, layer, store)),
        Err(AttestationFetchError::Pull(failure)) => {
            if policy_allows {
                Ok(None)
            } else {
                Err(pull_failed(artifact, reference, &failure))
            }
        }
        Err(AttestationFetchError::Malformed(_)) => {
            if policy_allows {
                Ok(None)
            } else {
                Err(plugin_provenance::attestation_invalid_error(
                    name,
                    digest,
                    AttestationFailureReason::Malformed,
                    expected_signer.cloned(),
                    None,
                ))
            }
        }
    }
}

/// Checks provenance attestation for `reference` against `provenance`,
/// returning the verified [`SignerIdentity`] when attested and matching,
/// `None` when allowed without a matching attestation (exact digest allowed
/// in Operator policy, or first-party without attestation), or
/// `plugin_attestation_invalid` when refused.
async fn check_provenance(
    reference: &Reference,
    name: &str,
    digest: &str,
    provenance: &ProvenancePolicy<'_>,
    fetching: &Fetching,
    store: &PluginStore,
) -> Result<Option<SignerIdentity>, CallError> {
    let policy_allows = provenance.settings.is_some_and(|settings| {
        plugin_provenance::policy_allows_exact_digest(settings, name, digest)
    });
    if policy_allows && provenance.index.is_none() {
        return Ok(None);
    }
    let expected_signer = provenance
        .index
        .and_then(|idx| idx.entry(name))
        .map(|entry| &entry.signer);
    let origin = provenance
        .index
        .map_or(Origin::ThirdParty, |idx| idx.origin_of(name, Some(digest)));
    let Some(attestations) = fetch_attestations_or_allow(
        reference,
        name,
        digest,
        policy_allows,
        expected_signer,
        fetching,
        store,
    )
    .await?
    else {
        return Ok(None);
    };
    match plugin_provenance::verify_attestations(
        &attestations,
        digest,
        expected_signer,
        provenance.trust,
    ) {
        AttestationVerdict::Verified(signer) => Ok(Some(signer)),
        AttestationVerdict::Refused {
            reason,
            expected_signer,
            found_signer,
        } => {
            if policy_allows
                || (origin == Origin::FirstParty && reason == AttestationFailureReason::Missing)
            {
                return Ok(None);
            }
            Err(plugin_provenance::attestation_invalid_error(
                name,
                digest,
                reason,
                expected_signer,
                found_signer,
            ))
        }
    }
}

/// Constructs a resolved Operator policy allowing the exact `(name, digest)`
/// pair for callers that explicitly pass a digest-pinned [`Reference`]
/// without a separate Operator policy.
fn allow_exact_digest_policy(name: &str, digest: &str) -> Resolved {
    let mut map = Map::new();
    map.insert(
        PLUGINS.name.to_owned(),
        json!({
            name: {
                "allow": true,
                "digest": digest,
            }
        }),
    );
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(SettingsLayer::OperatorPolicy, &map, Product::Collie)
                .expect("invariant: valid exact-digest Operator policy entry"),
            ..Policy::default()
        },
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie)
        .expect("invariant: single-layer exact-digest policy resolves")
}

/// The lockfile at `path`, an empty one when there is none.
pub(crate) fn read_lockfile(path: &Path) -> Result<PluginLockfile, CallError> {
    match std::fs::read_to_string(path) {
        Ok(text) => plugin_lockfile::read(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PluginLockfile::empty()),
        Err(error) => Err(CallError::protocol(
            ProtocolFailure::InternalError,
            format!(
                "the Plugin lockfile {} cannot be read: {error}",
                path.display()
            ),
        )),
    }
}

/// Writes `text` as the lockfile at `path`, its folder made when missing.
pub(crate) fn write_lockfile(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        durable::create_folder(folder)?;
    }
    durable::replace(path, text.as_bytes())
}

/// What `plugin.add` answers for the Plugin `held`, pinned to `digest` in
/// `lockfile`.
fn added(
    held: Installed,
    digest: String,
    lockfile: &Path,
    signer: Option<SignerIdentity>,
) -> PluginAdded {
    let permissions = Permission::needed_by(&held.manifest)
        .into_iter()
        .map(stated)
        .collect();
    PluginAdded {
        name: held.manifest.name,
        version: held.manifest.version,
        digest,
        lockfile: lockfile.display().to_string(),
        kinds: held.manifest.kinds,
        network_level: held.manifest.network_level,
        env: held.manifest.env,
        files: held.manifest.files,
        processes: held.manifest.processes,
        browsers: held.manifest.browsers,
        spend: held.manifest.spend,
        permissions,
        signer,
        approval: None,
    }
}

/// `plugin_digest_mismatch` for `name`, pinned to `locked` in `lockfile`
/// and given as `found`.
fn mismatch(name: &str, locked: &str, found: &str, lockfile: &Path) -> CallError {
    let lockfile = lockfile.display().to_string();
    CallError::new(
        &PLUGIN_DIGEST_MISMATCH,
        format!(
            "The Plugin lockfile {lockfile} pins the Plugin `{name}` to {locked}, not to {found}: nothing was installed."
        ),
        &PluginDigestMismatchDetails {
            name: name.to_owned(),
            locked: locked.to_owned(),
            found: found.to_owned(),
            fix: format!(
                "Give the reference with {locked}, or remove the Plugin `{name}` from {lockfile} to accept {found}"
            ),
            lockfile,
        },
    )
}

/// `plugin_installed` for `name`, whose `version` the store holds.
fn installed(name: &str, version: &str) -> CallError {
    CallError::new(
        &PLUGIN_INSTALLED,
        format!(
            "The Plugin `{name}` is installed already, at version {version}, and the Plugin lockfile does not pin it to this digest: `plugin update` replaces it."
        ),
        &(),
    )
}

/// The failure `plugin.add` answers for the install of `reference` into
/// `store` that failed with `error`.
pub(crate) fn refusal(
    error: InstallError,
    reference: &Reference,
    store: &PluginStore,
) -> CallError {
    let artifact = ArtifactId {
        kind: ArtifactKind::Plugin,
        name: reference.plugin().to_owned(),
        build: reference.index().to_string(),
    };
    match error {
        InstallError::Offline(layer) => offline(artifact, layer, store),
        InstallError::Pull(failure) => pull_failed(artifact, reference, &failure),
        InstallError::Document(error) => artifact_refused(reference, &error),
        InstallError::Bundle(error) => artifact_refused(reference, &error),
        InstallError::Store(StoreError::Installed { name, version }) => installed(&name, &version),
        InstallError::Store(StoreError::Unreadable { source, .. })
        | InstallError::Staging(source) => store_unwritten(&source, store, reference.plugin()),
        InstallError::Store(error) => artifact_refused(reference, &error),
    }
}

/// `artifact_unavailable_offline` for `artifact`, whose pull `egress.offline`,
/// set at `layer`, refused.
pub(crate) fn offline(
    artifact: ArtifactId,
    layer: SettingsLayer,
    store: &PluginStore,
) -> CallError {
    let name = artifact.name.clone();
    CallError::new(
        &ARTIFACT_UNAVAILABLE_OFFLINE,
        format!(
            "The Plugin `{name}` is not in the Plugin store, and `{}` refuses its pull",
            EGRESS_OFFLINE.name
        ),
        &ArtifactUnavailableOfflineDetails {
            artifact,
            pinned_by: Vec::new(),
            cache: store.folder().display().to_string(),
            limit: SettingLimit {
                key: EGRESS_OFFLINE.name.to_owned(),
                value: serde_json::Value::Bool(true),
                layer,
                origin: None,
            },
            fix: format!(
                "Run `plugin add` where `{}` is not set, or lift it where it was set",
                EGRESS_OFFLINE.name
            ),
        },
    )
}

/// `artifact_download_failed` for `artifact`, which `reference`'s registry
/// did not serve.
pub(crate) fn pull_failed(
    artifact: ArtifactId,
    reference: &Reference,
    failure: &PullFailure,
) -> CallError {
    let host = reference.registry().to_owned();
    CallError::new(
        &ARTIFACT_DOWNLOAD_FAILED,
        format!(
            "Pulling the Plugin `{}` from {host} failed: {failure}",
            artifact.name
        ),
        &ArtifactDownloadFailedDetails {
            artifact,
            pinned_by: Vec::new(),
            failure: failure.download_failure(),
            fix: format!("Retry, or check that {host} serves {reference}"),
            host,
        },
    )
}

/// `plugin_artifact_refused` for `reference`'s artifact, refused for
/// `why`.
fn artifact_refused(reference: &Reference, why: &dyn std::fmt::Display) -> CallError {
    CallError::new(
        &PLUGIN_ARTIFACT_REFUSED,
        format!(
            "The artifact of the Plugin `{}` is refused: {why}. Nothing was installed.",
            reference.plugin()
        ),
        &(),
    )
}

/// `file_write_failed`, side `engine`, for the Plugin `name`, which
/// `store` could not take.
fn store_unwritten(error: &io::Error, store: &PluginStore, name: &str) -> CallError {
    let folder = store.folder();
    CallError::new(
        &FILE_WRITE_FAILED,
        format!(
            "The Plugin store {} could not take the Plugin `{name}` ({error}): free space there, or make it writable.",
            folder.display()
        ),
        &FileWriteFailedDetails {
            reason: reason_of(error.kind()).to_owned(),
            os_code: error.raw_os_error().and_then(os_code).map(str::to_owned),
            path: Some(folder.display().to_string()),
            side: WriteSide::Engine,
        },
    )
}

/// `file_write_failed`, side `program`, for the lockfile `lockfile`, which
/// could not pin the installed Plugin `name`.
pub(crate) fn unwritten(error: &io::Error, lockfile: &Path, name: &str) -> CallError {
    CallError::new(
        &FILE_WRITE_FAILED,
        format!(
            "The Plugin `{name}` is installed, but the Plugin lockfile {} could not pin it ({error}).",
            lockfile.display()
        ),
        &FileWriteFailedDetails {
            reason: reason_of(error.kind()).to_owned(),
            os_code: error.raw_os_error().and_then(os_code).map(str::to_owned),
            path: Some(lockfile.display().to_string()),
            side: WriteSide::Program,
        },
    )
}

#[cfg(test)]
mod tests;
