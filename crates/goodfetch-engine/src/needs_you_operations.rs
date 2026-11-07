//! The `needs_you.*` operations every Product carries (spec #592):
//! `needs_you.list` reads a page of "Needs you", and `needs_you.dismiss`
//! hides an item whose source allows it, both through the engine's
//! "Needs you" driver. `needs_you.allow_consent_domain` is a person's
//! answer to a `consent_wall` item (ADRs 0048 and 0052): it adds the
//! domain, or `*`, to `consent.domains` in user configuration in one call,
//! every other byte of the file kept, after the resolver confirmed that no
//! Operator policy locks the key, and records the answer in the Audit log
//! with the warning the person was shown; the item then leaves the list at
//! the source's next read. The catalog marks it person only, so the RPC
//! layer refuses it to every surface but the Console's and the CLI's.
//! The answers to a `wall_refetch_stopped` item, which write
//! `wall.refetch_domains` and `wall.denied_domains`, are in [`refetch`].

use std::io;
use std::sync::Arc;

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::USER_CONFIGURATION;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::needs_you::{
    CONSENT_DOMAIN_ALLOWED, ConsentDomainAllowed, NEEDS_YOU_ALLOW_CONSENT_DOMAIN,
    NEEDS_YOU_DENY_WALL_DOMAIN, NEEDS_YOU_DISMISS, NEEDS_YOU_LIST, NEEDS_YOU_REMOVE_REFETCH_DOMAIN,
    NeedsYouAllowConsentDomainParams, NeedsYouDismissParams, NeedsYouDismissed, NeedsYouList,
    NeedsYouListParams, consent_domain_warning,
};
use goodfetch_protocol::settings::{CONSENT_DOMAINS, SettingsLayer};
use serde_json::{Value, json};

use crate::audit::{AuditError, AuditLog, Origin};
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::durable;
use crate::needs_you::NeedsYouError;
use crate::run::settings_error;
use crate::settings::{
    HostDefaults, Layers, MachineLayers, SettingsError, resolve, set, user_configuration,
};

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
        Ok(url::Host::Domain(host)) => host.trim_end_matches('.').to_owned(),
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

/// What one answer changed.
enum Written {
    /// User configuration listed the domain already.
    Listed,
    /// The domain was added.
    Added,
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

/// One answer's inputs, as its blocking write takes them.
struct Answer {
    domain: String,
    warning: String,
    path: std::path::PathBuf,
    machine: MachineLayers,
    host: HostDefaults,
    product: Product,
    version: ProductVersion,
    audit: AuditLog,
    origin: Origin,
    now: jiff::Timestamp,
}

async fn allow_consent_domain(
    params: NeedsYouAllowConsentDomainParams,
    context: CallContext,
) -> Result<ConsentDomainAllowed, CallError> {
    let domain = consent_domain(&params.domain)?;
    let engine = context.engine();
    let settings = engine.settings().clone();
    let machine = settings
        .machine()
        .await
        .map_err(|error| settings_error(&error, &context))?;
    let warning = consent_domain_warning(&domain);
    let answer = Answer {
        domain: domain.clone(),
        warning: warning.clone(),
        path: settings.files().user_configuration().to_owned(),
        machine,
        host: settings.host_defaults().await,
        product: settings.product(),
        version: context.identity().product_version.clone(),
        audit: engine.audit().clone(),
        origin: Origin {
            actor: Actor::Person {
                name: context.person(),
            },
            surface: Some(context.surface()),
            layer: Some(SettingsLayer::UserConfiguration),
            run_id: None,
        },
        now: engine.now(),
    };
    let added = match Arc::clone(engine.owner())
        .blocking(move || write(answer))
        .await
    {
        Ok(Written::Added) => true,
        Ok(Written::Listed) => false,
        Err(Refused::Settings(error)) => {
            return Err(settings_error(&settings.files().named_in(error), &context));
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

/// Adds `answer`'s domain to user configuration's `consent.domains`, unless
/// it lists it already, once the resolver accepts the file so changed
/// under the Operator policy, and records it in the Audit log. Blocking.
fn write(answer: Answer) -> Result<Written, Refused> {
    let Answer {
        domain,
        warning,
        path,
        machine,
        host,
        product,
        version,
        audit,
        origin,
        now,
    } = answer;
    #[expect(
        clippy::disallowed_methods,
        reason = "the answer's file work runs under the engine's blocking policy"
    )]
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            format!("version = \"{}\"\n", USER_CONFIGURATION.version)
        }
        Err(error) => return Err(Refused::File(error)),
    };
    let layer = SettingsLayer::UserConfiguration;
    let current = user_configuration(layer, &text, product, &version).map_err(Refused::Settings)?;
    let mut domains: Vec<Value> = current
        .get(CONSENT_DOMAINS.name)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if domains
        .iter()
        .any(|listed| listed.as_str() == Some(&domain))
    {
        return Ok(Written::Listed);
    }
    domains.push(json!(domain));
    let rewritten = set(
        &text,
        "settings",
        CONSENT_DOMAINS.name,
        &Value::Array(domains),
        &USER_CONFIGURATION,
        &version,
        CONSENT_DOMAINS.arrived_in,
    )
    .map_err(Refused::Settings)?;
    let layers = Layers {
        host,
        operator_policy: machine.operator_policy,
        user_configuration: user_configuration(layer, &rewritten, product, &version)
            .map_err(Refused::Settings)?,
        ..Layers::default()
    };
    resolve(&layers, product, product).map_err(Refused::Settings)?;
    replace(&path, &rewritten).map_err(Refused::File)?;
    let details = json!({ "domain": domain, "warning": warning });
    audit
        .record(&origin, &CONSENT_DOMAIN_ALLOWED, details, now)
        .map_err(Refused::Unrecorded)?;
    Ok(Written::Added)
}

/// Replaces the file at `path` with `text`, through a file renamed over
/// it, so no reader sees half of one, and an answer acknowledged survives a
/// power cut (ADR 0002). Blocking.
fn replace(path: &std::path::Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        durable::create_folder(folder)?;
    }
    durable::replace(path, text.as_bytes())
}
