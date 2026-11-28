//! `plugin.list` and `plugin.remove` RPC handlers (ADRs 0033, 0094, 0098 and
//! 0099; issue 928).
//!
//! - `plugin.list` (a read, every surface) reads the shared Plugin store
//!   without modifying it, returning each installed Plugin's kinds, digest,
//!   signer, network level, Operator policy approvals, cached index
//!   conformance results, `settings_schema`, and whether the running Product
//!   carries its kinds (`available_in_product`, `unsupported_kinds`).
//! - `plugin.remove` (person-only) refuses with `run_live` if any live Run is
//!   using the Plugin; otherwise deletes it from the Plugin store, removes its
//!   entry from the Plugin lockfile, and clears its Operator policy approvals
//!   (audited as `plugin_removed`).
#![expect(
    clippy::disallowed_methods,
    reason = "store and lockfile IO run on the tasks module's blocking policy, never on an async worker"
)]

use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::plugin::PluginKindName;
use goodfetch_protocol::plugin::lockfile::{LOCKFILE_NAME, PluginLockfile};
use goodfetch_protocol::plugin::manage::{
    InstalledPluginApprovals, InstalledPluginInfo, PLUGIN_LIST, PLUGIN_REMOVE,
    PLUGIN_REMOVED_AUDIT, PluginApprovalsRemoval, PluginListParams, PluginListResult,
    PluginRemoveParams, PluginRemoved,
};
use goodfetch_protocol::plugin::settings::{PLUGINS, PluginEntry};
use goodfetch_protocol::run::{RUN_LIVE, RunDetails};
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::{Map, Value, json};

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::config;
use crate::durable;
use crate::plugin_index::PluginIndexStore;
use crate::plugin_lockfile;
use crate::plugin_store::{PluginStore, StoreError};
use crate::run::settings_error;
use crate::run_files::{os_code, reason_of};
use crate::settings::{Opening, Resolved};
use crate::settings_editor::{self, Change, SettingsEditor};

/// The `plugin.list` and `plugin.remove` handlers.
pub(crate) fn handlers() -> [Handler; 2] {
    [
        Handler::new(&PLUGIN_LIST, list_operation),
        Handler::new(&PLUGIN_REMOVE, remove_operation),
    ]
}

fn lockfile_path(
    project: Option<&goodfetch_protocol::settings::ProjectSettings>,
    context: &CallContext,
) -> PathBuf {
    let beside = match project {
        Some(project) => Path::new(&project.path),
        None => context.engine().settings().files().user_configuration(),
    };
    beside
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LOCKFILE_NAME)
}

fn load_lockfile_or_empty(path: &Path) -> Result<PluginLockfile, CallError> {
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

fn approvals_of(resolved: &Resolved, name: &str) -> Option<InstalledPluginApprovals> {
    let entry = match &resolved.value(&PLUGINS)[name] {
        Value::Null => return None,
        value => PluginEntry::read(value).ok()?,
    };
    if entry.allow.is_none() && entry.digest.is_none() && entry.approved.is_empty() {
        return None;
    }
    Some(InstalledPluginApprovals {
        allow: entry.allow,
        digest: entry.digest,
        approved: entry.approved,
    })
}

async fn list_operation(
    params: PluginListParams,
    context: CallContext,
) -> Result<PluginListResult, CallError> {
    let engine = context.engine();
    let product = context.identity().product;
    let lockfile = lockfile_path(params.project.as_ref(), &context);
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
    let blocking = engine.owner().blocking_threads();
    let store_folder = engine.home().plugins();
    let index_folder = engine.home().plugin_index();
    let now = engine.clock().timestamp();
    let (installed, pins, index) = blocking
        .run(move || {
            let installed = PluginStore::new(store_folder)
                .installed_all()
                .map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!("the Plugin store cannot be listed: {error}"),
                    )
                })?;
            let pins =
                load_lockfile_or_empty(&lockfile).unwrap_or_else(|_| PluginLockfile::empty());
            let index = PluginIndexStore::new(index_folder)
                .status(now)
                .ok()
                .and_then(|status| status.index().cloned());
            Ok::<_, CallError>((installed, pins, index))
        })
        .await?;

    let resolved = settings.resolved();
    let plugins = installed
        .into_iter()
        .map(|item| {
            let name = item.manifest.name;
            let locked = pins.locked(&name);
            let index_entry = index.as_ref().and_then(|idx| idx.entry(&name));
            let digest = item
                .digest
                .or_else(|| locked.map(|entry| entry.digest.clone()));
            let signer = locked
                .and_then(|entry| entry.signer.clone())
                .or_else(|| index_entry.map(|entry| entry.signer.to_string()));
            let conformance = index_entry
                .map(|entry| entry.conformance.clone())
                .unwrap_or_default();
            let unsupported_kinds: Vec<PluginKindName> = item
                .manifest
                .kinds
                .iter()
                .copied()
                .filter(|kind| !kind.in_product(product))
                .collect();
            let available_in_product = item
                .manifest
                .kinds
                .iter()
                .any(|kind| kind.in_product(product));
            InstalledPluginInfo {
                approvals: approvals_of(resolved, &name),
                name,
                version: item.manifest.version,
                kinds: item.manifest.kinds,
                digest,
                signer,
                network_level: item.manifest.network_level,
                conformance,
                settings_schema: item.manifest.settings_schema,
                available_in_product,
                unsupported_kinds,
            }
        })
        .collect();

    Ok(PluginListResult { product, plugins })
}

async fn remove_operation(
    params: PluginRemoveParams,
    context: CallContext,
) -> Result<PluginRemoved, CallError> {
    let name = params.name;
    let using = context.runs().runs_using_plugin(&name).await;
    if let Some(run_id) = using.into_iter().next() {
        return Err(CallError::new(
            &RUN_LIVE,
            format!(
                "The Plugin `{name}` is currently in use by live Run `{run_id}`: stop the Run before removing it."
            ),
            &RunDetails { run_id, path: None },
        ));
    }

    let engine = context.engine();
    let blocking = engine.owner().blocking_threads();
    let store_folder = engine.home().plugins();
    let name_for_store = name.clone();
    let removed_from_store = blocking
        .run(move || PluginStore::new(store_folder).remove(&name_for_store))
        .await
        .map_err(|error| match error {
            StoreError::Name(_) => {
                CallError::protocol(ProtocolFailure::InvalidParams, error.to_string())
            }
            other => CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the Plugin `{name}` could not be removed from the store: {other}"),
            ),
        })?;

    let lockfile = lockfile_path(params.project.as_ref(), &context);
    let lockfile_for_io = lockfile.clone();
    let name_for_lock = name.clone();
    let removed_from_lockfile = blocking
        .run(move || -> Result<bool, CallError> {
            let mut pins = load_lockfile_or_empty(&lockfile_for_io)?;
            if pins.remove(&name_for_lock).is_none() {
                return Ok(false);
            }
            let text = plugin_lockfile::text(&pins);
            durable::replace(&lockfile_for_io, text.as_bytes()).map_err(|error| {
                CallError::new(
                    &FILE_WRITE_FAILED,
                    format!(
                        "The Plugin lockfile {} could not be updated after removing `{name_for_lock}` ({error}).",
                        lockfile_for_io.display()
                    ),
                    &FileWriteFailedDetails {
                        reason: reason_of(error.kind()).to_owned(),
                        os_code: error.raw_os_error().and_then(os_code).map(str::to_owned),
                        path: Some(lockfile_for_io.display().to_string()),
                        side: WriteSide::Program,
                    },
                )
            })?;
            Ok(true)
        })
        .await?;

    let approvals = clear_approvals(&name, &context).await?;

    Ok(PluginRemoved {
        name,
        removed_from_store,
        removed_from_lockfile,
        approvals,
    })
}

async fn clear_approvals(
    name: &str,
    context: &CallContext,
) -> Result<PluginApprovalsRemoval, CallError> {
    let target = name.to_owned();
    let path = context.engine().settings().files().operator_policy();
    let policy = path.display().to_string();
    let details = json!({ "plugin": target });
    SettingsEditor::of(context)
        .edit(move |draft| {
            let layer = SettingsLayer::OperatorPolicy;
            let held = draft.operator_policy()?;
            let mut plugins = match held.settings.get(PLUGINS.name) {
                Some(Value::Object(plugins)) => plugins.clone(),
                _ => return Ok(Change::Kept(PluginApprovalsRemoval::Absent)),
            };
            let Some(existing) = plugins.get(&target) else {
                return Ok(Change::Kept(PluginApprovalsRemoval::Absent));
            };
            let entry = PluginEntry::read(existing).unwrap_or_default();
            let had_approvals = entry.allow.is_some()
                || entry.digest.is_some()
                || !entry.approved.is_empty()
                || !entry.spend.is_empty()
                || entry.reduced_confinement;
            if !had_approvals {
                return Ok(Change::Kept(PluginApprovalsRemoval::Absent));
            }
            if held.managed.is_some() {
                return Ok(Change::Kept(PluginApprovalsRemoval::Managed {
                    key: format!("{}.{target}", PLUGINS.name),
                }));
            }
            if entry.settings.is_empty() {
                plugins.remove(&target);
            } else {
                let remaining = PluginEntry {
                    settings: entry.settings,
                    ..PluginEntry::default()
                };
                let value = serde_json::to_value(&remaining)
                    .expect("invariant: a Plugin's entry serializes to JSON");
                plugins.insert(target, value);
            }
            if plugins.is_empty() {
                draft.set(layer, "settings", PLUGINS.name, &Value::Object(Map::new()))?;
            } else {
                draft.set(layer, "settings", PLUGINS.name, &Value::Object(plugins))?;
            }
            Ok::<_, settings_editor::Refused>(Change::Made {
                answer: PluginApprovalsRemoval::Removed { policy },
                layer,
                kind: &PLUGIN_REMOVED_AUDIT,
                details,
            })
        })
        .await
        .map_err(|refused| config::edit_error(refused, context))
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::catalog::Product;

    use super::*;

    #[test]
    fn product_membership_marks_foxhound_only_kinds_unavailable_on_collie() {
        let sink = PluginKindName::from_static("sink");
        let fetch_backend = PluginKindName::from_static("fetch_backend");
        assert!(!sink.in_product(Product::Collie));
        assert!(!fetch_backend.in_product(Product::Collie));
        assert!(sink.in_product(Product::Foxhound));
        assert!(fetch_backend.in_product(Product::Foxhound));
        assert!(PluginKindName::EXTRACT.in_product(Product::Collie));
    }
}
