//! A Session's proxy as a fetch tier's own HTTP transport takes it
//! (#1640): opened as a browser Session's is, from the same settings and
//! the same Proxy-provider driver, held for the Session's life, and reached
//! as an HTTP proxy, through the loopback relay a SOCKS5 proxy takes since
//! the transport speaks no SOCKS5. Each hop through it holds a Permit of
//! the proxy's limits, given back as the proxy's refusal when the proxy
//! refused the hop, which lowers its learned ceiling (ADR 0053). A hop the
//! proxy answered or failed itself raises the proxy's error, as a browser
//! Session's navigation does, never the site's answer (the proxy-or-site
//! rule).

use goodfetch_engine::CallError;
use goodfetch_engine::net_error::NetError;
use goodfetch_engine::run::state_unavailable;
use goodfetch_engine::runs::{self, ReportWarnings};
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::page::proxy::PROXY_PROVIDER;
use goodfetch_protocol::usage::PROXY_BYTES;
use secrecy::SecretString;

use super::limits::ProxyPermit;
use super::party::{Answer, Party, party, proxy_error, relays};
use super::relay::{Mode, Relay};
use super::{Opening, RunPowers, SessionProxy, lookup, proxy_addresses, relay_unavailable};
use crate::browser_run::BrowserRun;

/// The proxy one Session's HTTP transport sends through, as it opened.
pub struct TransportProxy {
    proxy: SessionProxy,
    /// The loopback relay in front of a SOCKS5 proxy the Session has no
    /// credentials for, which the transport reaches as an HTTP proxy.
    socks: Option<Relay>,
    /// The Session it opened for, which a provider's sticky username names.
    session: SessionId,
}

/// Where a transport sends its hops through a proxy: the proxy's URL, an
/// HTTP or HTTPS proxy's, and the credentials it answers with.
#[derive(Clone, Debug)]
pub struct ProxyRoute {
    /// The proxy's URL, the loopback relay's when the Session has one.
    pub url: String,
    /// The username and password the proxy asks for, if any.
    pub credentials: Option<(String, SecretString)>,
}

/// A Permit of the proxy's limits one hop through it holds: dropped, it
/// goes back as a request that ended.
#[derive(Debug)]
pub struct TransportPermit(ProxyPermit);

impl TransportPermit {
    /// Gives the Permit back now for a hop the proxy itself refused,
    /// lowering the proxy's ceiling.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the Permit to the Run's end, which
    /// frees it.
    pub async fn refused(self) {
        self.0.refused().await;
    }
}

impl TransportProxy {
    /// The proxy `settings`, a Run's, name for its Session `session`, the
    /// `ordinal`-th of the Run's to open one, from 1, if they name one.
    ///
    /// # Errors
    ///
    /// The errors of a browser Session's proxy as it opens
    /// (`setting_invalid`, the Secret provider's codes, `proxy_unavailable`
    /// and `proxy_relay_unavailable`), and `run_state_unavailable` when the
    /// Run's Limits on `proxy_bytes` cannot be read for a Proxy provider.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes any relay it opened.
    pub async fn open(
        run: &BrowserRun,
        settings: &Resolved,
        session: SessionId,
        ordinal: u64,
    ) -> Result<Option<Self>, CallError> {
        // A Proxy provider's price must be in the currency of each currency
        // Limit on `proxy_bytes` that holds the Run (ADR 0101), as a
        // browser Session's.
        let currencies = if settings.get::<Option<&str>>(&PROXY_PROVIDER).is_none() {
            Vec::new()
        } else {
            runs::usage(&run.charging, &run.state)
                .await
                .map_err(|error| {
                    state_unavailable(
                        run.identity.id().clone(),
                        run.identity.folder(),
                        &error.to_string(),
                    )
                })?
                .limits
                .into_iter()
                .filter(|limit| limit.usage_kind == PROXY_BYTES)
                .filter_map(|limit| limit.currency)
                .collect()
        };
        let opening = Opening {
            session: &session,
            ordinal,
            currencies: &currencies,
        };
        // A fetch tier's Session has no owner of its own: its proxy's tasks
        // start on the Run's, which its end stops.
        let powers = RunPowers {
            identity: run.identity.clone(),
            secrets: run.keys.secrets().clone(),
            politeness: run.politeness.clone(),
            charging: run.charging.clone(),
            refusals: run.engine.refusals(),
        };
        let Some(proxy) = SessionProxy::open(settings, &powers, &opening).await? else {
            return Ok(None);
        };
        let socks = if proxy.needs_transport_relay() {
            Some(proxy.open_transport_relay(&powers, settings).await?)
        } else {
            None
        };
        Ok(Some(Self {
            proxy,
            socks,
            session,
        }))
    }

    /// Where the transport sends its hops: its own loopback relay, with
    /// that relay's credentials, when it has one; where the Session's proxy
    /// says otherwise.
    #[must_use]
    pub fn route(&self) -> ProxyRoute {
        self.socks
            .as_ref()
            .map_or_else(|| self.proxy.transport_route(), relay_route)
    }

    /// The exit country the Proxy provider declared for this proxy, if any
    /// (ADR 0046).
    #[must_use]
    pub fn country(&self) -> Option<&str> {
        self.proxy.country()
    }

    /// Whether a hop to `url` goes around the proxy, as `proxy.bypass`
    /// says.
    #[must_use]
    pub fn bypasses(&self, url: &str) -> bool {
        self.proxy.bypasses(url)
    }

    /// A Permit for one hop through the proxy, once its limits admit it
    /// (ADRs 0053 and 0088); none when the proxy has no limits.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the limits as they were.
    pub async fn permit(&self) -> Option<TransportPermit> {
        self.proxy.permit().await.map(TransportPermit)
    }

    /// Whether the response to `url` with `status` and the `Proxy-Status`
    /// field `proxy_status` is the proxy refusing a request its plan does
    /// not admit: its own 429 or 502 (the proxy-or-site rule), never the
    /// site's nor the relay's own.
    #[must_use]
    pub fn refused_response(&self, url: &str, status: u16, proxy_status: Option<&str>) -> bool {
        let answer = self.response(url, proxy_status);
        self.proxy.refusal(url, answer, Some(status))
    }

    /// The error a response to `url` with `status` and the `Proxy-Status`
    /// field `proxy_status` raises when the proxy gave it, never the site
    /// (the proxy-or-site rule): `proxy_auth_failed` for its `407` to a
    /// request it forwarded, which a browser takes as the proxy's too;
    /// `proxy_unavailable` naming the error its field names otherwise.
    #[must_use]
    pub fn answered(
        &self,
        url: &str,
        status: u16,
        proxy_status: Option<&str>,
    ) -> Option<CallError> {
        let answer = self.response(url, proxy_status);
        if status == 407
            && matches!(
                answer,
                Answer::Response {
                    forwarded: true,
                    ..
                }
            )
        {
            return Some(self.proxy.auth_failed(url));
        }
        if party(answer) != Party::Proxy {
            return None;
        }
        let error = proxy_status.and_then(proxy_error).unwrap_or_default();
        Some(self.proxy.answered(url, &error))
    }

    /// The error a hop to `url` that failed with the network error
    /// `network_error`, as Chrome names it, raises when the proxy failed it
    /// itself: `proxy_auth_failed` or `proxy_unavailable`, as a browser
    /// Session's.
    #[must_use]
    pub fn failure(&self, url: &str, network_error: &str) -> Option<CallError> {
        self.proxy.failure(url, &NetError::of(network_error))
    }

    /// A response to `url` through the proxy, as a browser Session reads
    /// one; the `502` of the transport's own SOCKS5 relay for a proxy it
    /// could not use is the proxy's, as the Session's relay's is.
    fn response<'a>(&self, url: &str, proxy_status: Option<&'a str>) -> Answer<'a> {
        let relayed = proxy_status.is_some_and(relays)
            && self.socks.as_ref().is_some_and(|relay| relay.answered(url));
        if relayed {
            return Answer::Response {
                proxy_status,
                forwarded: true,
            };
        }
        self.proxy.response(url, proxy_status)
    }

    /// Whether the hop to `url` that failed with the network error
    /// `network_error`, as Chrome names it, is the proxy refusing its
    /// tunnel, never a proxy that could not be reached.
    #[must_use]
    pub fn refused_failure(&self, url: &str, network_error: &str) -> bool {
        let error = NetError::of(network_error);
        let down = self
            .socks
            .as_ref()
            .is_some_and(|relay| relay.unreachable(url));
        !down
            && self
                .proxy
                .refusal(url, Answer::Failure { error: &error }, None)
    }

    /// Warns the Run through `warnings`, once, when the Session outlived its
    /// provider's sticky lifetime, so its exit may have changed
    /// (`proxy_exit_may_change`).
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the warning unwritten.
    pub async fn warn_if_outlived(&self, warnings: &ReportWarnings) {
        self.proxy.warn_if_outlived(warnings, &self.session).await;
    }
}

impl SessionProxy {
    /// Whether a fetch tier's HTTP transport, which speaks no SOCKS5, needs
    /// a loopback relay of its own in front of the proxy: a SOCKS5 proxy
    /// the Session has no relay for.
    pub(crate) fn needs_transport_relay(&self) -> bool {
        self.relay.is_none() && self.server.scheme.is_socks()
    }

    /// Opens the transport's loopback relay to the proxy, which dials the
    /// addresses its host's lookup gave, judged as the Session's are.
    ///
    /// # Errors
    ///
    /// Those of [`super::proxy_addresses`] for a relayed proxy, and
    /// `proxy_relay_unavailable` when the relay cannot open.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes any relay it opened.
    pub(crate) async fn open_transport_relay(
        &self,
        run: &RunPowers,
        settings: &Resolved,
    ) -> Result<Relay, CallError> {
        let addresses = proxy_addresses(&self.server, self.judged, true, || {
            lookup(&self.server, settings)
        })
        .await?;
        let mode = Mode::Socks5 { credentials: None };
        Relay::open(run, addresses, mode, None, false)
            .await
            .map_err(|error| relay_unavailable(&self.server, &error))
    }

    /// Where a fetch tier's HTTP transport with no relay of its own sends
    /// its hops: the Session's relay, with that relay's credentials, when
    /// it has one; the proxy itself, with the Session's, otherwise.
    pub(crate) fn transport_route(&self) -> ProxyRoute {
        if let Some(relay) = &self.relay {
            return relay_route(relay);
        }
        ProxyRoute {
            url: self.server.to_string(),
            credentials: self.username.clone().map(|username| {
                let password = self
                    .password
                    .clone()
                    .unwrap_or_else(|| SecretString::from(""));
                (username, password)
            }),
        }
    }
}

/// The route through the loopback relay `relay`, an HTTP proxy, with its
/// own credentials.
pub(super) fn relay_route(relay: &Relay) -> ProxyRoute {
    let (username, password) = relay.credentials();
    ProxyRoute {
        url: format!("http://{}", relay.address()),
        credentials: Some((username.to_owned(), password.clone())),
    }
}

impl std::fmt::Debug for TransportProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransportProxy")
            .field("proxy", &self.proxy)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}
