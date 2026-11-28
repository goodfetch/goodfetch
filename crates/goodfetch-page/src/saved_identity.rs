//! Saved identity operations and reuse checks (ADRs 0015, 0035, 0056, 0089,
//! 0092 and 0098; ticket #736): the one place that validates identity names,
//! converts store and crypto errors, enforces Access mode / proxy / machine
//! binding consistency on reuse, audits save/reuse/delete/export/expiry events,
//! and serves `identity.list`, `identity.import`, `identity.delete`,
//! `identity.export`, and `identity.export_storage_state`.
//!
//! Size: keeps all Saved identity store validation, error mapping, reuse
//! consistency checks, and `identity.*` handlers together in one boundary module.

use std::time::SystemTime;

use data_encoding::HEXLOWER;
use goodfetch_engine::audit::Origin;
use goodfetch_engine::run::secret_error;
use goodfetch_engine::run_key::KeyError;
use goodfetch_engine::run_state::RunStateError;
use goodfetch_engine::saved_identities::{
    LoadedSavedIdentity, NewSavedIdentity, SAVED_IDENTITY_STORE, StoreError,
};
use goodfetch_engine::session_bundle::{EXPORT_WORK_FACTOR, IMPORT_LIMITS, PublicKey};
use goodfetch_engine::session_transfer::{Recipient, TransferError, export, import};
use goodfetch_engine::{CallContext, CallError};
use goodfetch_identity_profile::IdentitySeed;
use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::SharedTypeValue;
use goodfetch_protocol::checkpoint::{
    FORMAT_VERSION_UNSUPPORTED, FormatVersionUnsupportedDetails, SESSION_BOUND_TO_MACHINE,
    SessionBoundToMachineDetails,
};
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::identity::{
    IdentityDeleteParams, IdentityDeleted, IdentityExportParams, IdentityExportStorageStateParams,
    IdentityExported, IdentityImportParams, IdentityList, IdentityStorageStateExported,
    SAVED_IDENTITY_DELETED, SAVED_IDENTITY_EXPORTED, SAVED_IDENTITY_MISMATCH,
    SAVED_IDENTITY_NOT_FOUND, SAVED_IDENTITY_REUSED, SAVED_IDENTITY_SAVED, SavedIdentityInfo,
    SavedIdentityMismatchDetails, SavedIdentityNotFoundDetails,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    RUN_KEY_MISMATCH, RUN_STATE_KEY_REFERENCE, RunKeyMismatchDetails, SESSION_EXPORT_OPEN_FAILED,
    SessionExportFailure, SessionExportOpenFailedDetails,
};
use goodfetch_protocol::session_export::{ExportedIdentity, SavedProxyBinding};
use goodfetch_protocol::settings::session_source::{SessionSource, StateFileFormat};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SESSION_SOURCE, SettingsLayer,
};
use secrecy::zeroize::Zeroizing;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::access_mode::{AccessMode, BrowserFamily};
use crate::cdp::driver::Carried;

/// Maximum length of a Saved identity name.
const MAX_NAME_LEN: usize = 128;

/// Loaded Saved identity ready to seed an opening Session.
pub(crate) struct LoadedForSession {
    pub(crate) name: String,
    pub(crate) access_mode: String,
    pub(crate) browser_family: String,
    pub(crate) identity_seed: IdentitySeed,
    pub(crate) locale: Option<String>,
    pub(crate) timezone_id: Option<String>,
    pub(crate) proxy_binding: Option<SavedProxyBinding>,
    pub(crate) machine_binding: Option<String>,
    pub(crate) carried: Carried,
}

/// Validates a Saved identity name: non-empty, <= 128 chars, no control
/// characters or path separators.
pub(crate) fn validate_name(name: &str) -> Result<String, CallError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "Saved identity `name` must not be empty",
        ));
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("Saved identity `name` exceeds {MAX_NAME_LEN} bytes"),
        ));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "Saved identity `name` must not contain control characters or path separators",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Canonical wire name for a [`BrowserFamily`].
pub(crate) const fn browser_family_str(family: BrowserFamily) -> &'static str {
    match family {
        BrowserFamily::Chromium => "chromium",
        BrowserFamily::Firefox => "firefox",
        BrowserFamily::WebKit => "webkit",
    }
}

/// Converts a [`StoreError`] to a [`CallError`], mapping `lowest_minor` /
/// format refusals to `format_version_unsupported`.
pub(crate) fn store_error(error: &StoreError) -> CallError {
    match error {
        StoreError::State(RunStateError::Format {
            format,
            found,
            compatibility,
            install,
            ..
        }) => {
            let fix = match (compatibility, install) {
                (Compatibility::NewerMinor, Some(version)) => {
                    format!("Install Goodfetch {version} or newer to use `{format}`.")
                }
                (Compatibility::OlderMajor, _) => {
                    "Run `upgrade-files` to copy the older major's files into this major."
                        .to_owned()
                }
                _ => format!("Upgrade Goodfetch to a version that reads `{format}` {found}."),
            };
            CallError::new(
                &FORMAT_VERSION_UNSUPPORTED,
                error.to_string(),
                &FormatVersionUnsupportedDetails {
                    format: (*format).to_owned(),
                    found: found.clone(),
                    supported: SAVED_IDENTITY_STORE.version.to_string(),
                    fix,
                },
            )
        }
        other => CallError::protocol(ProtocolFailure::InternalError, other.to_string()),
    }
}

/// Converts a [`KeyError`] from opening a Saved identity to `run_key_mismatch`
/// (inside a Run), `session_export_open_failed` (outside a Run), or `InternalError`.
fn open_key_error(wrapped_by: &str, run_id: Option<&RunId>, error: &KeyError) -> CallError {
    match error {
        KeyError::DoesNotOpen => {
            let message = format!(
                "The key-encryption key `{wrapped_by}` does not open the Saved identity: it was wrapped under another key or tampered with."
            );
            let fix = format!(
                "Set `{}` to the Secret reference that wrapped this store.",
                RUN_STATE_KEY_REFERENCE.name
            );
            if let Some(run_id) = run_id {
                CallError::new(
                    &RUN_KEY_MISMATCH,
                    message,
                    &RunKeyMismatchDetails {
                        run_id: run_id.clone(),
                        setting: RUN_STATE_KEY_REFERENCE.name.to_owned(),
                        reference: wrapped_by.to_owned(),
                        wrapped_by: wrapped_by.to_owned(),
                        fix,
                    },
                )
            } else {
                CallError::new(
                    &SESSION_EXPORT_OPEN_FAILED,
                    message,
                    &SessionExportOpenFailedDetails {
                        reference: wrapped_by.to_owned(),
                        failure: SessionExportFailure::WrongKey,
                        fix,
                    },
                )
            }
        }
        KeyError::NoRandomness(_) => {
            CallError::protocol(ProtocolFailure::InternalError, error.to_string())
        }
        _ => CallError::protocol(ProtocolFailure::InternalError, error.to_string()),
    }
}

fn not_found(name: &str) -> CallError {
    CallError::new(
        &SAVED_IDENTITY_NOT_FOUND,
        format!("No Saved identity named `{name}` exists in this Product major's store."),
        &SavedIdentityNotFoundDetails {
            name: name.to_owned(),
        },
    )
}

/// Records an Audit log entry for a Saved identity action.
pub(crate) async fn audit(
    context: &CallContext,
    run_id: Option<RunId>,
    kind: &'static SharedTypeValue,
    details: Value,
) -> Result<(), CallError> {
    let origin = Origin {
        actor: Actor::ClientSurface {
            surface: context.surface(),
        },
        surface: Some(context.surface()),
        layer: None,
        run_id,
    };
    let now = context.engine().clock().timestamp();
    context
        .engine()
        .audit()
        .record(&origin, kind, details, now)
        .await
        .map(drop)
        .map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("could not record `{}` in the Audit log: {error}", kind.name),
            )
        })
}

/// Records `saved_identity_deleted` (`reason: "expired"`) for every identity
/// expired during a store read/list.
pub(crate) async fn audit_expired(context: &CallContext, expired: &[String]) {
    if expired.is_empty() {
        return;
    }
    let origin = Origin {
        actor: Actor::Engine,
        surface: None,
        layer: None,
        run_id: None,
    };
    let now = context.engine().clock().timestamp();
    for name in expired {
        if let Err(error) = context
            .engine()
            .audit()
            .record(
                &origin,
                &SAVED_IDENTITY_DELETED,
                json!({ "name": name, "reason": "expired" }),
                now,
            )
            .await
        {
            tracing::warn!(%error, name, "could not audit expired Saved identity");
        }
    }
}

/// Refuses reusing a Saved identity under a different Access mode (ADR 0035).
pub(crate) fn check_access_mode(
    saved: &LoadedForSession,
    requested: AccessMode,
) -> Result<(), CallError> {
    let actual = requested.as_str();
    if saved.access_mode != actual {
        return Err(CallError::new(
            &SAVED_IDENTITY_MISMATCH,
            format!(
                "Saved identity `{}` was saved under Access mode `{}` and cannot be reused under `{actual}` (ADR 0035).",
                saved.name, saved.access_mode
            ),
            &SavedIdentityMismatchDetails {
                name: saved.name.clone(),
                field: "access_mode".to_owned(),
                expected: saved.access_mode.clone(),
                actual: actual.to_owned(),
                fix: format!(
                    "Open the Session with `browser.access_mode: \"{}\"` or open a fresh Session with `source: null`.",
                    saved.access_mode
                ),
            },
        ));
    }
    Ok(())
}

/// Refuses reusing a Saved identity with an inconsistent proxy binding
/// (ADR 0035).
pub(crate) fn check_proxy_binding(
    saved: &LoadedForSession,
    actual: Option<&SavedProxyBinding>,
) -> Result<(), CallError> {
    match (&saved.proxy_binding, actual) {
        (None, None) => Ok(()),
        (None, Some(actual)) => Err(CallError::new(
            &SAVED_IDENTITY_MISMATCH,
            format!(
                "Saved identity `{}` was saved with direct egress and cannot be reused through a proxy (ADR 0035).",
                saved.name
            ),
            &SavedIdentityMismatchDetails {
                name: saved.name.clone(),
                field: "proxy".to_owned(),
                expected: "direct".to_owned(),
                actual: describe_proxy(actual),
                fix: "Clear `proxy.server` / `proxy.provider` or open a fresh Session.".to_owned(),
            },
        )),
        (Some(expected), None) => Err(CallError::new(
            &SAVED_IDENTITY_MISMATCH,
            format!(
                "Saved identity `{}` was saved through proxy `{}` and cannot be reused with direct egress (ADR 0035).",
                saved.name,
                describe_proxy(expected)
            ),
            &SavedIdentityMismatchDetails {
                name: saved.name.clone(),
                field: "proxy".to_owned(),
                expected: describe_proxy(expected),
                actual: "direct".to_owned(),
                fix: format!(
                    "Configure a proxy matching `{}` or open a fresh Session.",
                    describe_proxy(expected)
                ),
            },
        )),
        (Some(expected), Some(actual)) => {
            if proxy_bindings_consistent(expected, actual) {
                Ok(())
            } else {
                Err(CallError::new(
                    &SAVED_IDENTITY_MISMATCH,
                    format!(
                        "Saved identity `{}` was saved with proxy `{}` which is inconsistent with `{}` (ADR 0035).",
                        saved.name,
                        describe_proxy(expected),
                        describe_proxy(actual)
                    ),
                    &SavedIdentityMismatchDetails {
                        name: saved.name.clone(),
                        field: "proxy".to_owned(),
                        expected: describe_proxy(expected),
                        actual: describe_proxy(actual),
                        fix: format!(
                            "Use a proxy consistent with `{}` or open a fresh Session.",
                            describe_proxy(expected)
                        ),
                    },
                ))
            }
        }
    }
}

/// Checks machine binding for a direct-egress Saved identity or export bundle
/// (ADR 0056).
pub(crate) async fn check_machine_binding(
    machine_binding: Option<&str>,
    has_proxy: bool,
    run_id: &RunId,
    context: &CallContext,
) -> Result<(), CallError> {
    if has_proxy {
        return Ok(());
    }
    let Some(bound) = machine_binding else {
        return Ok(());
    };
    let current =
        context.runs().machine().await.map_err(|error| {
            CallError::protocol(ProtocolFailure::InternalError, error.to_string())
        })?;
    if bound != current {
        return Err(CallError::new(
            &SESSION_BOUND_TO_MACHINE,
            format!(
                "This Saved identity is bound to machine `{bound}` and cannot be reused on `{current}` over direct egress (ADR 0056)."
            ),
            &SessionBoundToMachineDetails {
                run_id: run_id.clone(),
            },
        ));
    }
    Ok(())
}

fn describe_proxy(binding: &SavedProxyBinding) -> String {
    let mut parts = Vec::new();
    if let Some(provider) = &binding.provider {
        parts.push(format!("provider={provider}"));
    }
    if let Some(kind) = &binding.kind {
        parts.push(format!("kind={}", kind.as_str()));
    }
    if let Some(country) = &binding.country {
        parts.push(format!("country={country}"));
    }
    if let Some(server) = &binding.server {
        parts.push(format!("server={server}"));
    }
    if parts.is_empty() {
        "proxy".to_owned()
    } else {
        parts.join(", ")
    }
}

fn proxy_bindings_consistent(expected: &SavedProxyBinding, actual: &SavedProxyBinding) -> bool {
    // Provider-backed proxies match on provider (when both set), country, and kind (ADR 0035).
    if expected.provider.is_some() || expected.country.is_some() || expected.kind.is_some() {
        if let (Some(exp_p), Some(act_p)) = (&expected.provider, &actual.provider)
            && exp_p != act_p
        {
            return false;
        }
        if let Some(exp_c) = &expected.country
            && actual.country.as_deref() != Some(exp_c.as_str())
        {
            return false;
        }
        if let Some(exp_k) = expected.kind
            && actual.kind != Some(exp_k)
        {
            return false;
        }
        if let Some(exp_s) = &expected.server
            && expected.provider.is_none()
            && actual.server.as_deref() != Some(exp_s.as_str())
        {
            return false;
        }
        return true;
    }
    // Direct `proxy.server` binding matches exact server scheme://host:port.
    expected.server.is_some() && expected.server == actual.server
}

/// Loads and decrypts `name` from the per-major Saved identity store for
/// Session reuse, touching `last_used_at` and pruning expired entries.
pub(crate) async fn load_for_session(
    name: &str,
    run_id: &RunId,
    context: &CallContext,
) -> Result<LoadedForSession, CallError> {
    let name = validate_name(name)?;
    let now = SystemTime::from(context.engine().clock().timestamp());
    let max_unused_age = context.runs().saved_identity_max_unused_age().await;
    let (sealed, expired) = context
        .engine()
        .saved_identities()
        .load_sealed(name.clone(), now, max_unused_age, true)
        .await
        .map_err(|error| store_error(&error))?;
    audit_expired(context, &expired).await;
    let sealed = sealed.ok_or_else(|| not_found(&name))?;
    let key_reference = sealed.key_reference.clone();
    let kek = context
        .runs()
        .saved_identity_open_kek(&key_reference, Some(run_id))
        .await
        .map_err(|error| secret_error(&error))?;
    let loaded = sealed
        .open(&kek)
        .map_err(|error| open_key_error(&key_reference, Some(run_id), &error))?;
    let carried = Carried::from_bytes(&loaded.payload_plaintext).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("Saved identity `{name}` payload could not be decoded: {error}"),
        )
    })?;
    Ok(LoadedForSession {
        name: loaded.info.name,
        access_mode: loaded.info.access_mode,
        browser_family: loaded.info.browser_family,
        identity_seed: IdentitySeed(loaded.identity_seed),
        locale: loaded.info.locale,
        timezone_id: loaded.info.timezone_id,
        proxy_binding: loaded.proxy_binding,
        machine_binding: loaded.machine_binding,
        carried,
    })
}

/// Records `saved_identity_reused` in the Audit log when a Session opens from
/// a Saved identity.
pub(crate) async fn audit_reused(
    saved: &LoadedForSession,
    run_id: &RunId,
    context: &CallContext,
) -> Result<(), CallError> {
    let (cookie_count, origin_count) = saved.carried.counts();
    audit(
        context,
        Some(run_id.clone()),
        &SAVED_IDENTITY_REUSED,
        json!({
            "name": saved.name,
            "access_mode": saved.access_mode,
            "cookie_count": cookie_count,
            "origin_count": origin_count,
        }),
    )
    .await
}

/// Handles `identity.list`.
pub(crate) async fn list(context: &CallContext) -> Result<IdentityList, CallError> {
    let now = SystemTime::from(context.engine().clock().timestamp());
    let max_unused_age = context.runs().saved_identity_max_unused_age().await;
    let (identities, expired) = context
        .engine()
        .saved_identities()
        .list(now, max_unused_age)
        .await
        .map_err(|error| store_error(&error))?;
    audit_expired(context, &expired).await;
    Ok(IdentityList { identities })
}

/// Handles `identity.delete`.
pub(crate) async fn delete(
    params: IdentityDeleteParams,
    context: &CallContext,
) -> Result<IdentityDeleted, CallError> {
    let name = validate_name(&params.name)?;
    let deleted = context
        .engine()
        .saved_identities()
        .delete(name.clone())
        .await
        .map_err(|error| store_error(&error))?;
    if deleted {
        audit(
            context,
            None,
            &SAVED_IDENTITY_DELETED,
            json!({ "name": name, "reason": "deleted" }),
        )
        .await?;
    }
    Ok(IdentityDeleted { name, deleted })
}

/// Handles `identity.import` (`state_file`, `cookie_header`, `installed_browser`,
/// and `profile_directory`, ADRs 0035, 0056).
pub(crate) async fn import_identity(
    params: IdentityImportParams,
    launcher: &crate::cdp::launch::Launcher,
    context: &CallContext,
) -> Result<SavedIdentityInfo, CallError> {
    let name = validate_name(&params.name)?;
    let requested_mode = params.access_mode.as_deref().unwrap_or("standard");
    if requested_mode == "identified" {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "`identified` Access mode is refused at import: a person's login is not a declared agent's (ADR 0056)",
        ));
    }
    if !matches!(requested_mode, "standard" | "stealth") {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("invalid `access_mode` `{requested_mode}`: expected `standard` or `stealth`"),
        ));
    }
    let source_kind = params.source.kind().to_owned();
    let read = read_import_source(params.source, launcher, context).await?;

    let carried = Carried::from_imported(read.state);
    let (cookie_count, origin_count) = carried.counts();
    let domains = carried.domains();
    let payload_bytes = carried.to_bytes().map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("could not serialize imported identity state: {error}"),
        )
    })?;

    // A session export bundle keeps its Saved identity's Access mode, seed,
    // proxy binding, and machine binding unchanged (ADR 0056).
    let meta = resolve_import_meta(
        requested_mode,
        read.bundle_identity,
        ImportOverrides {
            browser_family: read.browser_family,
            locale: params.locale.or(read.locale),
            timezone_id: params.timezone_id.or(read.timezone_id),
            proxy_server: params.proxy_server,
            proxy_country: params.proxy_country,
            proxy_kind: params.proxy_kind,
        },
        context,
    )
    .await?;

    let (key_ref, kek) = context
        .runs()
        .saved_identity_save_kek(None)
        .await
        .map_err(|error| secret_error(&error))?;
    let now = SystemTime::from(context.engine().clock().timestamp());
    let max_unused_age = context.runs().saved_identity_max_unused_age().await;
    let mut info = context
        .engine()
        .saved_identities()
        .save(
            NewSavedIdentity {
                name: name.clone(),
                access_mode: meta.access_mode.clone(),
                identity_seed: meta.identity_seed,
                browser_family: meta.browser_family,
                user_agent: meta.user_agent,
                locale: meta.locale,
                timezone_id: meta.timezone_id,
                proxy_binding: meta.proxy_binding,
                machine_binding: meta.machine_binding,
                preserve_existing_machine_binding: false,
                was_lent: meta.was_lent,
                source_kind: Some(source_kind.clone()),
                domains,
                cookie_count,
                origin_count,
                payload_plaintext: Zeroizing::new(payload_bytes),
            },
            key_ref.to_string(),
            kek,
            now,
            max_unused_age,
        )
        .await
        .map_err(|error| store_error(&error))?;

    info.warnings = read.warnings;

    audit(
        context,
        None,
        &SAVED_IDENTITY_SAVED,
        json!({
            "name": name,
            "source_kind": source_kind,
            "access_mode": meta.access_mode,
            "cookie_count": cookie_count,
            "origin_count": origin_count,
            "machine_bound": info.machine_bound,
            "proxy_bound": info.proxy_bound,
            "was_lent": info.was_lent,
        }),
    )
    .await?;

    Ok(info)
}

struct ImportMeta {
    access_mode: String,
    identity_seed: u64,
    browser_family: String,
    user_agent: Option<String>,
    locale: Option<String>,
    timezone_id: Option<String>,
    proxy_binding: Option<SavedProxyBinding>,
    machine_binding: Option<String>,
    was_lent: bool,
}

struct ImportOverrides {
    browser_family: Option<String>,
    locale: Option<String>,
    timezone_id: Option<String>,
    proxy_server: Option<String>,
    proxy_country: Option<String>,
    proxy_kind: Option<goodfetch_protocol::page::proxy::ProxyKind>,
}

struct ReadImportSource {
    state: crate::state_import::ImportedState,
    bundle_identity: Option<ExportedIdentity>,
    browser_family: Option<String>,
    locale: Option<String>,
    timezone_id: Option<String>,
    warnings: Vec<String>,
}

async fn resolve_import_meta(
    requested_mode: &str,
    bundle_identity: Option<ExportedIdentity>,
    overrides: ImportOverrides,
    context: &CallContext,
) -> Result<ImportMeta, CallError> {
    if let Some(bundle_id) = bundle_identity {
        return Ok(ImportMeta {
            access_mode: bundle_id.access_mode,
            identity_seed: bundle_id.identity_seed,
            browser_family: bundle_id.browser_family,
            user_agent: bundle_id.user_agent,
            locale: bundle_id.locale,
            timezone_id: bundle_id.timezone_id,
            proxy_binding: bundle_id.proxy_binding,
            machine_binding: bundle_id.machine_binding,
            was_lent: bundle_id.was_lent,
        });
    }
    let ImportOverrides {
        browser_family,
        locale,
        timezone_id,
        proxy_server,
        proxy_country,
        proxy_kind,
    } = overrides;
    let proxy_binding = (proxy_server.is_some() || proxy_country.is_some() || proxy_kind.is_some())
        .then_some(SavedProxyBinding {
            server: proxy_server,
            provider: None,
            country: proxy_country,
            kind: proxy_kind,
            sticky_session: None,
        });
    let machine_binding = if proxy_binding.is_none() {
        let machine = context.runs().machine().await.map_err(|error| {
            CallError::protocol(ProtocolFailure::InternalError, error.to_string())
        })?;
        Some(machine)
    } else {
        None
    };
    let seed = context.engine().draw_u64();
    let tz = timezone_id.or_else(|| Some(context.engine().host_zone().to_owned()));
    Ok(ImportMeta {
        access_mode: requested_mode.to_owned(),
        identity_seed: seed,
        browser_family: browser_family.unwrap_or_else(|| "chromium".to_owned()),
        user_agent: None,
        locale,
        timezone_id: tz,
        proxy_binding,
        machine_binding,
        was_lent: false,
    })
}

async fn read_import_source(
    source: SessionSource,
    launcher: &crate::cdp::launch::Launcher,
    context: &CallContext,
) -> Result<ReadImportSource, CallError> {
    match source {
        SessionSource::StateFile {
            path,
            format,
            key,
            domains,
        } => {
            read_state_file_source(&path, format, key.as_deref(), domains.as_deref(), context).await
        }
        SessionSource::CookieHeader { header, domain } => {
            let state = crate::state_import::parse_cookie_header(&header, &domain)
                .map_err(|e| CallError::protocol(ProtocolFailure::InvalidParams, e.to_string()))?;
            Ok(ReadImportSource {
                state,
                bundle_identity: None,
                browser_family: None,
                locale: None,
                timezone_id: None,
                warnings: Vec::new(),
            })
        }
        SessionSource::InstalledBrowser {
            browser,
            chromium_profile,
            domains,
            path,
            executable,
        } => {
            read_chromium_import_source(
                crate::cdp::import::ChromiumImportRequest {
                    browser,
                    path: path.map(std::path::PathBuf::from),
                    chromium_profile,
                    executable: executable.map(std::path::PathBuf::from),
                    domains,
                    layer: SettingsLayer::Run,
                },
                launcher,
                context,
            )
            .await
        }
        SessionSource::ProfileDirectory {
            path,
            browser,
            chromium_profile,
            domains,
        } => {
            read_chromium_import_source(
                crate::cdp::import::ChromiumImportRequest {
                    browser,
                    path: Some(std::path::PathBuf::from(path)),
                    chromium_profile,
                    executable: None,
                    domains,
                    layer: SettingsLayer::Run,
                },
                launcher,
                context,
            )
            .await
        }
        other => {
            let kind = other.kind();
            Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "This engine does not yet support `identity.import` from `{kind}`; use `state_file`, `cookie_header`, `installed_browser`, or `profile_directory`."
                ),
                &CapabilityUnavailableDetails {
                    key: SESSION_SOURCE.name.to_owned(),
                    layer: SettingsLayer::Run,
                    kind: kind.to_owned(),
                    fix: "`state_file`".to_owned(),
                },
            ))
        }
    }
}

async fn read_state_file_source(
    path: &str,
    format: Option<StateFileFormat>,
    key: Option<&str>,
    domains: Option<&[String]>,
    context: &CallContext,
) -> Result<ReadImportSource, CallError> {
    let bytes = read_file_capped(path, context).await?;
    let detected = format.unwrap_or_else(|| crate::state_import::detect_format(&bytes));
    let mut bundle_identity = None;
    let state = match detected {
        StateFileFormat::StorageState => crate::state_import::parse_storage_state(&bytes)
            .map_err(|e| CallError::protocol(ProtocolFailure::InvalidParams, e.to_string()))?,
        StateFileFormat::CookiesTxt => crate::state_import::parse_cookies_txt(&bytes)
            .map_err(|e| CallError::protocol(ProtocolFailure::InvalidParams, e.to_string()))?,
        StateFileFormat::CdpCookies => crate::state_import::parse_cdp_cookies(&bytes)
            .map_err(|e| CallError::protocol(ProtocolFailure::InvalidParams, e.to_string()))?,
        StateFileFormat::SessionExport => {
            let Some(key_str) = key else {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    "importing a `session_export` bundle requires `key` naming the age identity or passphrase Secret reference",
                ));
            };
            let reference = SecretReference::parse(key_str).map_err(|e| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("`key` is not a valid Secret reference: {e}"),
                )
            })?;
            let origin = Origin {
                actor: Actor::ClientSurface {
                    surface: context.surface(),
                },
                surface: Some(context.surface()),
                layer: None,
                run_id: None,
            };
            let resolving = context.runs().outside_a_run_secrets();
            let exported = import(bytes, &reference, IMPORT_LIMITS, &origin, &resolving)
                .await
                .map_err(|error| transfer_error(&error))?;
            bundle_identity.clone_from(&exported.identity);
            crate::state_import::from_session_export(exported)
        }
    };
    let filtered = crate::state_import::filter_domains(state, domains.unwrap_or(&[]));
    Ok(ReadImportSource {
        state: filtered,
        bundle_identity,
        browser_family: None,
        locale: None,
        timezone_id: None,
        warnings: Vec::new(),
    })
}

async fn read_chromium_import_source(
    request: crate::cdp::import::ChromiumImportRequest,
    launcher: &crate::cdp::launch::Launcher,
    context: &CallContext,
) -> Result<ReadImportSource, CallError> {
    let outcome = crate::cdp::import::read_chromium_browser(
        request,
        launcher,
        context.engine().tasks(),
        goodfetch_counted_work::CountedWork::default(),
        context.engine().host_zone(),
    )
    .await?;
    let mut warnings: Vec<String> = outcome.warnings.into_iter().map(|w| w.message).collect();
    if let Some(mismatch) = crate::cdp::import::family_mismatch_warning(
        outcome.source_family,
        crate::cdp::import::default_running_browser_family(),
    ) {
        warnings.push(mismatch.message);
    }
    Ok(ReadImportSource {
        state: outcome.state,
        bundle_identity: None,
        browser_family: Some(outcome.source_family.to_owned()),
        locale: outcome.locale,
        timezone_id: outcome.timezone_id,
        warnings,
    })
}

/// Handles `identity.export`: encrypts the Saved identity into ADR 0015's
/// `age`-encrypted `session-export` bundle.
pub(crate) async fn export_bundle(
    params: IdentityExportParams,
    context: &CallContext,
) -> Result<IdentityExported, CallError> {
    let name = validate_name(&params.name)?;
    let recipient = match (params.public_key.as_deref(), params.passphrase.as_deref()) {
        (Some(pk), None) => {
            let key: PublicKey = pk.parse().map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `public_key`: {error}"),
                )
            })?;
            Recipient::PublicKey(key)
        }
        (None, Some(passphrase)) => {
            let reference = SecretReference::parse(passphrase).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("`passphrase` must be a Secret reference: {error}"),
                )
            })?;
            Recipient::Passphrase {
                reference,
                work_factor: EXPORT_WORK_FACTOR,
            }
        }
        (Some(_), Some(_)) => {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "specify either `public_key` or `passphrase`, not both",
            ));
        }
        (None, None) => {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                "specify either `public_key` (`age1…`) or `passphrase` (a Secret reference)",
            ));
        }
    };

    let loaded = load_for_export(&name, context).await?;
    let carried = Carried::from_bytes(&loaded.payload_plaintext).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("Saved identity `{name}` payload could not be decoded: {error}"),
        )
    })?;
    let (cookie_count, origin_count) = carried.counts();
    let exported_identity = ExportedIdentity {
        access_mode: loaded.info.access_mode.clone(),
        identity_seed: loaded.identity_seed,
        browser_family: loaded.info.browser_family.clone(),
        user_agent: loaded.user_agent.clone(),
        locale: loaded.info.locale.clone(),
        timezone_id: loaded.info.timezone_id.clone(),
        proxy_binding: loaded.proxy_binding.clone(),
        machine_binding: loaded.machine_binding.clone(),
        was_lent: loaded.info.was_lent,
    };
    let session_export = carried.to_session_export(Some(exported_identity));
    let origin = Origin {
        actor: Actor::ClientSurface {
            surface: context.surface(),
        },
        surface: Some(context.surface()),
        layer: None,
        run_id: None,
    };
    let resolving = context.runs().outside_a_run_secrets();
    let bundle_bytes = export(session_export, recipient, &origin, &resolving)
        .await
        .map_err(|error| transfer_error(&error))?;
    let digest = HEXLOWER.encode(&Sha256::digest(&bundle_bytes));
    let bundle = String::from_utf8(bundle_bytes).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("ASCII-armored bundle was not UTF-8: {error}"),
        )
    })?;

    audit(
        context,
        None,
        &SAVED_IDENTITY_EXPORTED,
        json!({
            "name": name,
            "format": "session_export",
            "digest": digest,
            "cookie_count": cookie_count,
            "origin_count": origin_count,
        }),
    )
    .await?;

    Ok(IdentityExported {
        name,
        bundle,
        digest,
        cookie_count,
        origin_count,
    })
}

/// Handles `identity.export_storage_state` (`Reach::PersonOnly`, ADR 0056).
pub(crate) async fn export_storage_state(
    params: IdentityExportStorageStateParams,
    context: &CallContext,
) -> Result<IdentityStorageStateExported, CallError> {
    let name = validate_name(&params.name)?;
    let loaded = load_for_export(&name, context).await?;
    let carried = Carried::from_bytes(&loaded.payload_plaintext).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("Saved identity `{name}` payload could not be decoded: {error}"),
        )
    })?;
    let (cookie_count, origin_count) = carried.counts();
    let storage_state = carried.to_storage_state_json().map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("could not encode Playwright storageState JSON: {error}"),
        )
    })?;

    audit(
        context,
        None,
        &SAVED_IDENTITY_EXPORTED,
        json!({
            "name": name,
            "format": "storage_state",
            "cookie_count": cookie_count,
            "origin_count": origin_count,
        }),
    )
    .await?;

    Ok(IdentityStorageStateExported {
        name,
        storage_state,
        cookie_count,
        origin_count,
    })
}

async fn load_for_export(
    name: &str,
    context: &CallContext,
) -> Result<LoadedSavedIdentity, CallError> {
    let now = SystemTime::from(context.engine().clock().timestamp());
    let max_unused_age = context.runs().saved_identity_max_unused_age().await;
    let (sealed, expired) = context
        .engine()
        .saved_identities()
        .load_sealed(name.to_owned(), now, max_unused_age, false)
        .await
        .map_err(|error| store_error(&error))?;
    audit_expired(context, &expired).await;
    let sealed = sealed.ok_or_else(|| not_found(name))?;
    let key_reference = sealed.key_reference.clone();
    let kek = context
        .runs()
        .saved_identity_open_kek(&key_reference, None)
        .await
        .map_err(|error| secret_error(&error))?;
    sealed
        .open(&kek)
        .map_err(|error| open_key_error(&key_reference, None, &error))
}

fn transfer_error(error: &TransferError) -> CallError {
    match error {
        TransferError::Secret(secret) => secret_error(secret),
        TransferError::Open(open) => open.to_call_error(),
        TransferError::Unrecorded { source, .. } => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("could not record session transfer in the Audit log: {source}"),
        ),
    }
}

async fn read_file_capped(path: &str, context: &CallContext) -> Result<Vec<u8>, CallError> {
    context
        .engine()
        .saved_identities()
        .read_file_capped(
            path.to_owned(),
            crate::state_import::MAX_STATE_FILE_BYTES as u64,
        )
        .await
        .map_err(|message| CallError::protocol(ProtocolFailure::InvalidParams, message))
}
