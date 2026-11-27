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
use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::plugin::add::{
    PLUGIN_ADD, PLUGIN_ARTIFACT_REFUSED, PLUGIN_DIGEST_MISMATCH, PLUGIN_INSTALLED, PluginAddParams,
    PluginAdded, PluginApproval, PluginDigestMismatchDetails,
};
use goodfetch_protocol::plugin::approvals::{Approvals, PLUGIN_APPROVED, Permission, stated};
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
use crate::plugin_install::{self, InstallError, PullFailure};
use crate::plugin_lockfile;
use crate::plugin_oci::Reference;
use crate::plugin_runtime::{self, RuntimeCatalog};
use crate::plugin_store::{Installed, PluginStore, StoreError};
use crate::run::settings_error;
use crate::run_files::{os_code, reason_of};
use crate::settings::{Opening, rewrite};
use crate::settings_editor::{self, Change, SettingsEditor};
use crate::tasks::BlockingThreads;

/// The `plugin.*` handlers.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&PLUGIN_ADD, add_operation)]
}

async fn add_operation(
    params: PluginAddParams,
    context: CallContext,
) -> Result<PluginAdded, CallError> {
    let reference: Reference = params
        .reference
        .parse()
        .map_err(|error| CallError::protocol(ProtocolFailure::InvalidParams, format!("{error}")))?;
    let engine = context.engine();
    let beside = match &params.project {
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
            project: params.project,
            registered: context.registered_settings().clone(),
            acting_for_model: context.surface().acts_for_model(),
            exposed: context.exposed(),
            ..Opening::default()
        })
        .await
        .map_err(|error| settings_error(&error, &context))?;
    let fetching = Fetching::of(settings.resolved(), engine.clock(), engine.owner()).await;
    let blocking = engine.owner().blocking_threads();
    let store = PluginStore::new(engine.home().plugins());
    let location = CacheLocation::new(
        engine.home().root(),
        engine.home().browsers_path().map(Path::to_path_buf),
    );
    let cache = BrowserCache::new(&location, blocking.clone());
    let catalog = RuntimeCatalog::built_in();
    let mut added = add_with_runtime(
        &reference,
        &fetching,
        &blocking,
        &store,
        &lockfile,
        &cache,
        &catalog,
        engine.clock().timestamp(),
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
    let name = added.name.clone();
    let digest = added.digest.clone();
    let mut approved = Approvals::default();
    for stated in &added.permissions {
        approved.approve(&stated.permission);
    }
    let details = json!({
        "plugin": name,
        "digest": digest,
        "permissions": added.permissions,
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
            let settings = plugins
                .get(&name)
                .and_then(|entry| PluginEntry::read(entry).ok())
                .map(|entry| entry.settings)
                .unwrap_or_default();
            let entry = PluginEntry {
                settings,
                allow: Some(true),
                digest: Some(digest),
                approved,
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
/// committing the Plugin to `store`, and pins it in `lockfile`.
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
    let name = reference.plugin().to_owned();
    let digest = reference.index().to_string();
    let mut pins = {
        let path = lockfile.to_owned();
        blocking.run(move || read_lockfile(&path)).await?
    };
    let pinned = match pins.locked(&name) {
        Some(locked) if locked.digest != digest => {
            return Err(mismatch(&name, &locked.digest, &digest, lockfile));
        }
        locked => locked.is_some(),
    };
    let held = {
        let (store, name) = (store.clone(), name.clone());
        blocking.run(move || store.installed(&name)).await
    };
    match held {
        Ok(held) if pinned => return Ok(added(held, digest, lockfile)),
        Ok(held) => return Err(installed(&name, &held.manifest.version)),
        Err(StoreError::NotInstalled(_) | StoreError::Versions { versions: 0, .. }) => {}
        Err(error) => {
            return Err(CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the Plugin store cannot be read: {error}"),
            ));
        }
    }
    let staged = plugin_install::stage(reference, fetching, blocking, store)
        .await
        .map_err(|error| refusal(error, reference, store))?;
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
            signer: None,
            approvals: None,
        })
        .map_err(|found| mismatch(&name, &found.locked, &digest, lockfile))?;
        let (path, text) = (lockfile.to_owned(), plugin_lockfile::text(&pins));
        blocking
            .run(move || write_lockfile(&path, &text))
            .await
            .map_err(|error| unwritten(&error, lockfile, &name))?;
    }
    Ok(added(held, digest, lockfile))
}

/// The lockfile at `path`, an empty one when there is none.
fn read_lockfile(path: &Path) -> Result<PluginLockfile, CallError> {
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
fn write_lockfile(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        durable::create_folder(folder)?;
    }
    durable::replace(path, text.as_bytes())
}

/// What `plugin.add` answers for the Plugin `held`, pinned to `digest` in
/// `lockfile`.
fn added(held: Installed, digest: String, lockfile: &Path) -> PluginAdded {
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
        permissions,
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
fn refusal(error: InstallError, reference: &Reference, store: &PluginStore) -> CallError {
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
fn offline(artifact: ArtifactId, layer: SettingsLayer, store: &PluginStore) -> CallError {
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
fn pull_failed(artifact: ArtifactId, reference: &Reference, failure: &PullFailure) -> CallError {
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
fn unwritten(error: &io::Error, lockfile: &Path, name: &str) -> CallError {
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
