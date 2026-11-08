//! The `needs_you.*` operations every Product carries (spec #592):
//! `needs_you.list` reads a page of "Needs you", and `needs_you.dismiss`
//! hides an item whose source allows it, both through the engine's
//! "Needs you" driver. `needs_you.allow_consent_domain` is a person's
//! answer to a `consent_wall` item (ADRs 0048 and 0052): it adds the
//! domain, or `*`, to `consent.domains` in user configuration in one call,
//! an edit of the settings editor (`settings_editor`), every other byte of
//! the file kept, after the resolver confirmed that no
//! Operator policy locks the key, and records the answer in the Audit log
//! with the warning the person was shown; the item then leaves the list at
//! the source's next read. The catalog marks it person only, so the RPC
//! layer refuses it to every surface but the Console's and the CLI's.
//! The answers to a `wall_refetch_stopped` item, which write
//! `wall.refetch_domains` and `wall.denied_domains`, are in [`refetch`].

use std::io;
use std::sync::Arc;

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::needs_you::{
    CONSENT_DOMAIN_ALLOWED, ConsentDomainAllowed, NEEDS_YOU_ALLOW_CONSENT_DOMAIN,
    NEEDS_YOU_DENY_WALL_DOMAIN, NEEDS_YOU_DISMISS, NEEDS_YOU_LIST, NEEDS_YOU_REMOVE_REFETCH_DOMAIN,
    NeedsYouAllowConsentDomainParams, NeedsYouDismissParams, NeedsYouDismissed, NeedsYouList,
    NeedsYouListParams,
};
use goodfetch_protocol::settings::{CONSENT_DOMAINS, SettingsLayer};
use serde_json::{Value, json};

use crate::audit::AuditError;
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::host_name;
use crate::needs_you::NeedsYouError;
use crate::run::settings_error;
use crate::settings::SettingsError;
use crate::settings_editor::{self, Change, SettingsEditor};

mod refetch;

/// The `needs_you.*` handlers.
pub(crate) fn handlers() -> [Handler; 5] {
    [
        Handler::new(&NEEDS_YOU_LIST, list),
        Handler::new(&NEEDS_YOU_DISMISS, dismiss),
        Handler::new(&NEEDS_YOU_ALLOW_CONSENT_DOMAIN, allow_consent_domain),
        Handler::new(
            &NEEDS_YOU_REMOVE_REFETCH_DOMAIN,
            refetch::remove_refetch_domain,
        ),
        Handler::new(&NEEDS_YOU_DENY_WALL_DOMAIN, refetch::deny_wall_domain),
    ]
}

fn failed(error: &NeedsYouError) -> CallError {
    let failure = match error {
        NeedsYouError::Cursor(_) => ProtocolFailure::InvalidParams,
        NeedsYouError::Stopped | NeedsYouError::Index(_) => ProtocolFailure::InternalError,
    };
    CallError::protocol(failure, error.to_string())
}

async fn list(params: NeedsYouListParams, context: CallContext) -> Result<NeedsYouList, CallError> {
    context
        .engine()
        .needs_you()
        .list(params)
        .await
        .map_err(|error| failed(&error))
}

async fn dismiss(
    params: NeedsYouDismissParams,
    context: CallContext,
) -> Result<NeedsYouDismissed, CallError> {
    let dismissed = context
        .engine()
        .needs_you()
        .dismiss(params.item)
        .await
        .map_err(|error| failed(&error))?;
    Ok(NeedsYouDismissed { dismissed })
}

/// `domain` as `consent.domains` lists it: `*`, or the registrable domain
/// of a host name, in lower case and without its trailing dot.
fn consent_domain(domain: &str) -> Result<String, CallError> {
    if domain == "*" {
        return Ok(domain.to_owned());
    }
    let refused = |reason: &str| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`{domain}` {reason}: give a registrable domain, such as `example.com`, or `*`"
            ),
        )
    };
    let host = match url::Host::parse(domain) {
        Ok(url::Host::Domain(host)) => host_name::without_root(&host).to_owned(),
        _ => return Err(refused("names no domain")),
    };
    let registrable = psl::domain_str(&host).unwrap_or(&host);
    if registrable != host {
        return Err(refused(&format!(
            "is no registrable domain, whose is `{registrable}`"
        )));
    }
    Ok(host)
}

/// Why an answer failed: each but `Unrecorded` wrote nothing.
enum Refused {
    /// A setting's refusal, such as an Operator policy's lock.
    Settings(SettingsError),
    /// The file could not be read or written.
    File(io::Error),
    /// The answer does not fit what the file lists, which this says.
    Params(String),
    /// The answer was written, but the Audit log could not record it.
    Unrecorded(AuditError),
}

/// The call error of an answer written that the Audit log could not
/// record.
fn unrecorded(error: &AuditError) -> CallError {
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the answer was written, but the Audit log cannot record it: {error}"),
    )
}

impl From<settings_editor::Refused> for Refused {
    fn from(refused: settings_editor::Refused) -> Self {
        match refused {
            settings_editor::Refused::Settings(error) => Self::Settings(error),
            settings_editor::Refused::File(error) => Self::File(error),
            settings_editor::Refused::Unrecorded(error) => Self::Unrecorded(error),
        }
    }
}

async fn allow_consent_domain(
    params: NeedsYouAllowConsentDomainParams,
    context: CallContext,
) -> Result<ConsentDomainAllowed, CallError> {
    let domain = consent_domain(&params.domain)?;
    let warning = consent_domain_warning(&domain);
    let editor = SettingsEditor::resolving(&context).await;
    let (answered, shown) = (domain.clone(), warning.clone());
    let engine = context.engine();
    let added = match Arc::clone(engine.owner())
        .blocking(move || allow(&editor, &answered, &shown))
        .await
    {
        Ok(added) => added,
        Err(Refused::Settings(error)) => {
            return Err(settings_error(
                &engine.settings().files().named_in(error),
                &context,
            ));
        }
        Err(Refused::Params(message)) => {
            return Err(CallError::protocol(ProtocolFailure::InvalidParams, message));
        }
        Err(Refused::File(error)) => {
            return Err(CallError::protocol(
                ProtocolFailure::InternalError,
                format!("user configuration cannot be written: {error}"),
            ));
        }
        Err(Refused::Unrecorded(error)) => return Err(unrecorded(&error)),
    };
    Ok(ConsentDomainAllowed {
        domain,
        added,
        layer: SettingsLayer::UserConfiguration,
        warning,
    })
}

/// Adds `domain` to user configuration's `consent.domains`, unless it lists
/// it already, once the resolver accepts the file so changed under the
/// Operator policy, and records it in the Audit log with `warning`; answers
/// whether it was added. Blocking.
fn allow(editor: &SettingsEditor, domain: &str, warning: &str) -> Result<bool, Refused> {
    editor.edit(|draft| {
        let layer = SettingsLayer::UserConfiguration;
        draft.machine()?;
        let mut domains: Vec<Value> = draft
            .user_configuration()?
            .get(CONSENT_DOMAINS.name)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if domains
            .iter()
            .filter_map(Value::as_str)
            .any(|listed| host_name::lookup_key(listed) == domain)
        {
            return Ok(Change::Kept(false));
        }
        domains.push(json!(domain));
        draft.set(
            layer,
            "settings",
            CONSENT_DOMAINS.name,
            &Value::Array(domains),
        )?;
        Ok(Change::Made {
            answer: true,
            layer,
            kind: &CONSENT_DOMAIN_ALLOWED,
            details: json!({ "domain": domain, "warning": warning }),
        })
    })
}

/// What a person takes on by adding `domain`, or `*`, to `consent.domains`
/// (ADRs 0048 and 0052), in the words the Console shows beside the answer
/// and the Audit log records with it.
fn consent_domain_warning(domain: &str) -> String {
    if domain == "*" {
        "Accepting consent walls on every domain gives consent in your name on every site that shows one: those sites may load their trackers and set their cookies, and under the GDPR that consent is yours. An Agent also no longer asks you before accepting a consent prompt on any domain. Every later run accepts them until you remove `*` from `consent.domains`.".to_owned()
    } else {
        format!(
            "Accepting the consent walls of {domain} gives consent in your name: the site may load its trackers and set their cookies, and under the GDPR that consent is yours. Every later run accepts them until you remove {domain} from `consent.domains`."
        )
    }
}
