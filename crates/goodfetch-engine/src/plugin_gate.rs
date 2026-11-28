//! A Plugin's `net.fetch` (ADR 0033): the HTTP request the engine sends
//! for a Plugin, each destination checked under the egress class ADR 0033
//! gives it before any connection.
//!
//! A destination is an Operator-configured endpoint only when its origin is
//! that of a URL in the Plugin's own settings and a configuration layer set
//! them: the Operator policy, or user or project configuration, but not a
//! project configuration a Client acting for a model gave (ADR 0092) or one
//! given to a Run opened on an Exposed Daemon, as a `cdp_url` is classed
//! (ADR 0056). Every Plugin is treated as third-party, so a value from a
//! Recipe or a run is checked as a page's until Operator policy can allow
//! it (issue 900), as is every default the Plugin ships. An endpoint is
//! reached as any Operator-configured endpoint is, under
//! `egress.no_private_endpoints`; every other destination is checked as a
//! page's, under the run's domain policy and the resolved-address check,
//! with neither the run's `egress.private_ranges` nor a switched-off
//! `security.resolved_ip_check`, which are its pages' and never reach a
//! Plugin. No destination of either class reaches a link-local or
//! cloud-metadata address.
//!
//! The request carries none of the Session's cookies, goes straight to its
//! destination rather than through the Session's proxy, presents no client
//! certificate and follows no redirect: the Plugin gets the redirect and
//! asks for its location as a new `net.fetch`. A method other than `GET` or
//! `HEAD` toward a destination checked as a page's is an Effect, as is a
//! header such as `X-HTTP-Method-Override` that asks for one, refused until
//! the `effects` permission comes (issue 845).
//!
//! A request may name Secrets by reference, whose values the engine
//! inserts ([`Inserting`]) only toward the domains the Run binds them to;
//! the response then reaches the Plugin with every value the Run resolved
//! masked, in each encoding the engine's masks know.
//!
//! A call back is admitted only inside the call it names, one its instance
//! awaits the answer to, and while fewer than [`CALL_BACKS`] run; the
//! credits and leases that will bound it come with issue 827.

pub mod effects;
mod inserting;
pub mod policy;

use std::collections::BTreeMap;

use bytes::Bytes;
use goodfetch_protocol::egress::EgressRule;
use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use goodfetch_protocol::plugin::approvals::{
    PLUGIN_PERMISSION_DENIED, Permission, PluginPermissionDeniedDetails, fix,
};
use goodfetch_protocol::plugin::net::{
    NetFetchBody, NetFetchHeader, NetFetchParams, NetFetchResult, NetFetchSecret,
    PLUGIN_CALL_BACK_REFUSED, PLUGIN_FETCH_FAILED,
};
use goodfetch_protocol::plugin::settings::PLUGINS;
use goodfetch_protocol::response_limits::{
    LimitSetting, NETWORK_MAX_BODY_SIZE, RESPONSE_LIMIT_EXCEEDED, ResponseLimitExceededDetails,
};
use goodfetch_protocol::settings::SettingsLayer;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{HeaderMap, Method, Request};
use serde_json::Value;
use url::{Origin, Url};

pub use self::effects::{EffectsGate, PluginEffectEntry, PluginEffectJournal};
pub use self::inserting::Inserting;
use crate::call_error::{CallError, named};
use crate::clock::Clock;
use crate::egress::{
    ConfiguredKind, EgressClass, Gate, InvalidEntry, PRIVATE_RANGES, Policy, Refused, RequestKind,
};
use crate::framing::MAX_MESSAGE_BYTES;
use crate::http_client::{EndpointSettings, WholeFailure};
use crate::mask::Scope;
use crate::response_limits::{LimitExceeded, Limits};
use crate::settings::Resolved;
use crate::tasks::BlockingThreads;
use crate::tls::Trust;
use tokio::sync::OnceCell;

/// How many of a Plugin's call backs run at once; one past them is refused.
pub(crate) const CALL_BACKS: usize = 16;

/// The most bytes of a response's body a `net.fetch` answers with: half a
/// message to the Plugin, so that the body in Base64 and its headers fit
/// in one.
const BODY_LIMIT: usize = MAX_MESSAGE_BYTES / 2;

/// Where a Plugin's settings came from, which classes the endpoints they
/// hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SettingsSource {
    /// The layer that set the Plugin's entry of `plugins`, none when
    /// nothing did.
    pub(crate) layer: Option<SettingsLayer>,
    /// Whether a Client acting for a model gave that entry (ADR 0092).
    pub(crate) for_model: bool,
    /// Whether the Run was opened on an Exposed Daemon.
    pub(crate) on_an_exposed_daemon: bool,
}

impl SettingsSource {
    /// Where `settings` took the entry of the Plugin `plugin` from.
    pub(crate) fn of(settings: &Resolved, plugin: &str) -> Self {
        Self {
            layer: settings.entry_layer(&PLUGINS, plugin),
            for_model: settings.entry_for_model(&PLUGINS, plugin),
            on_an_exposed_daemon: settings.on_an_exposed_daemon(),
        }
    }

    /// The egress class of a URL its settings hold: Operator-configured
    /// from a configuration layer, but for a project configuration a model
    /// or an Exposed Daemon's Client gave; checked as a page's otherwise.
    pub(crate) fn class(self) -> EgressClass {
        let Some(layer) = self.layer else {
            return EgressClass::PageReachable;
        };
        if layer == SettingsLayer::ProjectConfiguration
            && (self.for_model || self.on_an_exposed_daemon)
        {
            return EgressClass::PageReachable;
        }
        ConfiguredKind::PluginEndpoint.classed().class(layer)
    }
}

/// What one Plugin's `net.fetch` reaches, and how.
#[derive(Clone, Debug)]
pub struct PluginFetch {
    /// The origins of its Operator-configured endpoints.
    endpoints: Vec<Origin>,
    /// The run's domain policy, as a Plugin's request checked as a page's
    /// meets it.
    page: Policy,
    /// What the machine's settings let an endpoint reach and trust.
    settings: EndpointSettings,
    /// `network.max_body_size`.
    max_body_size: u64,
    /// The authorities `tls.trusted_cas` lists, read on the first
    /// `net.fetch` and kept for the Plugin's others.
    trust: OnceCell<Trust>,
    /// How it inserts the Secrets a request names, none where the Plugin
    /// runs outside a Run, which then refuses them.
    inserting: Option<Inserting>,
    /// Optional Effects / `page_script` / navigation-scope gate for a live
    /// Run's Plugin (ADR 0033, issue 845).
    effects: Option<EffectsGate>,
}

impl PluginFetch {
    /// The `net.fetch` of the Plugin `plugin` under the Run's `settings`,
    /// its endpoints those of its entry of `plugins`, the files of
    /// `tls.trusted_cas` read on `blocking`.
    ///
    /// # Errors
    ///
    /// [`InvalidEntry`] for an entry of the run's domain policy or private
    /// ranges that is none.
    pub fn of(
        settings: &Resolved,
        plugin: &str,
        clock: Clock,
        blocking: BlockingThreads,
    ) -> Result<Self, InvalidEntry> {
        let endpoints = match SettingsSource::of(settings, plugin).class() {
            EgressClass::OperatorConfigured => {
                let mut origins = Vec::new();
                origins_in(&settings.value(&PLUGINS)[plugin]["settings"], &mut origins);
                origins
            }
            EgressClass::PageReachable => Vec::new(),
        };
        Ok(Self {
            endpoints,
            page: Policy::of(settings)?.for_plugin(),
            settings: EndpointSettings::of(settings, clock, blocking),
            max_body_size: Limits::of(settings).max_body_size,
            trust: OnceCell::new(),
            inserting: None,
            effects: None,
        })
    }

    /// This `net.fetch`, inserting the Secrets a request names as
    /// `inserting` says.
    #[must_use]
    pub fn inserting(self, inserting: Inserting) -> Self {
        Self {
            inserting: Some(inserting),
            ..self
        }
    }

    /// Attaches the Run's [`EffectsGate`] so Effects toward the run's sites
    /// are checked against `approved.effects`, audited and journaled (ADR 0033,
    /// issue 845).
    #[must_use]
    pub fn with_effects(self, effects: EffectsGate) -> Self {
        Self {
            effects: Some(effects),
            ..self
        }
    }

    /// The attached [`EffectsGate`], if any.
    #[must_use]
    pub fn effects_gate(&self) -> Option<&EffectsGate> {
        self.effects.as_ref()
    }

    /// The Run's domain policy as this Plugin's page-reachable requests meet it.
    #[must_use]
    pub fn page_policy(&self) -> &Policy {
        &self.page
    }

    /// The egress class of `url`: Operator-configured when its origin is
    /// one of the Plugin's endpoints.
    fn class(&self, url: Option<&Url>) -> EgressClass {
        match url {
            Some(url) if self.endpoints.contains(&url.origin()) => EgressClass::OperatorConfigured,
            _ => EgressClass::PageReachable,
        }
    }

    /// Sends the request `params` asks for, with the Secrets it names
    /// inserted, and answers its response, whole, every value the Run
    /// resolved masked in it.
    ///
    /// # Errors
    ///
    /// `invalid_params` for a method or header that is none;
    /// `plugin_call_back_refused` for `CONNECT`, a header the engine sets
    /// itself, an Effect, or Secrets named outside a Run; a Secret's
    /// failure as [`Inserting::insert`] says; `egress_refused` when the destination's
    /// egress class refuses it; `certificate_invalid` for a certificate
    /// that failed; `plugin_fetch_failed` when no whole response came;
    /// `response_limit_exceeded` for a body past `network.max_body_size`
    /// or [`BODY_LIMIT`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub async fn fetch(&self, mut params: NetFetchParams) -> Result<NetFetchResult, CallError> {
        let method = Method::from_bytes(params.method.as_bytes()).map_err(|_| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("`{}` is no HTTP method", params.method),
            )
        })?;
        if method == Method::CONNECT {
            return Err(refused("`net.fetch` sends no `CONNECT`"));
        }
        let url = Url::parse(&params.url).ok();
        if let Some(url) = &url
            && !matches!(url.scheme(), "http" | "https")
        {
            return Err(Refused {
                url: params.url,
                rule: EgressRule::Scheme,
                address: None,
                setting: None,
            }
            .error());
        }
        let class = self.class(url.as_ref());
        let effect_entry = self
            .authorize_effect(&method, &params, class, url.as_ref())
            .await?;
        let mut headers = HeaderMap::new();
        for header in &params.headers {
            let (name, value) = header_of(header)?;
            headers.append(name, value);
        }
        let mut body = params
            .body
            .take()
            .map(NetFetchBody::into_bytes)
            .unwrap_or_default();
        let sent_to = match self.inserted(&params, url, &mut headers, &mut body).await {
            Ok(sent_to) => sent_to,
            Err(error) => {
                mark_uncertain(effect_entry).await;
                return Err(error);
            }
        };
        let mut request = Request::new(Bytes::from(body));
        *request.method_mut() = method;
        *request.headers_mut() = headers;
        let gate = match class {
            EgressClass::OperatorConfigured => Gate::PluginEndpoint(self.settings.policy()),
            EgressClass::PageReachable => Gate::Session {
                policy: &self.page,
                kind: RequestKind::Fetch,
            },
        };
        let trust = self.trust.get_or_init(|| self.settings.trust()).await;
        let response = match self
            .settings
            .send_whole_through(
                ConfiguredKind::PluginEndpoint,
                gate,
                trust,
                &sent_to,
                request,
                BODY_LIMIT,
            )
            .await
        {
            Ok(response) => {
                if let Some(entry) = effect_entry {
                    entry.done().await?;
                }
                response
            }
            Err(failure) => {
                mark_uncertain(effect_entry).await;
                return Err(self.failed(&params.url, class, failure));
            }
        };
        let masked = |bytes: &[u8]| match &self.inserting {
            Some(inserting) => inserting
                .masks()
                .apply_bytes(bytes, Scope::Kept)
                .into_owned(),
            None => bytes.to_vec(),
        };
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| NetFetchHeader {
                name: name.as_str().to_owned(),
                value: String::from_utf8_lossy(&masked(value.as_bytes())).into_owned(),
            })
            .collect();
        Ok(NetFetchResult {
            status: response.status().as_u16(),
            headers,
            body: NetFetchBody::new(masked(response.body())),
        })
    }

    /// Authorizes a page-reachable `net.fetch` Effect against `self.effects`
    /// (opening a journal entry when approved) or refuses it when no gate is
    /// attached.
    async fn authorize_effect(
        &self,
        method: &Method,
        params: &NetFetchParams,
        class: EgressClass,
        url: Option<&Url>,
    ) -> Result<Option<Box<dyn PluginEffectEntry>>, CallError> {
        if class != EgressClass::PageReachable || !is_effect_request(method, params) {
            return Ok(None);
        }
        if let (Some(gate), Some(target_url)) = (&self.effects, url) {
            gate.authorize_net_fetch(method.as_str(), target_url, true, class, None)
                .await
        } else {
            no_effect(method, params)?;
            Ok(None)
        }
    }

    /// The URL `params` sends to: `url`, as it parsed, with the Secrets it
    /// names inserted in it, `headers` and `body`.
    ///
    /// # Errors
    ///
    /// `plugin_call_back_refused` for Secrets named outside a Run;
    /// `invalid_params` for Secrets named beside a URL that is none; a
    /// Secret's failure as [`Inserting::insert`] says.
    async fn inserted(
        &self,
        params: &NetFetchParams,
        url: Option<Url>,
        headers: &mut HeaderMap,
        body: &mut Vec<u8>,
    ) -> Result<String, CallError> {
        if params.secrets.is_empty() {
            return Ok(params.url.clone());
        }
        let inserting = self
            .inserting
            .as_ref()
            .ok_or_else(|| refused("a Plugin's `net.fetch` names Secrets only inside a Run"))?;
        let mut inserted = url.ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("`{}` is no URL to insert a Secret in", params.url),
            )
        })?;
        inserting
            .insert(&params.secrets, &mut inserted, headers, body)
            .await?;
        Ok(String::from(inserted))
    }

    /// The failure of a `net.fetch` to `url`, of `class`, that got no whole
    /// response: a refusal names `url`, the Plugin's own, never the one a
    /// Secret was inserted in.
    fn failed(&self, url: &str, class: EgressClass, failure: WholeFailure) -> CallError {
        match failure {
            // The run's private ranges, which a refusal checked as a page's
            // names, never reach a Plugin.
            WholeFailure::Refused(refused) => Refused {
                url: url.to_owned(),
                setting: refused.setting.filter(|setting| {
                    !(class == EgressClass::PageReachable && *setting == PRIVATE_RANGES)
                }),
                ..refused
            }
            .error(),
            WholeFailure::CertificateInvalid(invalid) => invalid.error(),
            WholeFailure::ClientCertificate(failure) => failure
                .setting_invalid()
                .unwrap_or_else(|| CallError::new(&PLUGIN_FETCH_FAILED, failure.to_string(), &())),
            WholeFailure::TooLong { .. } => self.too_long(url),
            WholeFailure::Network => CallError::new(
                &PLUGIN_FETCH_FAILED,
                format!("no whole response came from {url}"),
                &(),
            ),
        }
    }

    /// The failure of a response from `url` whose body passed the lower of
    /// `network.max_body_size` and [`BODY_LIMIT`].
    fn too_long(&self, url: &str) -> CallError {
        let limit = u64::try_from(BODY_LIMIT).unwrap_or(u64::MAX);
        if self.max_body_size <= limit {
            return LimitExceeded {
                url: url.to_owned(),
                setting: LimitSetting::MaxBodySize,
                limit: self.max_body_size,
            }
            .error();
        }
        CallError::new(
            &RESPONSE_LIMIT_EXCEEDED,
            format!(
                "the body of {url} is longer than a `net.fetch` answers with ({limit} bytes), below `{}`",
                named::setting(NETWORK_MAX_BODY_SIZE.name)
            ),
            &ResponseLimitExceededDetails {
                url: url.to_owned(),
                setting: LimitSetting::MaxBodySize,
                limit,
            },
        )
    }
}

/// The headers the engine sets itself, for the connection it opens.
const ENGINE_HEADERS: [&str; 8] = [
    "connection",
    "content-length",
    "host",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// The headers some servers read as the request's method, so that a `GET`
/// carrying one can act as a `POST` or a `DELETE`.
const METHOD_OVERRIDES: [&str; 3] = [
    "x-http-method-override",
    "x-http-method",
    "x-method-override",
];

/// Marks an in-flight Plugin Effect uncertain after a send failure.
async fn mark_uncertain(entry: Option<Box<dyn PluginEffectEntry>>) {
    if let Some(entry) = entry {
        entry
            .uncertain(goodfetch_protocol::effect::UncertainReason::FailedAfterSend)
            .await;
    }
}

/// Whether `method` and `params` ask for a write (an Effect when sent toward
/// a page-reachable site): any method other than `GET` or `HEAD`, or a
/// method-override header in the request or its inserted Secrets.
fn is_effect_request(method: &Method, params: &NetFetchParams) -> bool {
    if method != Method::GET && method != Method::HEAD {
        return true;
    }
    let secret_headers = params.secrets.iter().filter_map(|secret| match secret {
        NetFetchSecret::Header { name, .. } => Some(name),
        NetFetchSecret::Query { .. } | NetFetchSecret::Form { .. } => None,
    });
    params
        .headers
        .iter()
        .map(|header| &header.name)
        .chain(secret_headers)
        .any(|header| {
            METHOD_OVERRIDES
                .iter()
                .any(|name| header.eq_ignore_ascii_case(name))
        })
}

/// Refuses the request `params` asks for with `method`, checked as a
/// page's destination, when it is an Effect: a method other than `GET` and
/// `HEAD`, or a header, its own or a Secret's, that asks for another.
///
/// # Errors
///
/// `plugin_call_back_refused` for an Effect.
fn no_effect(method: &Method, params: &NetFetchParams) -> Result<(), CallError> {
    if method != Method::GET && method != Method::HEAD {
        return Err(refused(&format!(
            "a `{method}` to {}, checked as a page's destination, is an Effect, which needs the `effects` permission (issue 845)",
            params.url
        )));
    }
    let secret_headers = params.secrets.iter().filter_map(|secret| match secret {
        NetFetchSecret::Header { name, .. } => Some(name),
        NetFetchSecret::Query { .. } | NetFetchSecret::Form { .. } => None,
    });
    if let Some(header) = params
        .headers
        .iter()
        .map(|header| &header.name)
        .chain(secret_headers)
        .find(|header| {
            METHOD_OVERRIDES
                .iter()
                .any(|name| header.eq_ignore_ascii_case(name))
        })
    {
        return Err(refused(&format!(
            "the header `{header}` to {}, checked as a page's destination, asks for another method, an Effect, which needs the `effects` permission (issue 845)",
            params.url
        )));
    }
    Ok(())
}

/// The name and value of `header`, a request's.
///
/// # Errors
///
/// `invalid_params` for a name or value that is none, and
/// `plugin_call_back_refused` for a header the engine sets itself.
fn header_of(header: &NetFetchHeader) -> Result<(HeaderName, HeaderValue), CallError> {
    let name = header_name(&header.name)?;
    let value =
        HeaderValue::from_str(&header.value).map_err(|_| invalid_header(&header.name, "value"))?;
    Ok((name, value))
}

/// `name`, a request header's that the Plugin may set.
///
/// # Errors
///
/// `invalid_params` for a name that is none, and
/// `plugin_call_back_refused` for a header the engine sets itself.
fn header_name(name: &str) -> Result<HeaderName, CallError> {
    let header =
        HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid_header(name, "name"))?;
    if ENGINE_HEADERS.contains(&header.as_str()) || header.as_str().starts_with("proxy-") {
        return Err(refused(&format!(
            "the engine sets the header `{header}` itself"
        )));
    }
    Ok(header)
}

/// `invalid_params` for the header `name`, whose `what` is none.
fn invalid_header(name: &str, what: &str) -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        format!("the header `{name}` has no valid {what}"),
    )
}

/// `plugin_call_back_refused`, with `message`.
fn refused(message: &str) -> CallError {
    CallError::new(&PLUGIN_CALL_BACK_REFUSED, message, &())
}

/// `plugin_permission_denied` for the Plugin `name`'s `permission`.
pub(crate) fn permission_denied(name: &str, permission: Permission) -> CallError {
    let fix = fix(name, &format!("approved.{}", permission.field()));
    let message = format!(
        "the Operator policy has not approved the Plugin `{name}`'s permission `{}`",
        permission.field()
    );
    let details = PluginPermissionDeniedDetails {
        plugin: name.to_owned(),
        permission,
        fix,
    };
    CallError::new(&PLUGIN_PERMISSION_DENIED, message, &details)
}

/// Admits or refuses a Plugin's call back naming `call_id`, while its
/// instance awaits the answers to the calls `awaited` and runs `running`
/// call backs.
///
/// # Errors
///
/// `plugin_call_back_refused` when `call_id` names no call `awaited` holds,
/// or when [`CALL_BACKS`] already run.
pub(crate) fn admit_call_back<V>(
    awaited: &BTreeMap<RequestId, V>,
    running: usize,
    call_id: &RequestId,
) -> Result<(), CallError> {
    if !awaited.contains_key(call_id) {
        return Err(refused(&format!(
            "the Plugin called back naming {call_id:?}, which is no call it serves"
        )));
    }
    if running >= CALL_BACKS {
        return Err(refused(&format!(
            "the Plugin has {CALL_BACKS} call backs running, the most at once"
        )));
    }
    Ok(())
}

/// Collects into `origins` the origin of each `http` or `https` URL that
/// `value` holds, at any depth.
fn origins_in(value: &Value, origins: &mut Vec<Origin>) {
    match value {
        Value::String(text) => {
            if let Ok(url) = Url::parse(text)
                && matches!(url.scheme(), "http" | "https")
                && !origins.contains(&url.origin())
            {
                origins.push(url.origin());
            }
        }
        Value::Array(values) => {
            for value in values {
                origins_in(value, origins);
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                origins_in(value, origins);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests;
