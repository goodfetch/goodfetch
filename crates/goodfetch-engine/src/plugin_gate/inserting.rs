//! The Secrets a Plugin's `net.fetch` names by reference (ADR 0033): the
//! engine inserts each value on the way out, in a header or a query
//! parameter, with an optional template such as `Bearer {}`, or in the
//! form field the operator set for the reference's binding, and only
//! toward a domain the Run binds that reference to. A reference the
//! Plugin's manifest does not declare, or the Operator policy has not
//! approved for it, is refused, and one the Run binds to no placeholder is
//! sent nowhere. Every check passes before any value is resolved, so a
//! refused request resolves, and audits, none.

use std::collections::BTreeSet;

use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::approvals::{Approvals, Permission};
use goodfetch_protocol::plugin::net::NetFetchSecret;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::placeholders::{PlaceholderSource, SECRETS_PLACEHOLDERS};
use goodfetch_protocol::secrets::{SECRET_UNBOUND, SecretUnboundDetails};
use goodfetch_protocol::settings::SettingsLayer;
use hyper::HeaderMap;
use hyper::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;
use url::form_urlencoded;

use super::{header_name, permission_denied, refused};
use crate::call_error::CallError;
use crate::mask::Masks;
use crate::secrets::{self, Purpose, RunSecrets, SecretError, secret_error};
use crate::settings::Resolved;

/// The media type of a body a Secret goes in a form field of.
const FORM: &str = "application/x-www-form-urlencoded";

/// How one Plugin's `net.fetch` inserts the Secrets it names: the
/// references its manifest declares, those the Operator policy approved,
/// and the Run's bindings, whose values it resolves through the Run's
/// Secrets.
#[derive(Clone)]
pub struct Inserting {
    /// The Plugin's name.
    plugin: String,
    /// The references its manifest's `permissions.secrets` declares.
    declared: BTreeSet<String>,
    /// What the Operator policy approved for it.
    approved: Approvals,
    /// The Run's bindings to a Secret reference's value, a TOTP seed's
    /// left out, whose values are only typed.
    bindings: Vec<Binding>,
    /// The layer that set `secrets.placeholders`.
    layer: Option<SettingsLayer>,
    /// The Run the values are used in.
    run_id: RunId,
    /// The Run's Secrets, which resolve, audit and mask each value.
    secrets: RunSecrets,
}

/// One placeholder bound to a Secret reference's value.
#[derive(Clone, Debug)]
struct Binding {
    placeholder: String,
    reference: SecretReference,
    domains: Vec<String>,
    form_field: Option<String>,
}

/// One Secret of a request, checked and ready to resolve.
struct Planned<'a> {
    binding: &'a Binding,
    target: Target<'a>,
}

/// Where a planned Secret's value goes.
enum Target<'a> {
    Header {
        name: HeaderName,
        template: Option<&'a str>,
    },
    Query {
        name: &'a str,
        template: Option<&'a str>,
    },
    Form {
        field: &'a str,
    },
}

impl Inserting {
    /// How the Plugin `plugin`, whose manifest is `manifest` and whose
    /// approvals are `approved`, inserts Secrets in a Run of `settings`
    /// whose Secrets are `secrets`.
    pub fn new(
        plugin: &str,
        (manifest, approved): (&PluginManifest, &Approvals),
        settings: &Resolved,
        secrets: RunSecrets,
        run_id: RunId,
    ) -> Self {
        let bindings = settings
            .value(&SECRETS_PLACEHOLDERS)
            .as_object()
            .into_iter()
            .flat_map(|bound| bound.keys())
            .filter_map(|placeholder| {
                let bound = secrets::binding(settings, placeholder).ok()?;
                match bound.source {
                    PlaceholderSource::Reference {
                        reference,
                        totp: false,
                        form_field,
                    } => Some(Binding {
                        placeholder: placeholder.clone(),
                        reference,
                        domains: bound.domains,
                        form_field,
                    }),
                    PlaceholderSource::Reference { totp: true, .. }
                    | PlaceholderSource::CodeSource(_) => None,
                }
            })
            .collect();
        Self {
            plugin: plugin.to_owned(),
            declared: manifest.permissions.secrets.iter().cloned().collect(),
            approved: approved.clone(),
            bindings,
            layer: settings
                .effective(&SECRETS_PLACEHOLDERS)
                .map(|effective| effective.layer),
            run_id,
            secrets,
        }
    }

    /// The engine's masks, which every value the Run resolves joins.
    pub(super) fn masks(&self) -> &Masks {
        self.secrets.mask().masks()
    }

    /// Inserts the value of each of `named` into the request to `url`
    /// with `headers` and `body`, once each is checked.
    ///
    /// # Errors
    ///
    /// `invalid_params` for a URL with no host, a template without one
    /// `{}`, a header, query parameter or form field the request already
    /// sets, or a form field in a body that is no form;
    /// `plugin_call_back_refused` for a reference the manifest does not
    /// declare or one whose binding has no form field;
    /// `plugin_permission_denied` for one the Operator policy has not
    /// approved; `secret_unbound` for one no placeholder binds;
    /// `secret_domain_not_allowed` toward a host outside its bindings'
    /// domains; and the resolution's own failure.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the resolution under way; the request
    /// is not sent.
    pub(super) async fn insert(
        &self,
        named: &[NetFetchSecret],
        url: &mut Url,
        headers: &mut HeaderMap,
        body: &mut Vec<u8>,
    ) -> Result<(), CallError> {
        let host = url
            .host_str()
            .ok_or_else(|| invalid("a Secret goes only to a URL with a host".to_owned()))?
            .to_owned();
        let planned = self.planned(named, &host, url, headers, body)?;
        for Planned { binding, target } in planned {
            let value = self
                .secrets
                .resolve(
                    &binding.reference,
                    &Purpose {
                        placeholder: &binding.placeholder,
                        domain: Some(&host),
                        domains: &binding.domains,
                        layer: self.layer,
                        run_id: Some(&self.run_id),
                    },
                )
                .await
                .map_err(|error| secret_error(&error))?;
            match target {
                Target::Header { name, template } => {
                    let filled = filled(template, &value);
                    let mut value =
                        HeaderValue::from_str(filled.expose_secret()).map_err(|_| {
                            invalid(format!(
                                "the value of the Secret `{}` cannot go in the header `{name}`",
                                binding.reference
                            ))
                        })?;
                    value.set_sensitive(true);
                    headers.append(name, value);
                }
                Target::Query { name, template } => {
                    url.query_pairs_mut()
                        .append_pair(name, filled(template, &value).expose_secret());
                }
                Target::Form { field } => {
                    if !body.is_empty() {
                        body.push(b'&');
                    }
                    let pair = SecretString::from(
                        form_urlencoded::Serializer::new(String::new())
                            .append_pair(field, value.expose_secret())
                            .finish(),
                    );
                    body.extend_from_slice(pair.expose_secret().as_bytes());
                    if !headers.contains_key(CONTENT_TYPE) {
                        headers.insert(CONTENT_TYPE, HeaderValue::from_static(FORM));
                    }
                }
            }
        }
        Ok(())
    }

    /// Each of `named`, checked toward `host` against the request to `url`
    /// with `headers` and `body`, before any value is resolved.
    fn planned<'a>(
        &'a self,
        named: &'a [NetFetchSecret],
        host: &str,
        url: &Url,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<Planned<'a>>, CallError> {
        let mut headers_set: BTreeSet<String> = headers
            .keys()
            .map(|name| name.as_str().to_owned())
            .collect();
        let mut queried: BTreeSet<String> = url
            .query_pairs()
            .map(|(name, _)| name.into_owned())
            .collect();
        let mut fields: Option<BTreeSet<String>> = None;
        named
            .iter()
            .map(|secret| {
                let binding = self.binding(secret.reference(), host)?;
                let target = match secret {
                    NetFetchSecret::Header { name, template, .. } => {
                        let name = header_name(name)?;
                        if !headers_set.insert(name.as_str().to_owned()) {
                            return Err(invalid(format!(
                                "the header `{name}` is set twice, by a Secret and by the request or another Secret"
                            )));
                        }
                        Target::Header {
                            name,
                            template: checked_template(template.as_deref(), &binding.reference)?,
                        }
                    }
                    NetFetchSecret::Query { name, template, .. } => {
                        if !queried.insert(name.clone()) {
                            return Err(invalid(format!(
                                "the query parameter `{name}` is set twice, by a Secret and by the URL or another Secret"
                            )));
                        }
                        Target::Query {
                            name,
                            template: checked_template(template.as_deref(), &binding.reference)?,
                        }
                    }
                    NetFetchSecret::Form { .. } => {
                        let field = binding.form_field.as_deref().ok_or_else(|| {
                            refused(&format!(
                                "the Secret `{}` goes in a form field only where the operator sets `secrets.placeholders.{}.form_field`",
                                binding.reference, binding.placeholder
                            ))
                        })?;
                        let fields = match &mut fields {
                            Some(fields) => fields,
                            None => fields.insert(form_fields(headers, body)?),
                        };
                        if !fields.insert(field.to_owned()) {
                            return Err(invalid(format!(
                                "the form field `{field}` is set twice, by a Secret and by the body or another Secret"
                            )));
                        }
                        Target::Form { field }
                    }
                };
                Ok(Planned { binding, target })
            })
            .collect()
    }

    /// The binding the value of `reference` goes to `host` under: the
    /// first of the Run's bindings of it whose domains allow `host`.
    ///
    /// # Errors
    ///
    /// As [`Self::insert`] says of a reference.
    fn binding(&self, reference: &str, host: &str) -> Result<&Binding, CallError> {
        if !self.declared.contains(reference) {
            return Err(refused(&format!(
                "the Plugin `{}`'s manifest declares no Secret `{reference}` in `permissions.secrets`",
                self.plugin
            )));
        }
        let permission = Permission::Secret {
            reference: reference.to_owned(),
        };
        if !self.approved.covers(&permission) {
            return Err(permission_denied(&self.plugin, permission));
        }
        let mut bound = self
            .bindings
            .iter()
            .filter(|binding| binding.reference.as_str() == reference)
            .peekable();
        let Some(first) = bound.peek().copied() else {
            return Err(CallError::new(
                &SECRET_UNBOUND,
                format!(
                    "no Secret placeholder binds the reference `{reference}`, so its value goes nowhere; bind it in `secrets.placeholders` with the domains it may go to"
                ),
                &SecretUnboundDetails {
                    placeholder: reference.to_owned(),
                    bound: self
                        .bindings
                        .iter()
                        .map(|binding| binding.placeholder.clone())
                        .collect(),
                },
            ));
        };
        bound
            .find(|binding| secrets::allowed(host, &binding.domains))
            .ok_or_else(|| {
                secret_error(&SecretError::DomainNotAllowed {
                    placeholder: first.placeholder.clone(),
                    reference: first.reference.clone(),
                    domain: host.to_owned(),
                    domains: first.domains.clone(),
                })
            })
    }
}

impl std::fmt::Debug for Inserting {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Inserting")
            .field("plugin", &self.plugin)
            .field("declared", &self.declared)
            .field("bindings", &self.bindings)
            .finish_non_exhaustive()
    }
}

/// `template`, when it holds `{}` once, where the value of `reference`
/// goes.
///
/// # Errors
///
/// `invalid_params` for a template without one `{}`.
fn checked_template<'a>(
    template: Option<&'a str>,
    reference: &SecretReference,
) -> Result<Option<&'a str>, CallError> {
    match template {
        Some(text) if text.matches("{}").count() != 1 => Err(invalid(format!(
            "the template of the Secret `{reference}` holds `{{}}`, where its value goes, once"
        ))),
        _ => Ok(template),
    }
}

/// `value` in `template` at its one `{}`, or alone.
fn filled(template: Option<&str>, value: &SecretString) -> SecretString {
    match template {
        Some(template) => SecretString::from(template.replacen("{}", value.expose_secret(), 1)),
        None => value.clone(),
    }
}

/// The fields of `body`, a form, as `headers` say it is or it is empty.
///
/// # Errors
///
/// `invalid_params` for a body of another media type.
fn form_fields(headers: &HeaderMap, body: &[u8]) -> Result<BTreeSet<String>, CallError> {
    let essence = headers.get(CONTENT_TYPE).map(|value| {
        value
            .to_str()
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    });
    match essence.as_deref() {
        Some(FORM) => {}
        None if body.is_empty() => {}
        _ => {
            return Err(invalid(format!(
                "a Secret goes in a form field only of a body of `{FORM}`"
            )));
        }
    }
    Ok(form_urlencoded::parse(body)
        .map(|(name, _)| name.into_owned())
        .collect())
}

/// `invalid_params`, saying `message`.
fn invalid(message: String) -> CallError {
    CallError::protocol(ProtocolFailure::InvalidParams, message)
}
