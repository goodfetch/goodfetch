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
//! `HEAD` toward a destination checked as a page's is an Effect, refused
//! until the `effects` permission comes (issue 845).

use bytes::Bytes;
use goodfetch_protocol::egress::EgressRule;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::plugin::net::{
    NetFetchBody, NetFetchHeader, NetFetchParams, NetFetchResult, PLUGIN_CALL_BACK_REFUSED,
    PLUGIN_FETCH_FAILED,
};
use goodfetch_protocol::plugin::settings::PLUGINS;
use goodfetch_protocol::response_limits::{
    LimitSetting, NETWORK_MAX_BODY_SIZE, RESPONSE_LIMIT_EXCEEDED, ResponseLimitExceededDetails,
};
use goodfetch_protocol::settings::SettingsLayer;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Method, Request};
use serde_json::Value;
use url::{Origin, Url};

use crate::call_error::{CallError, named};
use crate::clock::Clock;
use crate::egress::{
    ConfiguredKind, EgressClass, Gate, InvalidEntry, PRIVATE_RANGES, Policy, Refused, RequestKind,
};
use crate::framing::MAX_MESSAGE_BYTES;
use crate::http_client::{EndpointSettings, WholeFailure};
use crate::response_limits::{LimitExceeded, Limits};
use crate::settings::Resolved;
use crate::tasks::BlockingThreads;

/// The most bytes of a response's body a `net.fetch` answers with: half a
/// message to the Plugin, so that the body in Base64 and its headers fit
/// in one.
const BODY_LIMIT: usize = MAX_MESSAGE_BYTES / 2;

/// Where a Plugin's settings came from, which classes the endpoints they
/// hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsSource {
    /// The layer that set the Plugin's entry of `plugins`, none when
    /// nothing did.
    pub layer: Option<SettingsLayer>,
    /// Whether a Client acting for a model gave that entry (ADR 0092).
    pub for_model: bool,
    /// Whether the Run was opened on an Exposed Daemon.
    pub on_an_exposed_daemon: bool,
}

impl SettingsSource {
    /// Where `settings` took the entry of the Plugin `plugin` from.
    pub fn of(settings: &Resolved, plugin: &str) -> Self {
        Self {
            layer: settings.entry_layer(&PLUGINS, plugin),
            for_model: settings.entry_for_model(&PLUGINS, plugin),
            on_an_exposed_daemon: settings.on_an_exposed_daemon(),
        }
    }

    /// The egress class of a URL its settings hold: Operator-configured
    /// from a configuration layer, but for a project configuration a model
    /// or an Exposed Daemon's Client gave; checked as a page's otherwise.
    pub fn class(self) -> EgressClass {
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
        })
    }

    /// The egress class of `url`: Operator-configured when its origin is
    /// one of the Plugin's endpoints.
    fn class(&self, url: Option<&Url>) -> EgressClass {
        match url {
            Some(url) if self.endpoints.contains(&url.origin()) => EgressClass::OperatorConfigured,
            _ => EgressClass::PageReachable,
        }
    }

    /// Sends the request `params` asks for and answers its response,
    /// whole.
    ///
    /// # Errors
    ///
    /// `invalid_params` for a method or header that is none;
    /// `plugin_call_back_refused` for `CONNECT`, a header the engine sets
    /// itself, or an Effect; `egress_refused` when the destination's
    /// egress class refuses it; `certificate_invalid` for a certificate
    /// that failed; `plugin_fetch_failed` when no whole response came;
    /// `response_limit_exceeded` for a body past `network.max_body_size`
    /// or [`BODY_LIMIT`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the connection it opened.
    pub async fn fetch(&self, params: NetFetchParams) -> Result<NetFetchResult, CallError> {
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
        if class == EgressClass::PageReachable && method != Method::GET && method != Method::HEAD {
            return Err(refused(&format!(
                "a `{method}` to {}, checked as a page's destination, is an Effect, which needs the `effects` permission (issue 845)",
                params.url
            )));
        }
        let mut request = Request::new(
            params
                .body
                .map_or_else(Bytes::new, |body| Bytes::from(body.into_bytes())),
        );
        *request.method_mut() = method;
        for header in params.headers {
            let (name, value) = header_of(&header)?;
            request.headers_mut().append(name, value);
        }
        let gate = match class {
            EgressClass::OperatorConfigured => Gate::PluginEndpoint(self.settings.policy()),
            EgressClass::PageReachable => Gate::Session {
                policy: &self.page,
                kind: RequestKind::Fetch,
            },
        };
        let response = self
            .settings
            .send_whole_through(
                ConfiguredKind::PluginEndpoint,
                gate,
                &params.url,
                request,
                BODY_LIMIT,
            )
            .await
            .map_err(|failure| self.failed(&params.url, class, failure))?;
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| NetFetchHeader {
                name: name.as_str().to_owned(),
                value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
            })
            .collect();
        Ok(NetFetchResult {
            status: response.status().as_u16(),
            headers,
            body: NetFetchBody::new(response.into_body().to_vec()),
        })
    }

    /// The failure of a `net.fetch` to `url`, of `class`, that got no whole
    /// response.
    fn failed(&self, url: &str, class: EgressClass, failure: WholeFailure) -> CallError {
        match failure {
            // The run's private ranges, which a refusal checked as a page's
            // names, never reach a Plugin.
            WholeFailure::Refused(refused) => Refused {
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

/// The name and value of `header`, a request's.
///
/// # Errors
///
/// `invalid_params` for a name or value that is none, and
/// `plugin_call_back_refused` for a header the engine sets itself.
fn header_of(header: &NetFetchHeader) -> Result<(HeaderName, HeaderValue), CallError> {
    let invalid = |what: &str| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("the header `{}` has no valid {what}", header.name),
        )
    };
    let name = HeaderName::from_bytes(header.name.as_bytes()).map_err(|_| invalid("name"))?;
    if ENGINE_HEADERS.contains(&name.as_str()) || name.as_str().starts_with("proxy-") {
        return Err(refused(&format!(
            "the engine sets the header `{name}` itself"
        )));
    }
    let value = HeaderValue::from_str(&header.value).map_err(|_| invalid("value"))?;
    Ok((name, value))
}

/// `plugin_call_back_refused`, with `message`.
fn refused(message: &str) -> CallError {
    CallError::new(&PLUGIN_CALL_BACK_REFUSED, message, &())
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
