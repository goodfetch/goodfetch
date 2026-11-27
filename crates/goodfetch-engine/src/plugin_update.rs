//! `plugin.update` (ADRs 0033, 0052 and 0099; issue 924): a person's answer
//! that previews or applies an update to an installed Plugin, showing the new
//! digest, verified signer identity and [`PermissionDiff`] (added, widened and
//! removed permissions).
//!
//! - Any added or widened permission requires explicit approval (`approve:
//!   true`), else the apply is refused with `plugin_permission_denied` and the
//!   store, lockfile and Operator policy stay untouched.
//! - A release whose verified signer differs from the previously recorded
//!   signer is refused with `plugin_signer_changed` until explicitly approved
//!   (`approve_signer_change: true`).
//! - Plugins are never updated automatically on engine start, Run start or
//!   Plugin index refresh.

use std::path::Path;

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::plugin::add::AttestationFailureReason;
use goodfetch_protocol::plugin::approvals::{
    Origin, PLUGIN_PERMISSION_DENIED, Permission, PluginPermissionDeniedDetails, StatedPermission,
    fix, stated,
};
use goodfetch_protocol::plugin::index::{PluginIndex, SignerIdentity};
use goodfetch_protocol::plugin::lockfile::LockedPlugin;
use goodfetch_protocol::plugin::update::{
    PLUGIN_SIGNER_CHANGED, PLUGIN_UPDATE, PermissionDiff, PluginSignerChangedDetails,
    PluginUpdateParams, PluginUpdated,
};

use crate::browser_cache::BrowserCache;
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::pinned_download::Fetching;
use crate::plugin_add::{self, ProvenancePolicy};
use crate::plugin_install;
use crate::plugin_lockfile;
use crate::plugin_oci::Reference;
use crate::plugin_provenance::{self, AttestationVerdict, ResolveError};
use crate::plugin_runtime::{self, RuntimeCatalog};
use crate::plugin_store::{Installed, PluginStore, StoreError};
use crate::tasks::BlockingThreads;

/// Options controlling whether [`update_checked`] previews or applies the
/// update, and which approvals the person gave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpdateOptions {
    /// Inspect the target release without modifying the store or lockfile.
    pub preview: bool,
    /// Person approved added or widened permissions.
    pub approve: bool,
    /// Person approved a changed signer identity.
    pub approve_signer_change: bool,
}

/// The `plugin.update` handler.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&PLUGIN_UPDATE, update_operation)]
}

async fn update_operation(
    params: PluginUpdateParams,
    context: CallContext,
) -> Result<PluginUpdated, CallError> {
    let ctx = plugin_add::open_install_context(params.project, &context).await?;
    let provenance = ProvenancePolicy {
        index: ctx.cached_index.as_ref(),
        settings: Some(&ctx.resolved),
        trust: context.engine().plugin_attestation_trust_root(),
    };
    let options = UpdateOptions {
        preview: params.preview,
        approve: params.approve,
        approve_signer_change: params.approve_signer_change,
    };
    let (mut updated, target_permissions) = update_checked(
        &params.reference,
        options,
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
    if updated.applied {
        updated.approval = Some(
            plugin_add::approve_permissions(
                updated.name.clone(),
                updated.digest.clone(),
                target_permissions,
                &context,
            )
            .await?,
        );
    }
    Ok(updated)
}

/// Previews or applies an update of an installed Plugin to `raw_reference`,
/// verifying provenance, checking for signer changes, computing the
/// [`PermissionDiff`], and updating `store` and `lockfile` only when not in
/// preview mode and all required approvals are present (ADR 0033, issue 924).
///
/// Returns the [`PluginUpdated`] answer together with the target release's
/// full [`StatedPermission`] list (so the RPC handler can update Operator
/// policy approvals when applied).
///
/// # Errors
///
/// - `plugin_name_ambiguous` for a look-alike or ambiguous name without a
///   digest,
/// - `plugin_attestation_invalid` for a missing, malformed, bad-signature or
///   wrong-digest provenance attestation without an exact-digest policy
///   allowance,
/// - `plugin_signer_changed` when the verified signer differs from the
///   previously recorded signer and `options.approve_signer_change` is false,
/// - `plugin_permission_denied` when the diff adds or widens permissions and
///   `options.approve` is false,
/// - `invalid_params` when the Plugin is not installed or the reference is
///   malformed,
/// - plus download, artifact and file-write errors from `plugin.add`.
#[expect(
    clippy::too_many_arguments,
    reason = "each dependency (provenance policy, registry, runtime cache, catalog, lockfile, clock timestamp) is injected explicitly for testing (ADR 0059)"
)]
pub async fn update_checked(
    raw_reference: &str,
    options: UpdateOptions,
    provenance: &ProvenancePolicy<'_>,
    fetching: &Fetching,
    blocking: &BlockingThreads,
    store: &PluginStore,
    lockfile: &Path,
    cache: &BrowserCache,
    catalog: &RuntimeCatalog,
    now: jiff::Timestamp,
) -> Result<(PluginUpdated, Vec<StatedPermission>), CallError> {
    let resolved_ref = resolve_update_reference(raw_reference, provenance.index)?;
    let reference = resolved_ref.reference();
    let name = reference.plugin().to_owned();
    let digest = reference.index().to_string();

    let installed = read_installed(store, &name, blocking).await?;
    let path = lockfile.to_owned();
    let mut pins = blocking
        .run(move || plugin_add::read_lockfile(&path))
        .await?;
    let previous_signer = pins.locked(&name).and_then(|locked| locked.signer.clone());
    let previous_digest = installed
        .digest
        .clone()
        .or_else(|| pins.locked(&name).map(|locked| locked.digest.clone()));

    let (verified_signer, signer_changed) = check_update_provenance(
        reference,
        &name,
        &digest,
        previous_signer.as_deref(),
        provenance,
        fetching,
        store,
    )
    .await?;

    let staged = plugin_install::stage(reference, fetching, blocking, store)
        .await
        .map_err(|error| plugin_add::refusal(error, reference, store))?;
    {
        let (store_clone, check_name, manifest) =
            (store.clone(), name.clone(), staged.manifest().clone());
        blocking
            .run(move || store_clone.check_scheme_conflict(&check_name, &manifest))
            .await
            .map_err(|error| {
                plugin_add::refusal(plugin_install::InstallError::Store(error), reference, store)
            })?;
    }

    let new_manifest = staged.manifest().clone();
    let diff = PermissionDiff::between(&installed.manifest, &new_manifest);
    let target_permissions: Vec<StatedPermission> = Permission::needed_by(&new_manifest)
        .into_iter()
        .map(stated)
        .collect();

    let (version, updated) = if options.preview {
        drop(staged);
        (new_manifest.version, false)
    } else {
        check_update_approvals(
            &name,
            &digest,
            options,
            signer_changed,
            previous_signer.as_deref(),
            verified_signer.as_ref(),
            &diff,
        )?;
        let held = commit_staged_update(
            staged,
            reference,
            &name,
            &digest,
            verified_signer.as_ref(),
            &mut pins,
            store,
            lockfile,
            (cache, catalog, now),
            (fetching, blocking),
        )
        .await?;
        (held.manifest.version, true)
    };

    Ok((
        build_answer(
            &installed,
            &version,
            previous_digest,
            digest,
            lockfile,
            previous_signer,
            verified_signer,
            signer_changed,
            diff,
            updated,
        ),
        target_permissions,
    ))
}

fn resolve_update_reference(
    raw_reference: &str,
    index: Option<&PluginIndex>,
) -> Result<plugin_provenance::ResolvedReference, CallError> {
    plugin_provenance::resolve_reference(raw_reference, index).map_err(|error| match error {
        ResolveError::Ambiguous { name, similar } => {
            plugin_provenance::name_ambiguous_error(&name, similar)
        }
        ResolveError::Malformed(err) => {
            CallError::protocol(ProtocolFailure::InvalidParams, format!("{err}"))
        }
    })
}

fn check_update_approvals(
    name: &str,
    digest: &str,
    options: UpdateOptions,
    signer_changed: bool,
    previous_signer: Option<&str>,
    verified_signer: Option<&SignerIdentity>,
    diff: &PermissionDiff,
) -> Result<(), CallError> {
    if signer_changed && !options.approve_signer_change {
        let found = verified_signer
            .expect("invariant: signer_changed is only true when a verified signer was found");
        return Err(signer_changed_error(
            name,
            previous_signer.unwrap_or_default(),
            found,
            digest,
        ));
    }
    if diff.requires_approval && !options.approve {
        return Err(permission_approval_required(name, diff));
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "commits runtime, store replacement and lockfile update as one step"
)]
async fn commit_staged_update(
    staged: plugin_install::StagedPlugin,
    reference: &Reference,
    name: &str,
    digest: &str,
    verified_signer: Option<&SignerIdentity>,
    pins: &mut goodfetch_protocol::plugin::lockfile::PluginLockfile,
    store: &PluginStore,
    lockfile: &Path,
    (cache, catalog, now): (&BrowserCache, &RuntimeCatalog, jiff::Timestamp),
    (fetching, blocking): (&Fetching, &BlockingThreads),
) -> Result<Installed, CallError> {
    let runtime = staged.manifest().runtime.clone();
    let version = staged.manifest().version.clone();
    let _held_runtime = plugin_runtime::ensure_installed(
        &runtime,
        (name, &version),
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
        .commit_replace(blocking, store)
        .await
        .map_err(|error| plugin_add::refusal(error, reference, store))?;
    pins.replace(LockedPlugin {
        name: name.to_owned(),
        version: held.manifest.version.clone(),
        digest: digest.to_owned(),
        signer: verified_signer.map(ToString::to_string),
        approvals: None,
    });
    let (path, text) = (lockfile.to_owned(), plugin_lockfile::text(pins));
    blocking
        .run(move || plugin_add::write_lockfile(&path, &text))
        .await
        .map_err(|error| plugin_add::unwritten(&error, lockfile, name))?;
    Ok(held)
}

#[expect(
    clippy::too_many_arguments,
    reason = "packs the fields of PluginUpdated from the update stages"
)]
fn build_answer(
    installed: &Installed,
    new_version: &str,
    previous_digest: Option<String>,
    digest: String,
    lockfile: &Path,
    previous_signer: Option<String>,
    signer: Option<SignerIdentity>,
    signer_changed: bool,
    diff: PermissionDiff,
    applied: bool,
) -> PluginUpdated {
    PluginUpdated {
        name: installed.manifest.name.clone(),
        previous_version: installed.manifest.version.clone(),
        version: new_version.to_owned(),
        previous_digest,
        digest,
        lockfile: lockfile.display().to_string(),
        previous_signer,
        signer,
        signer_changed,
        diff,
        applied,
        approval: None,
    }
}

async fn read_installed(
    store: &PluginStore,
    name: &str,
    blocking: &BlockingThreads,
) -> Result<Installed, CallError> {
    let store = store.clone();
    let plugin_name = name.to_owned();
    blocking
        .run(move || store.installed(&plugin_name))
        .await
        .map_err(|error| match error {
            StoreError::NotInstalled(_) | StoreError::Versions { versions: 0, .. } => {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!(
                        "the Plugin `{name}` is not installed: install it with `plugin add` first"
                    ),
                )
            }
            other => CallError::protocol(
                ProtocolFailure::InternalError,
                format!("the Plugin store cannot be read: {other}"),
            ),
        })
}

/// Verifies the target release's provenance attestation and detects whether
/// its verified signer identity changed from `previous_signer`.
async fn check_update_provenance(
    reference: &Reference,
    name: &str,
    digest: &str,
    previous_signer: Option<&str>,
    provenance: &ProvenancePolicy<'_>,
    fetching: &Fetching,
    store: &PluginStore,
) -> Result<(Option<SignerIdentity>, bool), CallError> {
    let policy_allows = provenance.settings.is_some_and(|settings| {
        plugin_provenance::policy_allows_exact_digest(settings, name, digest)
    });
    if policy_allows && provenance.index.is_none() && previous_signer.is_none() {
        return Ok((None, false));
    }
    let expected_signer = provenance
        .index
        .and_then(|idx| idx.entry(name))
        .map(|entry| &entry.signer);
    let origin = provenance
        .index
        .map_or(Origin::ThirdParty, |idx| idx.origin_of(name, Some(digest)));
    let Some(attestations) = plugin_add::fetch_attestations_or_allow(
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
        return Ok((None, false));
    };
    match plugin_provenance::verify_attestations(
        &attestations,
        digest,
        expected_signer,
        provenance.trust,
    ) {
        AttestationVerdict::Verified(signer) => {
            let changed = previous_signer.is_some_and(|prev| prev != signer.to_string());
            Ok((Some(signer), changed))
        }
        AttestationVerdict::Refused {
            reason: AttestationFailureReason::SignerMismatch,
            found_signer: Some(found),
            expected_signer: expected,
        } => {
            if let Some(prev) = previous_signer {
                let changed = prev != found.to_string();
                return Ok((Some(found), changed));
            }
            if policy_allows {
                return Ok((Some(found), false));
            }
            Err(plugin_provenance::attestation_invalid_error(
                name,
                digest,
                AttestationFailureReason::SignerMismatch,
                expected,
                Some(found),
            ))
        }
        AttestationVerdict::Refused {
            reason,
            expected_signer: expected,
            found_signer,
        } => {
            if policy_allows
                || (origin == Origin::FirstParty && reason == AttestationFailureReason::Missing)
            {
                return Ok((None, false));
            }
            Err(plugin_provenance::attestation_invalid_error(
                name,
                digest,
                reason,
                expected,
                found_signer,
            ))
        }
    }
}

fn signer_changed_error(
    name: &str,
    previous_signer: &str,
    found_signer: &SignerIdentity,
    digest: &str,
) -> CallError {
    let details = PluginSignerChangedDetails {
        name: name.to_owned(),
        previous_signer: previous_signer.to_owned(),
        found_signer: found_signer.clone(),
        digest: digest.to_owned(),
        fix: format!(
            "Review the new signer `{found_signer}` (previously `{previous_signer}`) and run `plugin update` interactively to approve the signer change."
        ),
    };
    CallError::new(
        &PLUGIN_SIGNER_CHANGED,
        format!(
            "The Plugin `{name}` release `{digest}` was signed by `{found_signer}`, which differs from the previously installed signer `{previous_signer}`: nothing was updated."
        ),
        &details,
    )
}

fn permission_approval_required(name: &str, diff: &PermissionDiff) -> CallError {
    let first = diff
        .added
        .first()
        .map(|s| s.permission.clone())
        .or_else(|| {
            diff.widened
                .iter()
                .find_map(|w| w.statement.as_ref().map(|s| s.permission.clone()))
        })
        .unwrap_or(Permission::Direct);
    let details = PluginPermissionDeniedDetails {
        plugin: name.to_owned(),
        fix: fix(name, first.field()),
        permission: first,
    };
    CallError::new(
        &PLUGIN_PERMISSION_DENIED,
        format!(
            "Updating the Plugin `{name}` adds or widens permissions ({} added, {} widened): approve the diff with `plugin update` before applying.",
            diff.added.len(),
            diff.widened.len()
        ),
        &details,
    )
}
