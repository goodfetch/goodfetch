//! A person's answers to a `wall_refetch_stopped` item (ADR 0052):
//! `needs_you.remove_refetch_domain` removes the domain from the machine's
//! layer of `wall.refetch_domains` that lists it, user configuration or
//! else the Operator policy, or replaces `*` there with the list the
//! person gives when only `*` covers it; `needs_you.deny_wall_domain` adds
//! the domain to the Operator policy's `wall.denied_domains` with its
//! reason. Each writes one file in one call, an edit of the settings
//! editor (`settings_editor`), every other byte kept, once the resolver
//! accepts the file so changed, refuses a managed Operator
//! policy with `setting_locked`, and records the answer in the Audit log.
//! A domain only project configuration lists is changed in its file: the
//! engine never writes a project's file. On a Daemon with accounts both
//! need the operator role, and denial step-up (the accounts and step-up
//! tickets); until then the caller is the operator.

use std::sync::Arc;

use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::needs_you::{
    NeedsYouDenyWallDomainParams, NeedsYouRemoveRefetchDomainParams, REFETCH_DOMAIN_REMOVED,
    RefetchDomainRemoved, WALL_DOMAIN_DENIED, WallDomainDenied,
};
use goodfetch_protocol::settings::{
    SettingLimit, SettingsLayer, WALL_DENIED_DOMAINS, WALL_REFETCH_DOMAINS,
};
use serde_json::{Value, json};

use super::{Refused, consent_domain, unrecorded};
use crate::call_error::CallError;
use crate::composition::CallContext;
use crate::host_name::lookup_key;
use crate::needs_you::refetch::{coverage, machine_listing};
use crate::run::settings_error;
use crate::settings::SettingsError;
use crate::settings_editor::{Change, Draft, SettingsEditor};

/// A registrable domain, never `*`, as `wall.refetch_domains` and
/// `wall.denied_domains` list it.
fn registrable(domain: &str) -> Result<String, CallError> {
    if domain.trim() == "*" {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "`*` names no domain: give a registrable domain, such as `example.com`".to_owned(),
        ));
    }
    consent_domain(domain)
}

/// Sets `setting` to `domains` in `table` of `layer`'s file, refusing a
/// managed Operator policy with `setting_locked`.
fn write(
    draft: &mut Draft<'_>,
    layer: SettingsLayer,
    table: &str,
    setting: &Setting,
    domains: &[String],
) -> Result<(), Refused> {
    let value = Value::Array(domains.iter().map(|domain| json!(domain)).collect());
    draft.set(layer, table, setting.name, &value)?;
    let policy = &draft.machine()?.operator_policy;
    if layer == SettingsLayer::OperatorPolicy && policy.managed.is_some() {
        return Err(Refused::Settings(SettingsError::Locked {
            key: setting.name.to_owned(),
            layer,
            limit: SettingLimit {
                key: setting.name.to_owned(),
                value: policy
                    .lock
                    .get(setting.name)
                    .or_else(|| policy.settings.get(setting.name))
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                layer,
            },
            fix: "the Operator policy is managed: change it where it is managed".to_owned(),
        }));
    }
    Ok(())
}

/// The call error of a write that refused.
fn refused(refused: Refused, context: &CallContext) -> CallError {
    match refused {
        Refused::Settings(error) => settings_error(
            &context.engine().settings().files().named_in(error),
            context,
        ),
        Refused::File(error) => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("a settings file cannot be written: {error}"),
        ),
        Refused::Params(message) => CallError::protocol(ProtocolFailure::InvalidParams, message),
        Refused::Unrecorded(error) => unrecorded(&error),
    }
}

pub(super) async fn remove_refetch_domain(
    params: NeedsYouRemoveRefetchDomainParams,
    context: CallContext,
) -> Result<RefetchDomainRemoved, CallError> {
    let domain = registrable(&params.domain)?;
    let instead = params
        .instead_of_star
        .map(|domains| {
            domains
                .iter()
                .map(|listed| registrable(listed))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let editor = SettingsEditor::resolving(&context).await;
    let owner = Arc::clone(context.engine().owner());
    owner
        .blocking(move || editor.edit(|draft| remove(draft, &domain, instead)))
        .await
        .map_err(|error| refused(error, &context))
}

/// Removes `domain` from the machine's layer of `wall.refetch_domains`,
/// or replaces `*` there with `instead`. Blocking.
fn remove(
    draft: &mut Draft<'_>,
    domain: &str,
    instead: Option<Vec<String>>,
) -> Result<Change<RefetchDomainRemoved>, Refused> {
    let unchanged = || RefetchDomainRemoved {
        domain: domain.to_owned(),
        removed: false,
        layer: None,
        domains: Vec::new(),
    };
    let Some(listing) = machine_listing(draft.machine()?) else {
        return Ok(Change::Kept(unchanged()));
    };
    let domains: Vec<String> = match coverage(&listing.domains, domain) {
        None => return Ok(Change::Kept(unchanged())),
        Some(false) => listing
            .domains
            .iter()
            .filter(|listed| lookup_key(listed) != domain)
            .cloned()
            .collect(),
        Some(true) => {
            let Some(instead) = instead else {
                return Err(Refused::Params(format!(
                    "only `*` in {} covers `{domain}`: give the domains that replace `*` there in `instead_of_star`",
                    listing.layer.as_str()
                )));
            };
            if instead.iter().any(|listed| listed == domain) {
                return Err(Refused::Params(format!(
                    "`instead_of_star` lists `{domain}`, the domain to remove"
                )));
            }
            let mut domains = Vec::new();
            for listed in listing.domains.iter().flat_map(|listed| {
                if listed == "*" {
                    instead.clone()
                } else {
                    vec![listed.clone()]
                }
            }) {
                if !domains.contains(&listed) {
                    domains.push(listed);
                }
            }
            domains
        }
    };
    write(
        draft,
        listing.layer,
        listing.table,
        &WALL_REFETCH_DOMAINS,
        &domains,
    )?;
    Ok(Change::Made {
        details: json!({ "domain": domain, "domains": domains }),
        answer: RefetchDomainRemoved {
            domain: domain.to_owned(),
            removed: true,
            layer: Some(listing.layer),
            domains,
        },
        layer: listing.layer,
        kind: &REFETCH_DOMAIN_REMOVED,
    })
}

pub(super) async fn deny_wall_domain(
    params: NeedsYouDenyWallDomainParams,
    context: CallContext,
) -> Result<WallDomainDenied, CallError> {
    let domain = registrable(&params.domain)?;
    let reason = params.reason.trim().to_owned();
    if reason.is_empty() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "a denial needs its reason, such as the site's objection".to_owned(),
        ));
    }
    let editor = SettingsEditor::resolving(&context).await;
    let owner = Arc::clone(context.engine().owner());
    owner
        .blocking(move || editor.edit(|draft| deny(draft, domain, reason)))
        .await
        .map_err(|error| refused(error, &context))
}

/// Adds `domain` to the Operator policy's `wall.denied_domains`, in its
/// lock when that sets the key, with `reason`. Blocking.
fn deny(
    draft: &mut Draft<'_>,
    domain: String,
    reason: String,
) -> Result<Change<WallDomainDenied>, Refused> {
    let policy = &draft.machine()?.operator_policy;
    let key = WALL_DENIED_DOMAINS.name;
    let (table, listed) = match policy.lock.get(key) {
        Some(listed) => ("lock", Some(listed)),
        None => ("settings", policy.settings.get(key)),
    };
    let mut domains: Vec<String> = listed
        .and_then(Value::as_array)
        .map(|listed| {
            listed
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if domains.iter().any(|listed| lookup_key(listed) == domain) {
        return Ok(Change::Kept(WallDomainDenied {
            domain,
            added: false,
            reason,
        }));
    }
    domains.push(domain.clone());
    let layer = SettingsLayer::OperatorPolicy;
    write(draft, layer, table, &WALL_DENIED_DOMAINS, &domains)?;
    Ok(Change::Made {
        details: json!({ "domain": domain, "reason": reason }),
        answer: WallDomainDenied {
            domain,
            added: true,
            reason,
        },
        layer,
        kind: &WALL_DOMAIN_DENIED,
    })
}
