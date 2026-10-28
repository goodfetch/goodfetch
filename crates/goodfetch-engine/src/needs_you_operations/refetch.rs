//! A person's answers to a `wall_refetch_stopped` item (ADR 0052):
//! `needs_you.remove_refetch_domain` removes the domain from the machine's
//! layer of `wall.refetch_domains` that lists it, user configuration or
//! else the Operator policy, or replaces `*` there with the list the
//! person gives when only `*` covers it; `needs_you.deny_wall_domain` adds
//! the domain to the Operator policy's `wall.denied_domains` with its
//! reason. Each writes one file in one call, every other byte kept, once
//! the resolver accepts the file so changed, refuses a managed Operator
//! policy with `setting_locked`, and records the answer in the Audit log.
//! A domain only project configuration lists is changed in its file: the
//! engine never writes a project's file. On a Daemon with accounts both
//! need the operator role, and denial step-up (the accounts and step-up
//! tickets); until then the caller is the operator.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::{Format, OPERATOR_POLICY, USER_CONFIGURATION};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::needs_you::{
    NeedsYouDenyWallDomainParams, NeedsYouRemoveRefetchDomainParams, REFETCH_DOMAIN_REMOVED,
    RefetchDomainRemoved, WALL_DOMAIN_DENIED, WallDomainDenied,
};
use goodfetch_protocol::settings::{
    SettingLimit, SettingsLayer, WALL_DENIED_DOMAINS, WALL_REFETCH_DOMAINS,
};
use serde_json::{Value, json};

use super::{Refused, consent_domain, replace};
use crate::audit::{AuditLog, Origin};
use crate::composition::{CallContext, CallError};
use crate::needs_you::refetch::{coverage, machine_listing, normalized};
use crate::run::settings_error;
use crate::settings::{
    HostDefaults, Layers, MachineLayers, SettingsError, operator_policy, resolve, set,
    user_configuration,
};

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

/// What every answer's blocking write takes: the machine's files and
/// layers as read, and how to record the answer.
struct Machine {
    user_configuration: PathBuf,
    operator_policy: PathBuf,
    layers: MachineLayers,
    host: HostDefaults,
    product: Product,
    version: ProductVersion,
    audit: AuditLog,
    surface: goodfetch_protocol::connection::Surface,
    /// The person answering, as the Audit log names them.
    person: String,
    now: jiff::Timestamp,
}

impl Machine {
    async fn of(context: &CallContext) -> Result<Self, CallError> {
        let runs = context.runs();
        let settings = runs.settings().clone();
        let layers = settings
            .machine()
            .await
            .map_err(|error| settings_error(&error, context.identity()))?;
        Ok(Self {
            user_configuration: settings.files().user_configuration().to_owned(),
            operator_policy: settings.files().operator_policy().to_owned(),
            layers,
            host: settings.host_defaults().await,
            product: settings.product(),
            version: context.identity().product_version.clone(),
            audit: runs.audit().clone(),
            surface: context.surface(),
            person: context.person(),
            now: runs.limits().now(),
        })
    }

    /// Sets `setting` to `domains` in `table` of `layer`'s file, once the
    /// resolver accepts the file so changed beside the other machine
    /// layer. Blocking.
    fn write(
        &self,
        layer: SettingsLayer,
        table: &str,
        setting: &Setting,
        domains: &[String],
    ) -> Result<(), Refused> {
        let (path, format): (&PathBuf, &'static Format) = match layer {
            SettingsLayer::OperatorPolicy => (&self.operator_policy, &OPERATOR_POLICY),
            _ => (&self.user_configuration, &USER_CONFIGURATION),
        };
        #[expect(
            clippy::disallowed_methods,
            reason = "the answer's file work runs under the engine's blocking policy"
        )]
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                format!("version = \"{}\"\n", format.version)
            }
            Err(error) => return Err(Refused::File(error)),
        };
        let value = Value::Array(domains.iter().map(|domain| json!(domain)).collect());
        let rewritten = set(
            &text,
            table,
            setting.name,
            &value,
            format,
            &self.version,
            setting.arrived_in,
        )
        .map_err(Refused::Settings)?;
        let mut layers = Layers {
            host: self.host.clone(),
            operator_policy: self.layers.operator_policy.clone(),
            user_configuration: self.layers.user_configuration.clone(),
            ..Layers::default()
        };
        if layer == SettingsLayer::OperatorPolicy {
            if self.layers.operator_policy.managed.is_some() {
                return Err(Refused::Settings(SettingsError::Locked {
                    key: setting.name.to_owned(),
                    layer,
                    limit: SettingLimit {
                        key: setting.name.to_owned(),
                        value: self
                            .layers
                            .operator_policy
                            .lock
                            .get(setting.name)
                            .or_else(|| self.layers.operator_policy.settings.get(setting.name))
                            .cloned()
                            .unwrap_or_else(|| json!([])),
                        layer,
                    },
                    fix: "the Operator policy is managed: change it where it is managed".to_owned(),
                }));
            }
            layers.operator_policy = operator_policy(&rewritten, self.product, &self.version)
                .map_err(Refused::Settings)?;
        } else {
            layers.user_configuration =
                user_configuration(layer, &rewritten, self.product, &self.version)
                    .map_err(Refused::Settings)?;
        }
        resolve(&layers, self.product, self.product).map_err(Refused::Settings)?;
        replace(path, &rewritten).map_err(Refused::File)
    }

    /// Records the answer `kind` with `details`, from `layer`.
    fn record(
        &self,
        kind: &goodfetch_protocol::catalog::SharedTypeValue,
        layer: SettingsLayer,
        details: Value,
    ) {
        let origin = Origin {
            actor: Actor::Person {
                name: self.person.clone(),
            },
            surface: Some(self.surface),
            layer: Some(layer),
            run_id: None,
        };
        if let Err(error) = self.audit.record(&origin, kind, details, self.now) {
            tracing::warn!(%error, kind = kind.name, "the Audit log cannot record a person's answer");
        }
    }
}

/// The call error of a write that refused.
fn refused(refused: Refused, context: &CallContext) -> CallError {
    match refused {
        Refused::Settings(error) => settings_error(
            &context.runs().settings().files().named_in(error),
            context.identity(),
        ),
        Refused::File(error) => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("a settings file cannot be written: {error}"),
        ),
        Refused::Params(message) => CallError::protocol(ProtocolFailure::InvalidParams, message),
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
    let machine = Machine::of(&context).await?;
    let owner = Arc::clone(&context.runs().limits().owner);
    let answered = domain.clone();
    owner
        .blocking(move || remove(&machine, &answered, instead))
        .await
        .map_err(|error| refused(error, &context))
}

/// Removes `domain` from the machine's layer of `wall.refetch_domains`,
/// or replaces `*` there with `instead`. Blocking.
fn remove(
    machine: &Machine,
    domain: &str,
    instead: Option<Vec<String>>,
) -> Result<RefetchDomainRemoved, Refused> {
    let unchanged = || RefetchDomainRemoved {
        domain: domain.to_owned(),
        removed: false,
        layer: None,
        domains: Vec::new(),
    };
    let Some(listing) = machine_listing(&machine.layers) else {
        return Ok(unchanged());
    };
    let domains: Vec<String> = match coverage(&listing.domains, domain) {
        None => return Ok(unchanged()),
        Some(false) => listing
            .domains
            .iter()
            .filter(|listed| normalized(listed) != domain)
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
    machine.write(
        listing.layer,
        listing.table,
        &WALL_REFETCH_DOMAINS,
        &domains,
    )?;
    machine.record(
        &REFETCH_DOMAIN_REMOVED,
        listing.layer,
        json!({ "domain": domain, "domains": domains }),
    );
    Ok(RefetchDomainRemoved {
        domain: domain.to_owned(),
        removed: true,
        layer: Some(listing.layer),
        domains,
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
    let machine = Machine::of(&context).await?;
    let owner = Arc::clone(&context.runs().limits().owner);
    owner
        .blocking(move || deny(&machine, domain, reason))
        .await
        .map_err(|error| refused(error, &context))
}

/// Adds `domain` to the Operator policy's `wall.denied_domains`, in its
/// lock when that sets the key, with `reason`. Blocking.
fn deny(machine: &Machine, domain: String, reason: String) -> Result<WallDomainDenied, Refused> {
    let policy = &machine.layers.operator_policy;
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
    if domains.iter().any(|listed| normalized(listed) == domain) {
        return Ok(WallDomainDenied {
            domain,
            added: false,
            reason,
        });
    }
    domains.push(domain.clone());
    let layer = SettingsLayer::OperatorPolicy;
    machine.write(layer, table, &WALL_DENIED_DOMAINS, &domains)?;
    machine.record(
        &WALL_DOMAIN_DENIED,
        layer,
        json!({ "domain": domain, "reason": reason }),
    );
    Ok(WallDomainDenied {
        domain,
        added: true,
        reason,
    })
}
